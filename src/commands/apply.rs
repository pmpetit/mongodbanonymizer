//! `manon apply` – read documents from a source MongoDB collection, apply the
//! masking rules from an inferred YAML schema, and write the anonymised
//! documents into a target MongoDB collection.
//!
//! # Example
//! ```text
//! manon apply --source-uri mongodb://localhost:27017 \
//!             --namespace mydb.listings             \
//!             --target-uri mongodb://remote:27017   \
//!             schema/listings/listings.yaml
//! ```

use anyhow::{Context, Result, anyhow};
use futures::TryStreamExt;
use indexmap::IndexMap;
use mongodb::{
    Client,
    bson::{Bson, Document, doc},
    options::ClientOptions,
};
use serde_yaml;

use crate::args::ApplyArgs;
use crate::commands::init::read_conf;
use crate::helpers::parse_namespace;
use crate::masking;
use crate::models::{CollectionSchema, FieldSchema};

/// Batch size for `insert_many` calls to the target collection.
const INSERT_BATCH: usize = 500;

// ─────────────────────────────────────────────────────────────────────────────
// Public entry point
// ─────────────────────────────────────────────────────────────────────────────

pub async fn run_apply(args: ApplyArgs) -> Result<()> {
    // ── 0. Resolve source_uri and namespace (CLI takes priority over conf) ────
    let (source_uri, source_ns) = if let Some(ref conf) = args.config {
        let c = read_conf(conf)?;
        let uri = args.mongo.source_uri.clone().or(c.uri).ok_or_else(|| {
            anyhow!("No source URI provided: pass --source-uri or add URI to the config file")
        })?;
        let ns = args.namespace.clone().or(c.namespace).ok_or_else(|| {
            anyhow!("No namespace provided: pass --namespace or add NAMESPACE to the config file")
        })?;
        (uri, ns)
    } else {
        let uri =
            args.mongo.source_uri.clone().ok_or_else(|| {
                anyhow!("No source URI provided: pass --source-uri or -c <config>")
            })?;
        let ns = args
            .namespace
            .clone()
            .ok_or_else(|| anyhow!("No namespace provided: pass --namespace or -c <config>"))?;
        (uri, ns)
    };

    // ── 1. Load and parse the YAML schema ────────────────────────────────────
    let yaml_str = std::fs::read_to_string(&args.masking_rules).with_context(|| {
        format!(
            "Failed to read schema file {}",
            args.masking_rules.display()
        )
    })?;
    let schema: CollectionSchema = serde_yaml::from_str(&yaml_str)
        .with_context(|| format!("Failed to parse YAML from {}", args.masking_rules.display()))?;

    // ── 2. Resolve namespaces ─────────────────────────────────────────────────
    let (source_db, source_coll) = parse_namespace(&source_ns)
        .with_context(|| format!("Invalid source namespace '{source_ns}'"))?;

    let target_ns_str = args
        .target_namespace
        .as_deref()
        .unwrap_or(source_ns.as_str())
        .to_owned();
    let (target_db, target_coll) = parse_namespace(&target_ns_str)
        .with_context(|| format!("Invalid target namespace '{target_ns_str}'"))?;

    // ── 3. Connect to source ──────────────────────────────────────────────────
    let source_opts = ClientOptions::parse(&source_uri)
        .await
        .context("Failed to parse source URI")?;
    let source_client =
        Client::with_options(source_opts).context("Failed to create source client")?;

    // ── 4. Connect to target ──────────────────────────────────────────────────
    let target_opts = ClientOptions::parse(&args.target_uri)
        .await
        .context("Failed to parse target URI")?;
    let target_client =
        Client::with_options(target_opts).context("Failed to create target client")?;

    let target_collection = target_client
        .database(target_db)
        .collection::<Document>(target_coll);

    // ── 5. Stream source → mask → insert target ───────────────────────────────
    let src_collection = source_client
        .database(source_db)
        .collection::<Document>(source_coll);

    let total = src_collection
        .estimated_document_count()
        .await
        .context("Failed to count source documents")?;

    println!(
        "Applying masking rules: {source_ns} → {target_ns_str}  ({total} documents estimated)"
    );

    let mut cursor = src_collection
        .find(doc! {})
        .await
        .context("Failed to open source cursor")?;

    let mut batch: Vec<Document> = Vec::with_capacity(INSERT_BATCH);
    let mut written: u64 = 0;

    while let Some(mut document) = cursor.try_next().await.context("Cursor error")? {
        apply_masking_to_doc(&mut document, &schema.object);
        batch.push(document);

        if batch.len() >= INSERT_BATCH {
            target_collection
                .insert_many(batch.drain(..).collect::<Vec<_>>())
                .await
                .context("Failed to insert batch")?;
            written += INSERT_BATCH as u64;
            println!("  written {written} …");
        }
    }

    if !batch.is_empty() {
        let n = batch.len() as u64;
        target_collection
            .insert_many(batch)
            .await
            .context("Failed to insert final batch")?;
        written += n;
    }

    println!("Done. {written} document(s) written to {target_ns_str}.");
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Masking logic applied to live BSON documents
// ─────────────────────────────────────────────────────────────────────────────

/// Walk a BSON [`Document`] and replace field values according to the masking
/// rules recorded in `schema` (the `object` map from a [`CollectionSchema`]).
pub fn apply_masking_to_doc(doc: &mut Document, schema: &IndexMap<String, FieldSchema>) {
    for (key, field_schema) in schema {
        if let Some(value) = doc.get_mut(key) {
            mask_bson_field(value, field_schema);
        }
    }
}

/// Apply the masking rule(s) from `field_schema` to a single BSON value.
///
/// The function dispatches on the concrete BSON variant so it handles:
/// * `String`   → masked with the String TypeSchema method
/// * `Double / Int32 / Int64` → masked with the Number TypeSchema method
/// * `Array`    → each element is recursed with the array-items FieldSchema
/// * `Document` → recursed with the Object TypeSchema's sub-schema
fn mask_bson_field(value: &mut Bson, field_schema: &FieldSchema) {
    match value {
        // ── String ─────────────────────────────────────────────────────────
        Bson::String(s) => {
            if let Some(ts) = field_schema.types.get("String") {
                if let Some(mc) = &ts.masking {
                    if mc.enabled {
                        *s = masking::mask_value(&mc.method, s);
                    }
                }
            }
        }

        // ── Number ─────────────────────────────────────────────────────────
        Bson::Double(v) => {
            if let Some(ts) = field_schema.types.get("Number") {
                if let Some(mc) = &ts.masking {
                    if mc.enabled {
                        let masked = masking::mask_value(&mc.method, &v.to_string());
                        if let Ok(n) = masked.parse::<f64>() {
                            *v = n;
                        }
                    }
                }
            }
        }
        Bson::Int32(v) => {
            if let Some(ts) = field_schema.types.get("Number") {
                if let Some(mc) = &ts.masking {
                    if mc.enabled {
                        let masked = masking::mask_value(&mc.method, &v.to_string());
                        if let Ok(n) = masked.parse::<f64>() {
                            *v = n as i32;
                        }
                    }
                }
            }
        }
        Bson::Int64(v) => {
            if let Some(ts) = field_schema.types.get("Number") {
                if let Some(mc) = &ts.masking {
                    if mc.enabled {
                        let masked = masking::mask_value(&mc.method, &v.to_string());
                        if let Ok(n) = masked.parse::<f64>() {
                            *v = n as i64;
                        }
                    }
                }
            }
        }

        // ── Array ───────────────────────────────────────────────────────────
        // The masking rule lives on the item types, not the Array TypeSchema.
        Bson::Array(arr) => {
            if let Some(ts) = field_schema.types.get("Array") {
                if let Some(items_schema) = &ts.array {
                    for elem in arr.iter_mut() {
                        mask_bson_field(elem, items_schema);
                    }
                }
            }
        }

        // ── Embedded document ──────────────────────────────────────────────
        Bson::Document(sub_doc) => {
            if let Some(ts) = field_schema.types.get("Object") {
                if let Some(obj_schema) = &ts.object {
                    apply_masking_to_doc(sub_doc, obj_schema);
                }
            }
        }

        // All other BSON types (Date, ObjectId, Boolean, …) are left unchanged.
        _ => {}
    }
}
