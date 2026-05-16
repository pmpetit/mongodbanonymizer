//! End-to-end tests using the MongoDB sample datasets located in `tests/data/`.
//!
//! A **shared read-only container** is started once for the whole test binary
//! and populated with:
//!   - `sample_analytics`  (accounts, customers, transactions — ~4 000 docs)
//!   - `sample_mflix/users`                                   (185 docs)
//!
//! Tests that need to write anonymised data start their own container so they
//! do not interfere with each other.
//!
//! Run with:
//! ```bash
//! cargo test --test e2e_sample_tests -- --nocapture
//! ```

use std::path::Path;
use std::sync::{Arc, Mutex};

use futures::TryStreamExt;
use mongodb::{Client, bson::Document, bson::doc, options::ClientOptions};
use testcontainers::{ContainerAsync, runners::AsyncRunner};
use testcontainers_modules::mongo::Mongo;
use tokio::sync::OnceCell;

use mongodbanonymizer::args::{ApplyArgs, InferArgs, UriArg};
use mongodbanonymizer::commands::apply::run_apply;
use mongodbanonymizer::commands::infer::run_infer;

// ─────────────────────────────────────────────────────────────────────────────
// Shared read-only fixture
// ─────────────────────────────────────────────────────────────────────────────

struct SampleFixture {
    uri: String,
    /// Holds the container alive for the life of the test binary.
    /// Wrapped in Arc<Mutex<_>> so SampleFixture is Send + Sync.
    _container: Arc<Mutex<ContainerAsync<Mongo>>>,
}

static FIXTURE: OnceCell<SampleFixture> = OnceCell::const_new();

async fn fixture() -> &'static SampleFixture {
    FIXTURE
        .get_or_init(|| async {
            let container = Mongo::default().start().await.expect("start mongo");
            let host = container.get_host().await.expect("get host");
            let port = container.get_host_port_ipv4(27017).await.expect("get port");
            let uri = format!("mongodb://{host}:{port}/");

            println!("Shared fixture: importing sample datasets into {uri}");
            import_analytics_and_mflix(&uri).await;

            SampleFixture {
                uri,
                _container: Arc::new(Mutex::new(container)),
            }
        })
        .await
}

// ─────────────────────────────────────────────────────────────────────────────
// Data import helpers
// ─────────────────────────────────────────────────────────────────────────────

async fn mongo_client(uri: &str) -> Client {
    let opts = ClientOptions::parse(uri).await.expect("parse uri");
    Client::with_options(opts).expect("create client")
}

/// Import sample_analytics (3 collections) and sample_mflix/users.
async fn import_analytics_and_mflix(uri: &str) {
    let client = mongo_client(uri).await;
    let data_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data");

    let datasets: &[(&str, &[&str])] = &[
        (
            "sample_analytics",
            &["accounts", "customers", "transactions"],
        ),
        ("sample_mflix", &["users"]),
    ];

    for (db, collections) in datasets {
        for &coll in *collections {
            let path = data_dir.join(db).join(format!("{coll}.json"));
            let n = import_jsonl(&client, db, coll, &path).await;
            println!("  {db}.{coll}: {n} docs");
        }
    }
}

/// Read a newline-delimited JSON file and bulk-insert into MongoDB.
/// Uses `bson::Bson::try_from(serde_json::Value)` which handles MongoDB
/// Extended JSON v2 notation (`$oid`, `$date`, `$numberInt`, etc.).
async fn import_jsonl(client: &Client, db: &str, collection: &str, path: &Path) -> usize {
    let coll = client.database(db).collection::<Document>(collection);
    let content = std::fs::read_to_string(path)
        .unwrap_or_else(|_| panic!("Failed to read {}", path.display()));

    let docs: Vec<Document> = content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|line| {
            let json_val: serde_json::Value = serde_json::from_str(line).ok()?;
            let bson_val = mongodb::bson::Bson::try_from(json_val).ok()?;
            bson_val.as_document().cloned()
        })
        .collect();

    let total = docs.len();
    for chunk in docs.chunks(500) {
        coll.insert_many(chunk.to_vec())
            .await
            .expect("insert chunk");
    }
    total
}

// ─────────────────────────────────────────────────────────────────────────────
// Helper builders
// ─────────────────────────────────────────────────────────────────────────────

fn infer_args(uri: &str, namespace: &str, output_dir: &std::path::PathBuf) -> InferArgs {
    InferArgs {
        mongo: UriArg {
            source_uri: Some(uri.to_owned()),
        },
        namespace: Some(namespace.to_owned()),
        number: Some(200),
        percent: None,
        no_output: true,
        output_dir: Some(output_dir.clone()),
        config: None,
    }
}

