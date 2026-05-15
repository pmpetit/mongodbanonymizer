//! Integration tests for schema inference, masking annotation, value masking,
//! and live-document anonymisation.
//!
//! No live MongoDB connection is required: all tests use in-memory BSON
//! documents, hand-built schemas, or direct calls to the masking functions.

use std::collections::HashMap;

use indexmap::IndexMap;
use mongodb::bson::{self, Bson, doc};
use mongodbanonymizer::analyzer::{Analyzer, annotate_masking, mask_sampled_values};
use mongodbanonymizer::commands::apply::apply_masking_to_doc;
use mongodbanonymizer::masking;
use mongodbanonymizer::models::{CollectionSchema, FieldSchema, MaskingConfig, TypeSchema};

// ──────────────────────────────────────────────────────────────────────────────
// Helpers
// ──────────────────────────────────────────────────────────────────────────────

/// Feed BSON documents through the `Analyzer` and return the finished schema.
fn analyze_docs(docs: &[bson::Document]) -> CollectionSchema {
    let mut analyzer = Analyzer::new(true);
    for doc in docs {
        analyzer.process_document(doc);
    }
    analyzer.finish()
}

/// Build a minimal `FieldSchema` with a single String type carrying a masking rule.
fn string_field_with_masking(method: &str) -> FieldSchema {
    let ts = TypeSchema {
        masking: Some(MaskingConfig {
            enabled: true,
            method: method.to_owned(),
        }),
        probability: 1.0,
        sampled: 1,
        as_jsonb: false,
        ndistinct: Some(1.0),
        object: None,
        array: None,
        values: Some(vec![serde_json::Value::String("original".into())]),
    };
    let mut types = IndexMap::new();
    types.insert("String".to_owned(), ts);
    FieldSchema {
        probability: 1.0,
        types,
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Analyzer — schema inference
// ──────────────────────────────────────────────────────────────────────────────

#[test]
fn test_basic_field_count() {
    let docs = vec![
        doc! { "_id": 1, "name": "Alice" },
        doc! { "_id": 2, "name": "Bob" },
        doc! { "_id": 3, "name": "Carol" },
    ];
    let schema = analyze_docs(&docs);
    assert_eq!(schema.count, 3);
    assert!(schema.object.contains_key("_id"));
    assert!(schema.object.contains_key("name"));
}

#[test]
fn test_id_sorted_first() {
    let docs = vec![doc! { "zebra": "z", "_id": 1, "apple": "a" }];
    let schema = analyze_docs(&docs);
    let keys: Vec<&str> = schema.object.keys().map(|s: &String| s.as_str()).collect();
    assert_eq!(keys[0], "_id", "_id must be the first key");
}

#[test]
fn test_remaining_fields_sorted_alphabetically() {
    let docs = vec![doc! { "_id": 1, "zoo": 1, "alpha": 2, "beta": 3 }];
    let schema = analyze_docs(&docs);
    let keys: Vec<&str> = schema.object.keys().map(|s: &String| s.as_str()).collect();
    assert_eq!(keys[0], "_id");
    assert_eq!(keys[1], "alpha");
    assert_eq!(keys[2], "beta");
    assert_eq!(keys[3], "zoo");
}

#[test]
fn test_numeric_type_mapped_to_number() {
    let docs = vec![
        doc! { "_id": 1, "score": 42_i32 },
        doc! { "_id": 2, "score": 3.14_f64 },
    ];
    let schema = analyze_docs(&docs);
    let score = schema.object.get("score").expect("score field missing");
    assert!(
        score.types.contains_key("Number"),
        "Int32 and f64 should both map to 'Number'"
    );
}

#[test]
fn test_undefined_injected_for_missing_fields() {
    let docs = vec![doc! { "_id": 1, "optional": "present" }, doc! { "_id": 2 }];
    let schema = analyze_docs(&docs);
    let field = schema.object.get("optional").expect("optional missing");
    assert!(
        field.types.contains_key("Undefined"),
        "Undefined must be injected when field is absent in some docs"
    );
}

#[test]
fn test_probability_computed_correctly() {
    let docs = vec![
        doc! { "_id": 1, "x": 1 },
        doc! { "_id": 2, "x": 2 },
        doc! { "_id": 3 },
    ];
    let schema = analyze_docs(&docs);
    let x = schema.object.get("x").unwrap();
    let expected = 2.0 / 3.0;
    assert!(
        (x.probability - expected).abs() < 1e-9,
        "probability should be {expected} but was {}",
        x.probability
    );
}

#[test]
fn test_nested_object_schema() {
    let docs = vec![doc! {
        "_id": 1,
        "address": { "city": "Paris", "zip": "75001" }
    }];
    let schema = analyze_docs(&docs);
    let address = schema.object.get("address").expect("address missing");
    let obj_type = address.types.get("Object").expect("Object type missing");
    let nested = obj_type
        .object
        .as_ref()
        .expect("nested object schema missing");
    assert!(nested.contains_key("city"), "nested city field missing");
    assert!(nested.contains_key("zip"), "nested zip field missing");
}

#[test]
fn test_array_type_detected() {
    let docs = vec![doc! { "_id": 1, "tags": ["rust", "mongodb"] }];
    let schema = analyze_docs(&docs);
    let tags = schema.object.get("tags").expect("tags missing");
    assert!(tags.types.contains_key("Array"), "Array type expected");
    let arr_type = tags.types.get("Array").unwrap();
    assert!(
        arr_type.array.is_some(),
        "array items schema should be present"
    );
}

#[test]
fn test_sample_values_collected() {
    let docs: Vec<bson::Document> = (0..5)
        .map(|i| doc! { "_id": i, "name": format!("user{i}") })
        .collect();
    let schema = analyze_docs(&docs);
    let name = schema.object.get("name").unwrap();
    let str_type = name.types.get("String").unwrap();
    let values = str_type
        .values
        .as_ref()
        .expect("values should be collected");
    assert!(!values.is_empty(), "should have sampled values");
}

// ──────────────────────────────────────────────────────────────────────────────
// Masking annotation — annotate_masking
// ──────────────────────────────────────────────────────────────────────────────

#[test]
fn test_annotate_masking_attaches_method_to_string_field() {
    let docs = vec![doc! { "_id": 1, "email": "alice@example.com" }];
    let mut schema = analyze_docs(&docs);

    let mut map = HashMap::new();
    map.insert("email".to_owned(), "MASK_CONTACT_URI".to_owned());
    annotate_masking(&mut schema, &map);

    let email = schema.object.get("email").unwrap();
    let ts = email.types.get("String").expect("String type missing");
    let mc = ts
        .masking
        .as_ref()
        .expect("masking config missing after annotation");
    assert!(mc.enabled);
    assert_eq!(mc.method, "MASK_CONTACT_URI");
}

#[test]
fn test_annotate_masking_array_inner_items_annotated() {
    // An Array field: masking should be placed on the inner String items, not on the Array itself.
    let docs = vec![doc! { "_id": 1, "aliases": ["bob", "robert"] }];
    let mut schema = analyze_docs(&docs);

    let mut map = HashMap::new();
    map.insert("aliases".to_owned(), "PRESERVE_TOKEN".to_owned());
    annotate_masking(&mut schema, &map);

    let aliases = schema.object.get("aliases").unwrap();

    // Array TypeSchema itself must NOT carry masking
    let arr_ts = aliases.types.get("Array").expect("Array type missing");
    assert!(
        arr_ts.masking.is_none(),
        "Array TypeSchema itself must not be annotated"
    );

    // Inner String items must carry masking
    let inner_str = arr_ts
        .array
        .as_ref()
        .expect("array items schema missing")
        .types
        .get("String")
        .expect("inner String type missing");
    let mc = inner_str
        .masking
        .as_ref()
        .expect("inner String must have masking");
    assert!(mc.enabled);
    assert_eq!(mc.method, "PRESERVE_TOKEN");
}

#[test]
fn test_annotate_masking_unknown_field_unchanged() {
    let docs = vec![doc! { "_id": 1, "score": 42_i32 }];
    let mut schema = analyze_docs(&docs);
    // No entry for "score" in the map → field stays unannotated
    annotate_masking(&mut schema, &HashMap::new());
    let score = schema.object.get("score").unwrap();
    let num_ts = score.types.get("Number").unwrap();
    assert!(num_ts.masking.is_none());
}

// ──────────────────────────────────────────────────────────────────────────────
// Value masking — mask_sampled_values
// ──────────────────────────────────────────────────────────────────────────────

#[test]
fn test_mask_sampled_values_replaces_string_values() {
    let docs = vec![
        doc! { "_id": 1, "email": "alice@example.com" },
        doc! { "_id": 2, "email": "bob@example.com" },
    ];
    let mut schema = analyze_docs(&docs);

    let mut map = HashMap::new();
    map.insert("email".to_owned(), "MASK_CONTACT_URI".to_owned());
    annotate_masking(&mut schema, &map);
    mask_sampled_values(&mut schema);

    let email_ts = schema
        .object
        .get("email")
        .unwrap()
        .types
        .get("String")
        .unwrap();
    let values = email_ts
        .values
        .as_ref()
        .expect("values missing after masking");
    for v in values {
        let s = v.as_str().expect("value should be a string");
        assert!(
            !s.contains("alice") && !s.contains("bob"),
            "original name must not appear after masking, got: {s}"
        );
        assert!(s.contains('@'), "masked email must retain the @ separator");
    }
}

#[test]
fn test_mask_sampled_values_disabled_masking_unchanged() {
    let docs = vec![doc! { "_id": 1, "label": "hello" }];
    let mut schema = analyze_docs(&docs);

    // Inject a disabled masking rule manually
    if let Some(field) = schema.object.get_mut("label") {
        if let Some(ts) = field.types.get_mut("String") {
            ts.masking = Some(MaskingConfig {
                enabled: false,
                method: "PRESERVE_TOKEN".to_owned(),
            });
        }
    }
    mask_sampled_values(&mut schema);

    let ts = schema
        .object
        .get("label")
        .unwrap()
        .types
        .get("String")
        .unwrap();
    let v = ts.values.as_ref().unwrap().first().unwrap();
    assert_eq!(
        v.as_str().unwrap(),
        "hello",
        "disabled masking must leave value unchanged"
    );
}

// ──────────────────────────────────────────────────────────────────────────────
// Live-document masking — apply_masking_to_doc
// ──────────────────────────────────────────────────────────────────────────────

#[test]
fn test_apply_masking_to_doc_string_field() {
    let mut doc = doc! { "email": "alice@example.com" };
    let mut schema: IndexMap<String, FieldSchema> = IndexMap::new();
    schema.insert(
        "email".to_owned(),
        string_field_with_masking("MASK_CONTACT_URI"),
    );

    apply_masking_to_doc(&mut doc, &schema);

    let v = doc.get("email").unwrap();
    if let Bson::String(s) = v {
        assert_ne!(s, "alice@example.com", "email must be masked");
        assert!(s.contains('@'), "masked email must retain @");
    } else {
        panic!("expected Bson::String");
    }
}

#[test]
fn test_apply_masking_to_doc_integer_field() {
    let mut doc = doc! { "phone": 123456789_i32 };
    let ts = TypeSchema {
        masking: Some(MaskingConfig {
            enabled: true,
            method: "REDACT_ALPHANUMERIC".to_owned(),
        }),
        probability: 1.0,
        sampled: 1,
        as_jsonb: false,
        ndistinct: Some(1.0),
        object: None,
        array: None,
        values: None,
    };
    let mut types = IndexMap::new();
    types.insert("Number".to_owned(), ts);
    let mut schema: IndexMap<String, FieldSchema> = IndexMap::new();
    schema.insert(
        "phone".to_owned(),
        FieldSchema {
            probability: 1.0,
            types,
        },
    );

    apply_masking_to_doc(&mut doc, &schema);

    // REDACT_ALPHANUMERIC replaces digits with '9', parsed back to f64
    match doc.get("phone").unwrap() {
        Bson::Int32(n) => assert_ne!(*n, 123456789_i32),
        Bson::Double(n) => assert_ne!(*n, 123456789.0_f64),
        other => panic!("unexpected BSON type: {other:?}"),
    }
}

#[test]
fn test_apply_masking_to_doc_nested_object() {
    let mut doc = doc! {
        "host": {
            "host_name": "Alice",
            "host_location": "Paris"
        }
    };

    // Build a schema for the nested object
    let mut inner_schema: IndexMap<String, FieldSchema> = IndexMap::new();
    inner_schema.insert(
        "host_name".to_owned(),
        string_field_with_masking("PRESERVE_TOKEN"),
    );
    inner_schema.insert(
        "host_location".to_owned(),
        string_field_with_masking("GENERALIZE_LOCATION"),
    );

    let obj_ts = TypeSchema {
        masking: None,
        probability: 1.0,
        sampled: 1,
        as_jsonb: false,
        ndistinct: None,
        object: Some(inner_schema),
        array: None,
        values: None,
    };
    let mut host_types = IndexMap::new();
    host_types.insert("Object".to_owned(), obj_ts);
    let mut schema: IndexMap<String, FieldSchema> = IndexMap::new();
    schema.insert(
        "host".to_owned(),
        FieldSchema {
            probability: 1.0,
            types: host_types,
        },
    );

    apply_masking_to_doc(&mut doc, &schema);

    let host = doc
        .get_document("host")
        .expect("host must remain a document");
    let name = host.get_str("host_name").expect("host_name missing");
    let loc = host
        .get_str("host_location")
        .expect("host_location missing");
    assert_ne!(name, "Alice", "host_name must be masked");
    assert_ne!(loc, "Paris", "host_location must be masked");
}

#[test]
fn test_apply_masking_to_doc_unrecognised_field_untouched() {
    let mut doc = doc! { "score": 42_i32, "label": "hello" };
    // schema covers only "label"
    let mut schema: IndexMap<String, FieldSchema> = IndexMap::new();
    schema.insert(
        "label".to_owned(),
        string_field_with_masking("STATIC_BLOB_REPLACEMENT"),
    );

    apply_masking_to_doc(&mut doc, &schema);

    // score must be unchanged
    assert_eq!(doc.get_i32("score").unwrap(), 42);
    // label must be redacted
    assert_eq!(doc.get_str("label").unwrap(), "[REDACTED]");
}

// ──────────────────────────────────────────────────────────────────────────────
// Masking methods — unit tests
// ──────────────────────────────────────────────────────────────────────────────

#[test]
fn test_preserve_token_same_input_same_output() {
    let a = masking::mask_value("PRESERVE_TOKEN", "Alice");
    let b = masking::mask_value("PRESERVE_TOKEN", "Alice");
    assert_eq!(a, b, "PRESERVE_TOKEN must be deterministic");
}

#[test]
fn test_preserve_token_different_inputs_different_outputs() {
    let a = masking::mask_value("PRESERVE_TOKEN", "Alice");
    let b = masking::mask_value("PRESERVE_TOKEN", "Bob");
    assert_ne!(a, b);
}

#[test]
fn test_preserve_token_preserves_character_classes() {
    let output = masking::mask_value("PRESERVE_TOKEN", "Hello-World 123");
    let chars: Vec<char> = output.chars().collect();
    // positions 0-4: letters (Hello), 5: '-', 6-10: letters (World), 11: ' ', 12-14: digits
    for c in &chars[0..5] {
        assert!(c.is_alphabetic(), "expected letter, got '{c}'");
    }
    assert_eq!(chars[5], '-');
    for c in &chars[6..11] {
        assert!(c.is_alphabetic(), "expected letter, got '{c}'");
    }
    assert_eq!(chars[11], ' ');
    for c in &chars[12..15] {
        assert!(c.is_ascii_digit(), "expected digit, got '{c}'");
    }
}

#[test]
fn test_redact_alphanumeric_replaces_letters_and_digits() {
    let output = masking::mask_value("REDACT_ALPHANUMERIC", "AB-1234");
    assert_eq!(output, "XX-9999");
}

#[test]
fn test_redact_alphanumeric_keeps_separators() {
    let output = masking::mask_value("REDACT_ALPHANUMERIC", "foo bar");
    assert_eq!(output, "XXX XXX");
}

#[test]
fn test_mask_contact_uri_email_keeps_domain() {
    let output = masking::mask_value("MASK_CONTACT_URI", "alice@example.com");
    assert!(
        output.ends_with("@example.com"),
        "domain must be preserved, got: {output}"
    );
    assert!(!output.contains("alice"), "local part must be masked");
}

#[test]
fn test_mask_contact_uri_url_keeps_host() {
    let output = masking::mask_value(
        "MASK_CONTACT_URI",
        "https://cdn.example.com/images/photo.jpg",
    );
    assert!(
        output.starts_with("https://cdn.example.com"),
        "host must be preserved, got: {output}"
    );
    assert!(!output.contains("photo"), "path must be masked");
}

#[test]
fn test_mask_network_id_ipv4_zeroes_last_octet() {
    let output = masking::mask_value("MASK_NETWORK_ID", "192.168.1.42");
    assert_eq!(output, "192.168.1.0");
}

#[test]
fn test_mask_network_id_mac_zeroes_last_three_bytes() {
    let output = masking::mask_value("MASK_NETWORK_ID", "aa:bb:cc:11:22:33");
    assert_eq!(output, "aa:bb:cc:00:00:00");
}

#[test]
fn test_generalize_location_truncates_postal_code() {
    let output = masking::mask_value("GENERALIZE_LOCATION", "75013");
    // keeps first ceil(5/2) = 3 chars, zeroes the rest → "75000"
    assert_eq!(output, "75000");
}

#[test]
fn test_static_blob_replacement_always_redacted() {
    let output = masking::mask_value(
        "STATIC_BLOB_REPLACEMENT",
        "Great location near the Eiffel Tower!",
    );
    assert_eq!(output, "[REDACTED]");
}

#[test]
fn test_static_mapping_deterministic() {
    let a = masking::mask_value("STATIC_MAPPING", "premium");
    let b = masking::mask_value("STATIC_MAPPING", "premium");
    assert_eq!(a, b);
    // Output must be one of A–E
    assert!(
        ["A", "B", "C", "D", "E"].contains(&a.as_str()),
        "unexpected token: {a}"
    );
}

#[test]
fn test_noisy_date_changes_date() {
    let output = masking::mask_value("NOISY_DATE", "2024-06-15");
    // Must still look like a date
    assert!(
        output.contains('-'),
        "output must still be a date-like string, got: {output}"
    );
    // The year is preserved
    assert!(
        output.starts_with("2024"),
        "year must be preserved, got: {output}"
    );
}

#[test]
fn test_noisy_date_deterministic() {
    let a = masking::mask_value("NOISY_DATE", "2024-06-15");
    let b = masking::mask_value("NOISY_DATE", "2024-06-15");
    assert_eq!(a, b);
}

#[test]
fn test_unknown_method_returns_input_unchanged() {
    let output = masking::mask_value("NONEXISTENT_METHOD", "hello");
    assert_eq!(output, "hello");
}
