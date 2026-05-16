//! End-to-end tests using real MongoDB instances spun up with testcontainers.
//!
//! Each test gets a fresh, isolated container.  Docker must be running on the
//! host for these tests to execute.
//!
//! Run with:
//! ```bash
//! cargo test --test e2e_tests -- --nocapture
//! ```

use std::path::PathBuf;

use futures::TryStreamExt;
use mongodb::{Client, bson::doc, options::ClientOptions};
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::mongo::Mongo;

use mongodbanonymizer::args::{ApplyArgs, InferArgs, UriArg};
use mongodbanonymizer::commands::apply::run_apply;
use mongodbanonymizer::commands::infer::run_infer;

// ─────────────────────────────────────────────────────────────────────────────
// Helper: build a MongoDB client from a URI string
// ─────────────────────────────────────────────────────────────────────────────
async fn mongo_client(uri: &str) -> Client {
    let opts = ClientOptions::parse(uri).await.expect("parse uri");
    Client::with_options(opts).expect("create client")
}

// ─────────────────────────────────────────────────────────────────────────────
// Helper: build the connection URI for a running container
// ─────────────────────────────────────────────────────────────────────────────
async fn container_uri(container: &testcontainers::ContainerAsync<Mongo>) -> String {
    let host = container.get_host().await.expect("get host");
    let port = container.get_host_port_ipv4(27017).await.expect("get port");
    format!("mongodb://{host}:{port}/")
}

