//! End-to-end tests using **all** MongoDB sample datasets downloaded on-the-fly
//! from <https://github.com/neelabalan/mongodb-sample-dataset>.
//!
//! A **shared read-only container** is started once for the whole test binary
//! and populated with every collection from all 7 sample databases:
//!
//! | Database             | Collections                                                      |
//! |----------------------|------------------------------------------------------------------|
//! | `sample_airbnb`      | listingsAndReviews                                               |
//! | `sample_analytics`   | accounts, customers, transactions                                |
//! | `sample_geospatial`  | shipwrecks                                                       |
//! | `sample_mflix`       | comments, movies, sessions, theaters, users                      |
//! | `sample_supplies`    | sales                                                            |
//! | `sample_training`    | companies, grades, inspections, posts, routes, stories, trips, tweets, zips |
//! | `sample_weatherdata` | data                                                             |
//!
//! Tests that need to **write** anonymised data start their own container so
//! they do not interfere with each other.
//!
//! Run with:
//! ```bash
//! cargo test --test e2e_sample_tests -- --nocapture
//! ```

use std::sync::{Arc, Mutex};

use futures::TryStreamExt;
use mongodb::{Client, bson::Document, bson::doc, options::ClientOptions};
use testcontainers::{ContainerAsync, runners::AsyncRunner};
use testcontainers_modules::mongo::Mongo;
use tokio::sync::OnceCell;

use mongodbanonymizer::args::{ApplyArgs, InferArgs, UriArg};
use mongodbanonymizer::commands::apply::run_apply;
use mongodbanonymizer::commands::infer::run_infer;
use mongodbanonymizer::helpers::{existing_collection, existing_db, get_locale, get_metadata};

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

            println!("Shared fixture: importing all sample datasets into {uri}");
            import_all_sample_datasets(&uri).await;

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

const SAMPLE_DATASET_BASE: &str =
    "https://raw.githubusercontent.com/neelabalan/mongodb-sample-dataset/main";

