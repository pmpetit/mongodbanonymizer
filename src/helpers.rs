use anyhow::{Context, Result, anyhow};
use futures::TryStreamExt;
use mongodb::{Client, bson::doc};
use std::time::Duration;

pub fn parse_namespace(ns: &str) -> Result<(&str, &str)> {
    let dot = ns
        .find('.')
        .ok_or_else(|| anyhow!("Namespace must be in the form <db>.<collection>, got: {ns}"))?;
    Ok((&ns[..dot], &ns[dot + 1..]))
}

pub async fn existing_db(client: &Client, dbname: &str) -> Result<bool> {
    let existing_dbs = client
        .list_database_names()
        .await
        .context("Failed to list databases")?;
    if !existing_dbs.iter().any(|d| d == dbname) {
        eprintln!(
            "Warning: database '{dbname}' does not exist on the server. Available databases: {}",
            existing_dbs
                .iter()
                .filter(|d| !SYSTEM_DATABASES.contains(&d.as_str()))
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        );
        return Ok(false);
    }
    Ok(true)
}

pub async fn existing_collection(client: &Client, dbname: &str, coll_name: &str) -> Result<bool> {
    let existing_coll = client
        .database(dbname)
        .list_collection_names()
        .await
        .context("Failed to list collections")?;
    if !existing_coll.iter().any(|c| c == coll_name) {
        eprintln!(
            "Warning: collection '{coll_name}' does not exist in database '{dbname}'. Available collections: {}",
            existing_coll
                .iter()
                .filter(|c| !c.starts_with("system."))
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        );
        return Ok(false);
    }
    Ok(true)
}

pub async fn get_locale(client: &Client, dbname: &str, coll_name: &str) -> Result<String> {
    let mut cursor = client
        .database(dbname)
        .list_collections()
        .filter(doc! { "name": coll_name })
        .await
        .context("Failed to list collections for locale inference")?;

    if let Some(spec) = cursor
        .try_next()
        .await
        .context("Failed to read collection info")?
    {
        if let Some(collation) = spec.options.collation {
            return Ok(collation.locale);
        }
    }

    // No collation defined on the collection – MongoDB default is "simple" (binary comparison).
    Ok("simple".to_string())
}

pub async fn get_metadata(client: &Client, dbname: &str, coll_name: &str) -> Result<()> {
    let db = client.database(dbname);
    let mut cursor = db
        .list_collections()
        .filter(doc! { "name": coll_name })
        .await?;
    while let Some(spec) = cursor.try_next().await? {
        println!("{:#?}", spec);
        println!("Collection name: {}", spec.name);
        println!("Collection type: {:#?}", spec.collection_type);
        let options = spec.options;
        println!("Options: {:#?}", options);
        if let Some(collation) = options.collation {
            println!("Collation: {:#?}", collation);
        } else {
            println!("No explicit collection collation set.");
        }
    }
    Ok(())
}

/// MongoDB system databases that are skipped when enumerating user databases.
const SYSTEM_DATABASES: &[&str] = &["admin", "local", "config"];

pub const SAMPLE_MAX_TIME: Duration = Duration::from_secs(120);
pub const DEFAULT_SAMPLE_SIZE: u64 = 1000;