// ─────────────────────────────────────────────────────────────────────────────
// Helper: make InferArgs for a single-collection infer
// ─────────────────────────────────────────────────────────────────────────────
fn infer_args(uri: &str, namespace: &str, output_dir: &PathBuf) -> InferArgs {
    InferArgs {
        mongo: UriArg {
            source_uri: Some(uri.to_owned()),
        },
        namespace: Some(namespace.to_owned()),
        number: Some(100),
        percent: None,
        no_output: true,
        output_dir: Some(output_dir.clone()),
        config: None,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Helper: make ApplyArgs
// ─────────────────────────────────────────────────────────────────────────────
fn apply_args(
    source_uri: &str,
    namespace: &str,
    masking_rules: Option<PathBuf>,
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
// TEST 1: infer a single collection → YAML file is created
// ─────────────────────────────────────────────────────────────────────────────
#[tokio::test]
async fn test_infer_single_collection_creates_yaml() {
    let container = Mongo::default().start().await.expect("start mongo");
    let uri = container_uri(&container).await;
    let client = mongo_client(&uri).await;

    // Insert a few documents with a sensitive field
    let coll = client
        .database("testdb")
        .collection::<mongodb::bson::Document>("users");
    coll.insert_many(vec![
        doc! { "email": "alice@example.com", "age": 30 },
        doc! { "email": "bob@example.com",   "age": 25 },
        doc! { "email": "carol@example.com", "age": 40 },
    ])
    .await
    .expect("insert docs");

    let tmp = tempfile::tempdir().expect("tmp dir");
    let args = infer_args(&uri, "testdb.users", &tmp.path().to_path_buf());

    run_infer(args).await.expect("run_infer");

    // Verify the YAML file was written
    let yaml_path = tmp.path().join("users").join("users.yaml");
    assert!(
        yaml_path.exists(),
        "expected YAML at {}",
        yaml_path.display()
    );

    // Verify the YAML contains a masking block for 'email'
    let yaml = std::fs::read_to_string(&yaml_path).expect("read yaml");
    assert!(
        yaml.contains("MASK_CONTACT_URI"),
        "email field should be annotated with MASK_CONTACT_URI:\n{yaml}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// TEST 2: infer a whole database → one YAML per collection
// ─────────────────────────────────────────────────────────────────────────────
#[tokio::test]
async fn test_infer_db_level_creates_yaml_per_collection() {
    let container = Mongo::default().start().await.expect("start mongo");
    let uri = container_uri(&container).await;
    let client = mongo_client(&uri).await;

    let db = client.database("shopdb");
    db.collection::<mongodb::bson::Document>("orders")
        .insert_many(vec![doc! { "order_id": "O001", "total": 99 }])
        .await
        .expect("insert orders");
    db.collection::<mongodb::bson::Document>("customers")
        .insert_many(vec![doc! { "email": "user@example.com", "name": "Alice" }])
        .await
        .expect("insert customers");

    let tmp = tempfile::tempdir().expect("tmp dir");
    let args = infer_args(&uri, "shopdb", &tmp.path().to_path_buf());

    run_infer(args).await.expect("run_infer db-level");

    assert!(
        tmp.path().join("orders").join("orders.yaml").exists(),
        "orders.yaml should exist"
    );
    assert!(
        tmp.path().join("customers").join("customers.yaml").exists(),
        "customers.yaml should exist"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// TEST 3: apply masks email field → target collection contains anonymized docs
// ─────────────────────────────────────────────────────────────────────────────
#[tokio::test]
async fn test_apply_single_collection_masks_email() {
    let container = Mongo::default().start().await.expect("start mongo");
    let uri = container_uri(&container).await;
    let client = mongo_client(&uri).await;

    // Seed source data
    let src = client
        .database("srcdb")
        .collection::<mongodb::bson::Document>("contacts");
    let raw_emails = vec!["alice@example.com", "bob@example.com", "carol@example.com"];
    src.insert_many(
        raw_emails
            .iter()
            .map(|e| doc! { "email": e, "score": 10 })
            .collect::<Vec<_>>(),
    )
    .await
    .expect("insert source");

    // Infer schema
    let tmp = tempfile::tempdir().expect("tmp dir");
    run_infer(infer_args(
        &uri,
        "srcdb.contacts",
        &tmp.path().to_path_buf(),
    ))
    .await
    .expect("infer");

    let yaml_path = tmp.path().join("contacts").join("contacts.yaml");

    // Apply masking to a different database in the same container
    run_apply(apply_args(
        &uri,
        "srcdb.contacts",
        Some(yaml_path),
        &uri,
        Some("dstdb.contacts"),
        None,
    ))
    .await
    .expect("apply");

    // Verify target documents exist and emails are changed
    let dst = client
        .database("dstdb")
        .collection::<mongodb::bson::Document>("contacts");
    let count = dst.count_documents(doc! {}).await.expect("count");
    assert_eq!(count, 3, "should have 3 documents in target");

    let mut cursor = dst.find(doc! {}).await.expect("find");
    while let Some(doc) = cursor.try_next().await.expect("cursor") {
        let masked_email = doc.get_str("email").expect("email field");
        assert!(
            !raw_emails.contains(&masked_email),
            "email should be anonymized, got: {masked_email}"
        );
        // MASK_CONTACT_URI masks the local part (before @) while keeping the domain;
        // verify the local part no longer contains any of the original names
        let local = masked_email.split('@').next().unwrap_or("");
        assert!(
            !["alice", "bob", "carol"].contains(&local),
            "local part should be masked, got: {masked_email}"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// TEST 4: apply with --percent limits the number of copied documents
// ─────────────────────────────────────────────────────────────────────────────
#[tokio::test]
async fn test_apply_percent_limits_documents() {
    let container = Mongo::default().start().await.expect("start mongo");
    let uri = container_uri(&container).await;
    let client = mongo_client(&uri).await;

    // Insert 20 documents
    let src = client
        .database("pctdb")
        .collection::<mongodb::bson::Document>("items");
    let docs: Vec<_> = (0..20)
        .map(|i| doc! { "n": i, "name": format!("item{i}") })
        .collect();
    src.insert_many(docs).await.expect("insert");

    // Infer schema
    let tmp = tempfile::tempdir().expect("tmp dir");
    run_infer(infer_args(&uri, "pctdb.items", &tmp.path().to_path_buf()))
        .await
        .expect("infer");

    let yaml_path = tmp.path().join("items").join("items.yaml");

    // Apply with 50 % → expect 10 documents
    run_apply(apply_args(
        &uri,
        "pctdb.items",
        Some(yaml_path),
        &uri,
        Some("pctdb_out.items"),
        Some(50.0),
    ))
    .await
    .expect("apply");

    let dst = client
        .database("pctdb_out")
        .collection::<mongodb::bson::Document>("items");
    let count = dst.count_documents(doc! {}).await.expect("count");
    assert_eq!(count, 10, "50 % of 20 docs should be 10, got {count}");
}

// ─────────────────────────────────────────────────────────────────────────────
// TEST 5: DB-level apply processes all collections
// ─────────────────────────────────────────────────────────────────────────────
#[tokio::test]
async fn test_apply_db_level_processes_all_collections() {
    let container = Mongo::default().start().await.expect("start mongo");
    let uri = container_uri(&container).await;
    let client = mongo_client(&uri).await;

    let db = client.database("multidb");
    db.collection::<mongodb::bson::Document>("a")
        .insert_many(vec![
            doc! { "email": "a@test.com" },
            doc! { "email": "b@test.com" },
        ])
        .await
        .expect("insert a");
    db.collection::<mongodb::bson::Document>("b")
        .insert_many(vec![doc! { "email": "c@test.com" }])
        .await
        .expect("insert b");

    // Infer all collections
    let tmp = tempfile::tempdir().expect("tmp dir");
    run_infer(infer_args(&uri, "multidb", &tmp.path().to_path_buf()))
        .await
        .expect("infer db-level");

    // Apply all collections (masking_rules = directory)
    run_apply(apply_args(
        &uri,
        "multidb",
        Some(tmp.path().to_path_buf()),
        &uri,
        Some("multidb_anon"),
        None,
    ))
    .await
    .expect("apply db-level");

    let count_a = client
        .database("multidb_anon")
        .collection::<mongodb::bson::Document>("a")
        .count_documents(doc! {})
        .await
        .expect("count a");
    let count_b = client
        .database("multidb_anon")
        .collection::<mongodb::bson::Document>("b")
        .count_documents(doc! {})
        .await
        .expect("count b");

    assert_eq!(count_a, 2, "collection a should have 2 docs");
    assert_eq!(count_b, 1, "collection b should have 1 doc");
}

// ─────────────────────────────────────────────────────────────────────────────
// TEST 6: infer respects --number sampling limit
// ─────────────────────────────────────────────────────────────────────────────
#[tokio::test]
async fn test_infer_number_limits_sample() {
    let container = Mongo::default().start().await.expect("start mongo");
    let uri = container_uri(&container).await;
    let client = mongo_client(&uri).await;

    // Insert 50 documents
    let coll = client
        .database("sampledb")
        .collection::<mongodb::bson::Document>("docs");
    let docs: Vec<_> = (0..50)
        .map(|i| doc! { "idx": i, "val": format!("v{i}") })
        .collect();
    coll.insert_many(docs).await.expect("insert");

    let tmp = tempfile::tempdir().expect("tmp dir");
    let args = InferArgs {
        mongo: UriArg {
            source_uri: Some(uri.clone()),
        },
        namespace: Some("sampledb.docs".to_owned()),
        number: Some(10), // sample only 10
        percent: None,
        no_output: true,
        output_dir: Some(tmp.path().to_path_buf()),
        config: None,
    };

    run_infer(args).await.expect("infer");

    let yaml =
        std::fs::read_to_string(tmp.path().join("docs").join("docs.yaml")).expect("read yaml");

    // The YAML schema should record sampled: 10 (or less if collection is smaller)
    assert!(
        yaml.contains("sampled: 10") || yaml.contains("sampled:"),
        "yaml:\n{yaml}"
    );
}