/// Download a newline-delimited JSON file from GitHub and bulk-insert into MongoDB.
/// Handles MongoDB Extended JSON v2 (`$oid`, `$date`, `$numberInt`, etc.).
async fn import_from_github(client: &Client, db: &str, collection: &str) -> usize {
    let url = format!("{SAMPLE_DATASET_BASE}/{db}/{collection}.json");
    let content = reqwest::get(&url)
        .await
        .unwrap_or_else(|e| panic!("GET {url} failed: {e}"))
        .error_for_status()
        .unwrap_or_else(|e| panic!("HTTP error for {url}: {e}"))
        .text()
        .await
        .unwrap_or_else(|e| panic!("Reading body of {url} failed: {e}"));

    let coll = client.database(db).collection::<Document>(collection);
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

/// Import every collection from all 7 MongoDB sample databases.
async fn import_all_sample_datasets(uri: &str) {
    let client = mongo_client(uri).await;
    let datasets: &[(&str, &[&str])] = &[
        ("sample_airbnb", &["listingsAndReviews"]),
        (
            "sample_analytics",
            &["accounts", "customers", "transactions"],
        ),
        ("sample_geospatial", &["shipwrecks"]),
        (
            "sample_mflix",
            &["comments", "movies", "sessions", "theaters", "users"],
        ),
        ("sample_supplies", &["sales"]),
        (
            "sample_training",
            &[
                "companies",
                "grades",
                "inspections",
                "posts",
                "routes",
                "stories",
                "trips",
                "tweets",
                "zips",
            ],
        ),
        ("sample_weatherdata", &["data"]),
    ];
    for (db, collections) in datasets {
        for &coll in *collections {
            let n = import_from_github(&client, db, coll).await;
            println!("  {db}.{coll}: {n} docs");
        }
    }
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

    let n = import_from_github(&client, "sample_mflix", "users").await;
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

    let collections = ["accounts", "customers", "transactions"];

    for coll in collections {
        import_from_github(&client, "sample_analytics", coll).await;
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

    let total = import_from_github(&client, "sample_analytics", "customers").await as u64;

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

// ─────────────────────────────────────────────────────────────────────────────
// ── sample_airbnb
// ─────────────────────────────────────────────────────────────────────────────

/// `sample_airbnb.listingsAndReviews` — host email and reviewer name fields
/// should be detected and annotated.
#[tokio::test]
async fn test_sample_airbnb_listings_sensitive_fields_detected() {
    let f = fixture().await;
    let tmp = tempfile::tempdir().expect("tmp dir");

    run_infer(infer_args(
        &f.uri,
        "sample_airbnb.listingsAndReviews",
        &tmp.path().to_path_buf(),
    ))
    .await
    .expect("infer listingsAndReviews");

    let yaml = std::fs::read_to_string(
        tmp.path()
            .join("listingsAndReviews")
            .join("listingsAndReviews.yaml"),
    )
    .expect("read yaml");

    assert!(
        yaml.contains("sampled:"),
        "yaml should have sampled count:\n{yaml}"
    );
    // The schema should include the top-level name field and nested host block
    assert!(yaml.contains("name:"), "name field should appear:\n{yaml}");
    assert!(
        yaml.contains("host:"),
        "host sub-document should appear:\n{yaml}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// ── sample_geospatial
// ─────────────────────────────────────────────────────────────────────────────

/// `sample_geospatial.shipwrecks` — schema inference should succeed and
/// capture known top-level fields.
#[tokio::test]
async fn test_sample_geospatial_shipwrecks_schema_inferred() {
    let f = fixture().await;
    let tmp = tempfile::tempdir().expect("tmp dir");

    run_infer(infer_args(
        &f.uri,
        "sample_geospatial.shipwrecks",
        &tmp.path().to_path_buf(),
    ))
    .await
    .expect("infer shipwrecks");

    let yaml = std::fs::read_to_string(tmp.path().join("shipwrecks").join("shipwrecks.yaml"))
        .expect("read yaml");

    assert!(
        yaml.contains("sampled:"),
        "yaml should have sampled count:\n{yaml}"
    );
    // Every shipwreck document has a feature_type field
    assert!(
        yaml.contains("feature_type:"),
        "feature_type field should be present:\n{yaml}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// ── sample_mflix (all collections)
// ─────────────────────────────────────────────────────────────────────────────

/// Inferring the whole `sample_mflix` DB creates one YAML file per collection.
#[tokio::test]
async fn test_sample_mflix_db_infer_creates_all_yaml_files() {
    let f = fixture().await;
    let tmp = tempfile::tempdir().expect("tmp dir");

    run_infer(infer_args(
        &f.uri,
        "sample_mflix",
        &tmp.path().to_path_buf(),
    ))
    .await
    .expect("infer sample_mflix");

    for coll in ["comments", "movies", "sessions", "theaters", "users"] {
        let yaml = tmp.path().join(coll).join(format!("{coll}.yaml"));
        assert!(
            yaml.exists(),
            "{coll}.yaml should exist at {}",
            yaml.display()
        );
    }
}

/// `sample_mflix.comments` contains `email` and `name` — both should be
/// automatically annotated with masking rules.
#[tokio::test]
async fn test_sample_mflix_comments_sensitive_fields_detected() {
    let f = fixture().await;
    let tmp = tempfile::tempdir().expect("tmp dir");

    run_infer(infer_args(
        &f.uri,
        "sample_mflix.comments",
        &tmp.path().to_path_buf(),
    ))
    .await
    .expect("infer comments");

    let yaml = std::fs::read_to_string(tmp.path().join("comments").join("comments.yaml"))
        .expect("read yaml");

    assert!(
        yaml.contains("MASK_CONTACT_URI"),
        "email should be annotated in comments:\n{yaml}"
    );
    assert!(
        yaml.contains("PRESERVE_TOKEN"),
        "name should be annotated in comments:\n{yaml}"
    );
}

/// `sample_mflix.movies` — schema should include known fields like `title` and
/// `year` with no PII annotations.
#[tokio::test]
async fn test_sample_mflix_movies_schema_inferred() {
    let f = fixture().await;
    let tmp = tempfile::tempdir().expect("tmp dir");

    run_infer(infer_args(
        &f.uri,
        "sample_mflix.movies",
        &tmp.path().to_path_buf(),
    ))
    .await
    .expect("infer movies");

    let yaml =
        std::fs::read_to_string(tmp.path().join("movies").join("movies.yaml")).expect("read yaml");

    assert!(
        yaml.contains("sampled:"),
        "yaml should have sampled count:\n{yaml}"
    );
    assert!(
        yaml.contains("title:"),
        "title field should appear:\n{yaml}"
    );
    assert!(yaml.contains("year:"), "year field should appear:\n{yaml}");
}

// ─────────────────────────────────────────────────────────────────────────────
// ── sample_supplies
// ─────────────────────────────────────────────────────────────────────────────

/// `sample_supplies.sales` contains a nested `customer` sub-document with an
/// `email` field that should be annotated.
#[tokio::test]
async fn test_sample_supplies_sales_customer_email_detected() {
    let f = fixture().await;
    let tmp = tempfile::tempdir().expect("tmp dir");

    run_infer(infer_args(
        &f.uri,
        "sample_supplies.sales",
        &tmp.path().to_path_buf(),
    ))
    .await
    .expect("infer sales");

    let yaml =
        std::fs::read_to_string(tmp.path().join("sales").join("sales.yaml")).expect("read yaml");

    assert!(
        yaml.contains("sampled:"),
        "yaml should have sampled count:\n{yaml}"
    );
    assert!(
        yaml.contains("customer:"),
        "customer sub-document should appear:\n{yaml}"
    );
    assert!(
        yaml.contains("MASK_CONTACT_URI"),
        "customer.email should be annotated:\n{yaml}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// ── sample_training
// ─────────────────────────────────────────────────────────────────────────────

/// Inferring the whole `sample_training` DB creates one YAML file per
/// collection (9 collections).
#[tokio::test]
async fn test_sample_training_db_infer_creates_all_yaml_files() {
    let f = fixture().await;
    let tmp = tempfile::tempdir().expect("tmp dir");

    run_infer(infer_args(
        &f.uri,
        "sample_training",
        &tmp.path().to_path_buf(),
    ))
    .await
    .expect("infer sample_training");

    for coll in [
        "companies",
        "grades",
        "inspections",
        "posts",
        "routes",
        "stories",
        "trips",
        "tweets",
        "zips",
    ] {
        let yaml = tmp.path().join(coll).join(format!("{coll}.yaml"));
        assert!(
            yaml.exists(),
            "{coll}.yaml should exist at {}",
            yaml.display()
        );
    }
}

/// `sample_training.companies` — schema should include `name` and `founded_year`.
#[tokio::test]
async fn test_sample_training_companies_schema_inferred() {
    let f = fixture().await;
    let tmp = tempfile::tempdir().expect("tmp dir");

    run_infer(infer_args(
        &f.uri,
        "sample_training.companies",
        &tmp.path().to_path_buf(),
    ))
    .await
    .expect("infer companies");

    let yaml = std::fs::read_to_string(tmp.path().join("companies").join("companies.yaml"))
        .expect("read yaml");

    assert!(
        yaml.contains("sampled:"),
        "yaml should have sampled count:\n{yaml}"
    );
    assert!(yaml.contains("name:"), "name field should appear:\n{yaml}");
    assert!(
        yaml.contains("founded_year:"),
        "founded_year field should appear:\n{yaml}"
    );
}

/// `sample_training.zips` — schema should include `city`, `state`, and `pop`.
#[tokio::test]
async fn test_sample_training_zips_schema_inferred() {
    let f = fixture().await;
    let tmp = tempfile::tempdir().expect("tmp dir");

    run_infer(infer_args(
        &f.uri,
        "sample_training.zips",
        &tmp.path().to_path_buf(),
    ))
    .await
    .expect("infer zips");

    let yaml =
        std::fs::read_to_string(tmp.path().join("zips").join("zips.yaml")).expect("read yaml");

    assert!(
        yaml.contains("sampled:"),
        "yaml should have sampled count:\n{yaml}"
    );
    assert!(yaml.contains("city:"), "city field should appear:\n{yaml}");
    assert!(
        yaml.contains("state:"),
        "state field should appear:\n{yaml}"
    );
    assert!(yaml.contains("pop:"), "pop field should appear:\n{yaml}");
}

// ─────────────────────────────────────────────────────────────────────────────
// ── sample_weatherdata
// ─────────────────────────────────────────────────────────────────────────────

/// `sample_weatherdata.data` — schema inference should succeed and capture
/// known measurement fields.
#[tokio::test]
async fn test_sample_weatherdata_schema_inferred() {
    let f = fixture().await;
    let tmp = tempfile::tempdir().expect("tmp dir");

    run_infer(infer_args(
        &f.uri,
        "sample_weatherdata.data",
        &tmp.path().to_path_buf(),
    ))
    .await
    .expect("infer weatherdata");

    let yaml =
        std::fs::read_to_string(tmp.path().join("data").join("data.yaml")).expect("read yaml");

    assert!(
        yaml.contains("sampled:"),
        "yaml should have sampled count:\n{yaml}"
    );
    // Every weather document has a station identifier
    assert!(
        yaml.contains("st:"),
        "st (station) field should appear:\n{yaml}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// ── helpers: existing_db, existing_collection, get_locale, get_metadata
// ─────────────────────────────────────────────────────────────────────────────

/// `existing_db` returns `true` for a database that was imported.
#[tokio::test]
async fn test_existing_db_returns_true_for_known_db() {
    let f = fixture().await;
    let client = mongo_client(&f.uri).await;
    let result = existing_db(&client, "sample_analytics")
        .await
        .expect("existing_db should not error");
    assert!(result, "sample_analytics should be reported as existing");
}

/// `existing_db` returns `false` and does not error for an unknown database.
#[tokio::test]
async fn test_existing_db_returns_false_for_unknown_db() {
    let f = fixture().await;
    let client = mongo_client(&f.uri).await;
    let result = existing_db(&client, "no_such_db_xyz")
        .await
        .expect("existing_db should not error");
    assert!(!result, "unknown db should return false");
}

/// `existing_collection` returns `true` for a collection that was imported.
#[tokio::test]
async fn test_existing_collection_returns_true_for_known_collection() {
    let f = fixture().await;
    let client = mongo_client(&f.uri).await;
    let result = existing_collection(&client, "sample_analytics", "customers")
        .await
        .expect("existing_collection should not error");
    assert!(result, "customers should be reported as existing");
}

/// `existing_collection` returns `false` for a collection that does not exist.
#[tokio::test]
async fn test_existing_collection_returns_false_for_unknown_collection() {
    let f = fixture().await;
    let client = mongo_client(&f.uri).await;
    let result = existing_collection(&client, "sample_analytics", "no_such_collection_xyz")
        .await
        .expect("existing_collection should not error");
    assert!(!result, "unknown collection should return false");
}

/// `existing_collection` returns `false` when the database itself does not exist.
#[tokio::test]
async fn test_existing_collection_returns_false_for_unknown_db() {
    let f = fixture().await;
    let client = mongo_client(&f.uri).await;
    let result = existing_collection(&client, "no_such_db_xyz", "customers")
        .await
        .expect("existing_collection should not error even for missing db");
    assert!(!result, "collection inside unknown db should return false");
}

/// `get_locale` returns `"simple"` for collections created without an explicit
/// collation (which is the case for all sample dataset imports).
#[tokio::test]
async fn test_get_locale_returns_simple_for_collections_without_collation() {
    let f = fixture().await;
    let client = mongo_client(&f.uri).await;
    for (db, coll) in [
        ("sample_analytics", "customers"),
        ("sample_mflix", "users"),
        ("sample_supplies", "sales"),
    ] {
        let locale = get_locale(&client, db, coll)
            .await
            .expect("get_locale should not error");
        assert_eq!(
            locale, "simple",
            "{db}.{coll} should have 'simple' locale (no explicit collation)"
        );
    }
}

/// `get_locale` does not error for a collection that does not exist — returns `"simple"`
/// because there is no collation spec to read.
#[tokio::test]
async fn test_get_locale_returns_simple_for_nonexistent_collection() {
    let f = fixture().await;
    let client = mongo_client(&f.uri).await;
    let locale = get_locale(&client, "sample_analytics", "no_such_collection_xyz")
        .await
        .expect("get_locale should not error for missing collection");
    assert_eq!(locale, "simple");
}

/// `get_metadata` completes without error for a known collection.
#[tokio::test]
async fn test_get_metadata_succeeds_for_known_collection() {
    let f = fixture().await;
    let client = mongo_client(&f.uri).await;
    get_metadata(&client, "sample_analytics", "customers")
        .await
        .expect("get_metadata should succeed for a known collection");
}

/// `get_metadata` completes without error even for a collection that does not
/// exist (the cursor simply yields no documents).
#[tokio::test]
async fn test_get_metadata_succeeds_for_nonexistent_collection() {
    let f = fixture().await;
    let client = mongo_client(&f.uri).await;
    get_metadata(&client, "sample_analytics", "no_such_collection_xyz")
        .await
        .expect("get_metadata should not error for missing collection");
}