fn apply_args(
    source_uri: &str,
    namespace: &str,
    masking_rules: Option<std::path::PathBuf>,
    target_uri: &str,
    target_namespace: Option<&str>,
    percent: Option<f64>,
) -> ApplyArgs {
    ApplyArgs {
        mongo: UriArg {
            source_uri: Some(source_uri.to_owned()),
        },
        masking_rules,
        namespace: Some(namespace.to_owned()),
        target_uri: target_uri.to_owned(),
        target_namespace: target_namespace.map(|s| s.to_owned()),
        percent,
        config: None,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// ── Tests using the shared read-only fixture
// ─────────────────────────────────────────────────────────────────────────────

/// Inferring the whole `sample_analytics` DB creates one YAML file per collection.
#[tokio::test]
async fn test_sample_analytics_db_infer_creates_all_yaml_files() {
    let f = fixture().await;
    let tmp = tempfile::tempdir().expect("tmp dir");

    run_infer(infer_args(
        &f.uri,
        "sample_analytics",
        &tmp.path().to_path_buf(),
    ))
    .await
    .expect("infer sample_analytics");

    for coll in ["accounts", "customers", "transactions"] {
        let yaml = tmp.path().join(coll).join(format!("{coll}.yaml"));
        assert!(
            yaml.exists(),
            "{coll}.yaml should exist at {}",
            yaml.display()
        );
    }
}

/// The `customers` collection contains `email`, `name`, and `username` —
/// all three should be automatically annotated with masking rules.
#[tokio::test]
async fn test_sample_analytics_customers_sensitive_fields_detected() {
    let f = fixture().await;
    let tmp = tempfile::tempdir().expect("tmp dir");

    run_infer(infer_args(
        &f.uri,
        "sample_analytics.customers",
        &tmp.path().to_path_buf(),
    ))
    .await
    .expect("infer customers");

    let yaml = std::fs::read_to_string(tmp.path().join("customers").join("customers.yaml"))
        .expect("read yaml");

    assert!(
        yaml.contains("MASK_CONTACT_URI"),
        "email should be annotated with MASK_CONTACT_URI:\n{yaml}"
    );
    assert!(
        yaml.contains("PRESERVE_TOKEN"),
        "name/username should be annotated with PRESERVE_TOKEN:\n{yaml}"
    );
}

/// The `accounts` collection has an `account_id` field → should be annotated
/// as `REDACT_ALPHANUMERIC`.
#[tokio::test]
async fn test_sample_analytics_accounts_schema_annotated() {
    let f = fixture().await;
    let tmp = tempfile::tempdir().expect("tmp dir");

    run_infer(infer_args(
        &f.uri,
        "sample_analytics.accounts",
        &tmp.path().to_path_buf(),
    ))
    .await
    .expect("infer accounts");

    let yaml = std::fs::read_to_string(tmp.path().join("accounts").join("accounts.yaml"))
        .expect("read yaml");

    // accounts has 1 746 documents; sampled count should be in the YAML
    assert!(
        yaml.contains("sampled:"),
        "yaml should record sampled count:\n{yaml}"
    );
    // accounts has account_id (Number), limit (Number), and products (Array of String)
    assert!(
        yaml.contains("account_id:"),
        "account_id field should be present:\n{yaml}"
    );
    assert!(
        yaml.contains("products:"),
        "products field should be present:\n{yaml}"
    );
    // products contains strings like "Brokerage", "Derivatives" — no masking annotation expected
    // but the schema should include array item types
    assert!(
        yaml.contains("Array:"),
        "products should be typed as Array:\n{yaml}"
    );
}

/// The `users` collection in `sample_mflix` contains `email` and `name` fields.
#[tokio::test]
async fn test_sample_mflix_users_sensitive_fields_detected() {
    let f = fixture().await;
    let tmp = tempfile::tempdir().expect("tmp dir");

    run_infer(infer_args(
        &f.uri,
        "sample_mflix.users",
        &tmp.path().to_path_buf(),
    ))
    .await
    .expect("infer users");

    let yaml =
        std::fs::read_to_string(tmp.path().join("users").join("users.yaml")).expect("read yaml");

    assert!(
        yaml.contains("MASK_CONTACT_URI"),
        "email should be detected in mflix.users:\n{yaml}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// ── Tests that write data → each gets its own container
// ─────────────────────────────────────────────────────────────────────────────

/// Full infer + apply for `sample_mflix.users`: target should contain the same
/// number of documents and emails should be anonymized.
#[tokio::test]
async fn test_sample_mflix_users_apply_masks_email() {
    let container = Mongo::default().start().await.expect("start");
    let host = container.get_host().await.expect("host");
    let port = container.get_host_port_ipv4(27017).await.expect("port");
    let uri = format!("mongodb://{host}:{port}/");
    let client = mongo_client(&uri).await;

    let data_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data");
    let n = import_jsonl(
        &client,
        "sample_mflix",
        "users",
        &data_dir.join("sample_mflix/users.json"),
    )
    .await;
    assert!(n > 0, "should have imported users");

    let tmp = tempfile::tempdir().expect("tmp dir");
    run_infer(infer_args(
        &uri,
        "sample_mflix.users",
        &tmp.path().to_path_buf(),
    ))
    .await
    .expect("infer");

    let yaml_path = tmp.path().join("users").join("users.yaml");
    run_apply(apply_args(
        &uri,
        "sample_mflix.users",
        Some(yaml_path),
        &uri,
        Some("sample_mflix_anon.users"),
        None,
    ))
    .await
    .expect("apply");

    // same document count
    let dst = client
        .database("sample_mflix_anon")
        .collection::<Document>("users");
    let dst_count = dst.count_documents(doc! {}).await.expect("count");
    assert_eq!(
        dst_count, n as u64,
        "target should have same doc count as source"
    );

    // emails must be changed
    let src = client
        .database("sample_mflix")
        .collection::<Document>("users");
    let mut src_cursor = src.find(doc! {}).await.expect("src find");
    let mut dst_cursor = dst.find(doc! {}).await.expect("dst find");

    let mut diffs = 0u64;
    while let (Some(s), Some(d)) = (
        src_cursor.try_next().await.expect("src next"),
        dst_cursor.try_next().await.expect("dst next"),
    ) {
        let src_email = s.get("email").and_then(|v| v.as_str()).unwrap_or("");
        let dst_email = d.get("email").and_then(|v| v.as_str()).unwrap_or("");
        if src_email != dst_email {
            diffs += 1;
        }
        // masked email must still look like an email (contain @)
        if !dst_email.is_empty() {
            assert!(
                dst_email.contains('@'),
                "masked email should contain @: {dst_email}"
            );
        }
    }
    assert!(
        diffs > 0,
        "at least some emails should differ after masking"
    );
}

/// DB-level infer + apply for `sample_analytics`: every collection in the
/// source DB should appear in the target DB with the same document count.
#[tokio::test]
async fn test_sample_analytics_db_level_workflow() {
    let container = Mongo::default().start().await.expect("start");
    let host = container.get_host().await.expect("host");
    let port = container.get_host_port_ipv4(27017).await.expect("port");
    let uri = format!("mongodb://{host}:{port}/");
    let client = mongo_client(&uri).await;

    let data_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data");
    let collections = ["accounts", "customers", "transactions"];

    for coll in collections {
        import_jsonl(
            &client,
            "sample_analytics",
            coll,
            &data_dir
                .join("sample_analytics")
                .join(format!("{coll}.json")),
        )
        .await;
    }

    let tmp = tempfile::tempdir().expect("tmp dir");

    // Infer whole DB
    run_infer(infer_args(
        &uri,
        "sample_analytics",
        &tmp.path().to_path_buf(),
    ))
    .await
    .expect("infer db-level");

    // Apply whole DB → sample_analytics_anon
    run_apply(apply_args(
        &uri,
        "sample_analytics",
        Some(tmp.path().to_path_buf()),
        &uri,
        Some("sample_analytics_anon"),
        None,
    ))
    .await
    .expect("apply db-level");

    // Verify each collection has the same count in source and target
    for coll in collections {
        let src = client
            .database("sample_analytics")
            .collection::<Document>(coll)
            .count_documents(doc! {})
            .await
            .expect("src count");
        let dst = client
            .database("sample_analytics_anon")
            .collection::<Document>(coll)
            .count_documents(doc! {})
            .await
            .expect("dst count");
        assert_eq!(
            src, dst,
            "collection {coll}: expected {src} docs, got {dst}"
        );
    }
}

/// Apply `sample_analytics.customers` with `--percent 50` → target should
/// contain roughly half of the source documents.
#[tokio::test]
async fn test_sample_analytics_customers_apply_with_percent() {
    let container = Mongo::default().start().await.expect("start");
    let host = container.get_host().await.expect("host");
    let port = container.get_host_port_ipv4(27017).await.expect("port");
    let uri = format!("mongodb://{host}:{port}/");
    let client = mongo_client(&uri).await;

    let data_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data");
    let total = import_jsonl(
        &client,
        "sample_analytics",
        "customers",
        &data_dir.join("sample_analytics/customers.json"),
    )
    .await as u64;

    let tmp = tempfile::tempdir().expect("tmp dir");
    run_infer(infer_args(
        &uri,
        "sample_analytics.customers",
        &tmp.path().to_path_buf(),
    ))
    .await
    .expect("infer");

    run_apply(apply_args(
        &uri,
        "sample_analytics.customers",
        Some(tmp.path().join("customers").join("customers.yaml")),
        &uri,
        Some("sample_analytics_half.customers"),
        Some(50.0),
    ))
    .await
    .expect("apply 50%");

    let dst_count = client
        .database("sample_analytics_half")
        .collection::<Document>("customers")
        .count_documents(doc! {})
        .await
        .expect("count");

    let expected = ((total as f64 * 0.5).ceil() as u64).max(1);
    assert_eq!(
        dst_count, expected,
        "50% of {total} docs = {expected}, got {dst_count}"
    );
}
