use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Values parsed from a `.conf` file produced by `manon init`.
pub struct ConfData {
    pub base_dir: PathBuf,
    pub project_dir: String,
    pub uri: Option<String>,
    pub namespace: Option<String>,
    pub number: Option<u64>,
    pub percent: Option<f64>,
}

fn is_false(b: &bool) -> bool {
    !b
}

/// Top-level schema for a sampled collection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionSchema {
    /// Total number of documents in the collection.
    pub count: u64,
    /// Number of documents actually sampled and analysed.
    pub sampled: u64,
    /// Field schemas, ordered: `_id` first then case-insensitive alphabetical.
    pub object: IndexMap<String, FieldSchema>,
}

/// Schema for a single field (across all observed documents).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldSchema {
    /// `count / total_docs` – probability the field is present.
    pub probability: f64,
    /// Type distribution for this field.
    pub types: IndexMap<String, TypeSchema>,
}

/// Masking configuration attached to a field type during schema inference.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaskingConfig {
    pub enabled: bool,
    pub method: String,
}

/// Schema for one BSON type within a field.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypeSchema {
    /// Masking rule derived from identifier CSVs (present when field is recognized).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub masking: Option<MaskingConfig>,
    /// `count / field.count` – probability of this type given the field exists.
    pub probability: f64,
    /// Number of documents/items in which this type was observed (denominator for nested probabilities).
    pub sampled: u64,
    /// When `true`, `to-pg` emits a `JSONB` column for this Object field
    /// instead of creating a separate child table. Set by `infer --jsonb`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub as_jsonb: bool,
    /// Average number of array elements per document (only set when `type_name == "Array"`).
    /// Equivalent to PostgreSQL `n_distinct` when positive: an absolute average cardinality.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ndistinct: Option<f64>,
    /// Sub-document schema (present when `type_name == "Object"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object: Option<IndexMap<String, FieldSchema>>,
    /// Array-items schema (present when `type_name == "Array"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub array: Option<Box<FieldSchema>>,
    /// Reservoir-sampled values (present when value collection is enabled).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub values: Option<Vec<serde_json::Value>>,
}
