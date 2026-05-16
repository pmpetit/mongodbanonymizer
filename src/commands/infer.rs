use crate::analyzer::{Analyzer, annotate_masking, mask_sampled_values};
use crate::args::{InferArgs, UriArg};
use crate::commands::init::read_conf;
use crate::helpers::{DEFAULT_SAMPLE_SIZE, existing_collection, existing_db, parse_namespace};
use crate::models::CollectionSchema;
use anyhow::{Context, Result, anyhow};
use futures::TryStreamExt;
use mongodb::{Client, bson::doc, options::ClientOptions};
use serde_yaml;
use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;

pub async fn run_infer(args: InferArgs) -> Result<()> {
    // Resolve URI, namespace, number, and percent – reading conf file if -c was given
    let (resolved_uri, effective_output_dir, conf_namespace, conf_number, conf_percent) =
        if let Some(ref conf) = args.config {
            let c = read_conf(conf)?;
            let uri = args.mongo.source_uri.clone().or(c.uri).ok_or_else(|| {
                anyhow!("No URI provided: pass it as an argument or add URI to the config file")
            })?;
            let out_dir = args.output_dir.clone().unwrap_or_else(|| {
                c.base_dir
                    .join(&c.project_dir)
                    .join("source")
                    .join("collections")
            });
            (uri, Some(out_dir), c.namespace, c.number, c.percent)
        } else {
            let uri = args
                .mongo
                .source_uri
                .clone()
                .ok_or_else(|| anyhow!("No URI provided: pass --uri or -c <config>"))?;
            (uri, args.output_dir.clone(), None, None, None)
        };

    let namespace = args.namespace.clone().or(conf_namespace);

    let client_options = ClientOptions::parse(&resolved_uri)
        .await
        .context("Failed to parse client options")?;
    let client = Client::with_options(client_options).context("Failed to create client")?;

    // // CLI takes priority over conf for number/percent; then fall back to defaults
    let resolved_number = args.number.or(conf_number);
    let resolved_percent = args.percent.or(conf_percent);
    let _args = InferArgs {
        mongo: UriArg {
            source_uri: Some(resolved_uri),
        },
        namespace: namespace.clone(),
        output_dir: effective_output_dir,
        number: resolved_number,
        percent: resolved_percent,
        config: None,
        ..args
    };

    if let Some(ns) = namespace {
        if ns.contains('.') {
            let (db_name, coll_name) = parse_namespace(&ns)?;
            if existing_db(&client, db_name).await? {
                if existing_collection(&client, db_name, coll_name).await? {
                    println!("Inferring schema for collection: {db_name}.{coll_name}");
                    let _schema = infer_collection(&client, db_name, coll_name, &_args).await?;
                }
            }
        } else {
            // DB-only namespace: infer every collection in the database
            let db_name = ns.as_str();
            if !existing_db(&client, db_name).await? {
                return Err(anyhow!("Database '{db_name}' does not exist"));
            }
            let collection_names = client
                .database(db_name)
                .list_collection_names()
                .await
                .with_context(|| format!("Failed to list collections in '{db_name}'"))?;
            if collection_names.is_empty() {
                println!("No collections found in database '{db_name}'");
            } else {
                println!(
                    "Inferring schema for all {} collection(s) in database: {db_name}",
                    collection_names.len()
                );
                for coll_name in &collection_names {
                    println!("  → {db_name}.{coll_name}");
                    infer_collection(&client, db_name, coll_name, &_args).await?;
                }
            }
        }
    }

    Ok(())
}

async fn infer_collection(
    client: &Client,
    db_name: &str,
    coll_name: &str,
    args: &InferArgs,
) -> Result<CollectionSchema> {
    let output_dir = args.output_dir.as_deref();
    let db = client.database(db_name);
    let collection = db.collection::<mongodb::bson::Document>(coll_name);

    let (sample_size, known_total) = if let Some(pct) = args.percent {
        if pct <= 0.0 || pct > 100.0 {
            return Err(anyhow!(
                "--percent must be between 0 (exclusive) and 100 (inclusive), got {pct}"
            ));
        }
        let total = collection.count_documents(doc! {}).await?;
        let n = ((total as f64 * pct / 100.0).ceil() as u64).max(1);
        (n, Some(total))
    } else {
        (args.number.unwrap_or(DEFAULT_SAMPLE_SIZE), None)
    };

    let mut analyzer = Analyzer::new(true);

    let mut cursor = collection
        .find(doc! {})
        .limit(sample_size as i64)
        .await
        .with_context(|| format!("Failed to query {db_name}.{coll_name}"))?;
    while let Some(doc) = cursor.try_next().await.context("Cursor error")? {
        analyzer.process_document(&doc);
    }

    let mut schema = analyzer.finish();
    let total_docs = if let Some(t) = known_total {
        t
    } else {
        collection
            .estimated_document_count()
            .await
            .context("Failed to get document count")?
    };
    schema.count = total_docs;

    let field_method_map = field_method_map();
    annotate_masking(&mut schema, field_method_map);
    mask_sampled_values(&mut schema);

    let output_dir = output_dir; // rebind to keep borrow checker happy

    if let Some(out_dir) = output_dir {
        write_collection_yaml_files(out_dir, coll_name, &schema)
            .with_context(|| format!("Failed to write YAML output files for {coll_name}"))?;
    }

    Ok(schema)
}

/// Build a map from identifier key → masking method by combining the two data CSVs.
/// identifier.csv:          `locale \t field_name \t category`
/// identifier_category.csv: `category METHOD` (space-separated)
/// Returns a reference to the process-wide field→method map, built once from
/// the embedded CSV files.
pub fn field_method_map() -> &'static HashMap<String, String> {
    static MAP: OnceLock<HashMap<String, String>> = OnceLock::new();
    MAP.get_or_init(build_field_method_map)
}

fn build_field_method_map() -> HashMap<String, String> {
    const IDENTIFIER_CSV: &str = include_str!("../../data/identifier.csv");
    const IDENTIFIER_CATEGORY_CSV: &str = include_str!("../../data/identifier_category.csv");

    // Step 1: category → method
    let mut category_map: HashMap<String, String> = HashMap::new();
    for line in IDENTIFIER_CATEGORY_CSV.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.splitn(2, ' ');
        if let (Some(cat), Some(method)) = (parts.next(), parts.next()) {
            category_map.insert(cat.to_string(), method.trim().to_string());
        }
    }

    // Step 2: field_name → method  (via category)
    let mut field_map: HashMap<String, String> = HashMap::new();
    for line in IDENTIFIER_CSV.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() >= 3 {
            let field_name = cols[1];
            let category = cols[2];
            if let Some(method) = category_map.get(category) {
                field_map.insert(field_name.to_string(), method.clone());
            }
        }
    }

    field_map
}

/// Write `<dir>/<name>/<name>.yaml`.
fn write_collection_yaml_files(
    base: &Path,
    coll_name: &str,
    schema: &CollectionSchema,
) -> Result<()> {
    let safe_name = coll_name.replace('/', "_");
    let dir = base.join(&safe_name);
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("Failed to create directory {}", dir.display()))?;

    let yaml_path = dir.join(format!("{safe_name}.yaml"));
    std::fs::write(&yaml_path, serde_yaml::to_string(schema)?)
        .with_context(|| format!("Failed to write {}", yaml_path.display()))?;

    Ok(())
}
