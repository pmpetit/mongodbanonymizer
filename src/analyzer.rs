//! Probabilistic schema inference from BSON/MongoDB documents.
//!
//! # Design
//! * Every BSON value is mapped to an internal *type name* string (see [`bson_type_name`]).
//! * Per-field type-distribution counters are accumulated via [`Analyzer`].
//! * After processing all documents [`Analyzer::finish`] computes probabilities,
//!   injects implicit `Undefined` entries, and sorts fields (`_id` first, then
//!   case-insensitive alphabetical order).
//! * Values are kept via reservoir sampling; the reservoir capacity is 100 for
//!   `String`, `Binary`, `JavaScriptCode`, `JavaScriptCodeWithScope`, and 10 000
//!   for all other types.

use std::collections::{HashMap, HashSet};

use crate::masking;
use crate::models::{CollectionSchema, FieldSchema, MaskingConfig, TypeSchema};
use indexmap::IndexMap;
use mongodb::bson::Bson;
use rand::RngExt;
use rand::rngs::ThreadRng;

// ──────────────────────────────────────────────────────────────────────────────
// Internal type-name constants
// ──────────────────────────────────────────────────────────────────────────────

pub const TYPE_NUMBER: &str = "Number";
pub const TYPE_STRING: &str = "String";
pub const TYPE_BOOLEAN: &str = "Boolean";
pub const TYPE_DATE: &str = "Date";
pub const TYPE_OBJECTID: &str = "ObjectId";
pub const TYPE_NULL: &str = "Null";
pub const TYPE_BINARY: &str = "Binary";
pub const TYPE_ARRAY: &str = "Array";
pub const TYPE_OBJECT: &str = "Object";
pub const TYPE_DECIMAL128: &str = "Decimal128";
pub const TYPE_REGEX: &str = "RegularExpression";
pub const TYPE_CODE: &str = "JavaScriptCode";
pub const TYPE_CODE_W_SCOPE: &str = "JavaScriptCodeWithScope";
pub const TYPE_TIMESTAMP: &str = "Timestamp";
pub const TYPE_SYMBOL: &str = "Symbol";
pub const TYPE_DBPOINTER: &str = "DbPointer";
pub const TYPE_MAXKEY: &str = "MaxKey";
pub const TYPE_MINKEY: &str = "MinKey";
pub const TYPE_UNDEFINED: &str = "Undefined";

// ──────────────────────────────────────────────────────────────────────────────
// Internal accumulation helpers (not serialized)
// ──────────────────────────────────────────────────────────────────────────────

struct ValueReservoir {
    reservoir: Vec<serde_json::Value>,
    capacity: usize,
    seen: u64,
    rng: ThreadRng,
}

impl ValueReservoir {
    fn new(capacity: usize) -> Self {
        Self {
            reservoir: Vec::with_capacity(capacity.min(64)),
            capacity,
            seen: 0,
            rng: rand::rng(),
        }
    }

    fn add(&mut self, value: serde_json::Value) {
        self.seen += 1;
        if self.reservoir.len() < self.capacity {
            self.reservoir.push(value);
        } else {
            let idx = self.rng.random_range(0..self.seen) as usize;
            if idx < self.capacity {
                self.reservoir[idx] = value;
            }
        }
    }

    fn into_values(self) -> Vec<serde_json::Value> {
        self.reservoir
    }
}

/// Maximum number of distinct values tracked per type before we stop counting.
const DISTINCT_CAP: usize = 1_000;

struct TypeAcc {
    count: u64,
    nested_object: Option<ObjectAcc>,
    array_items: Option<Box<FieldAcc>>,
    values: Option<ValueReservoir>,
    /// Distinct serialized values seen for scalar types (capped at [`DISTINCT_CAP`]).
    distinct_values: HashSet<String>,
    /// First 20 distinct values in order of first appearance (scalar types only).
    first_distinct_values: Vec<serde_json::Value>,
}

impl TypeAcc {
    fn new(type_name: &str, collect_values: bool) -> Self {
        let values = if collect_values {
            let cap = reservoir_capacity(type_name);
            Some(ValueReservoir::new(cap))
        } else {
            None
        };
        Self {
            count: 0,
            nested_object: None,
            array_items: None,
            values,
            distinct_values: HashSet::new(),
            first_distinct_values: Vec::new(),
        }
    }
}

struct FieldAcc {
    count: u64,
    types: HashMap<String, TypeAcc>,
    collect_values: bool,
}

impl FieldAcc {
    fn new(collect_values: bool) -> Self {
        Self {
            count: 0,
            types: HashMap::new(),
            collect_values,
        }
    }

    fn observe_value(&mut self, bson: &Bson) {
        self.count += 1;
        let type_name = bson_type_name(bson);
        let acc = self
            .types
            .entry(type_name.to_owned())
            .or_insert_with(|| TypeAcc::new(type_name, self.collect_values));
        acc.count += 1;

        // Collect sample value and track distinct values for scalar types.
        if let Some(v) = bson_to_json_value(bson) {
            if let Some(reservoir) = acc.values.as_mut() {
                reservoir.add(v.clone());
            }
            // Track distinct values for non-Object, non-Array types.
            if acc.distinct_values.len() < DISTINCT_CAP {
                if let Ok(s) = serde_json::to_string(&v) {
                    let is_new = acc.distinct_values.insert(s);
                    if is_new && acc.first_distinct_values.len() < 20 {
                        acc.first_distinct_values.push(v);
                    }
                }
            }
        }

        match bson {
            Bson::Document(doc) => {
                let nested = acc.nested_object.get_or_insert_with(ObjectAcc::new);
                for (k, v) in doc {
                    nested.observe_field(k, v, self.collect_values);
                }
            }
            Bson::Array(arr) => {
                let items = acc
                    .array_items
                    .get_or_insert_with(|| Box::new(FieldAcc::new(self.collect_values)));
                for v in arr {
                    items.observe_value(v);
                }
            }
            _ => {}
        }
    }
}

struct ObjectAcc {
    fields: HashMap<String, FieldAcc>,
}

impl ObjectAcc {
    fn new() -> Self {
        Self {
            fields: HashMap::new(),
        }
    }

    fn observe_field(&mut self, key: &str, value: &Bson, collect_values: bool) {
        let acc = self
            .fields
            .entry(key.to_owned())
            .or_insert_with(|| FieldAcc::new(collect_values));
        acc.observe_value(value);
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Public Analyzer
// ──────────────────────────────────────────────────────────────────────────────

/// Accumulates BSON documents and produces a [`CollectionSchema`].
pub struct Analyzer {
    total_docs: u64,
    root: ObjectAcc,
    collect_values: bool,
}

impl Analyzer {
    /// Create a new analyzer.
    ///
    /// * `collect_values` – whether to reservoir-sample field values.
    pub fn new(collect_values: bool) -> Self {
        Self {
            total_docs: 0,
            root: ObjectAcc::new(),
            collect_values,
        }
    }

    /// Feed one BSON document into the analyzer.
    pub fn process_document(&mut self, doc: &mongodb::bson::Document) {
        self.total_docs += 1;
        for (key, value) in doc {
            self.root.observe_field(key, value, self.collect_values);
        }
    }

    /// Finalize and return the inferred [`CollectionSchema`].
    pub fn finish(self) -> CollectionSchema {
        let total = self.total_docs;
        let object = build_field_map(self.root, total);
        CollectionSchema {
            count: total,
            sampled: total,
            object,
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Schema-building helpers
// ──────────────────────────────────────────────────────────────────────────────

fn build_field_map(acc: ObjectAcc, total_docs: u64) -> IndexMap<String, FieldSchema> {
    let mut entries: Vec<(String, FieldSchema)> = acc
        .fields
        .into_iter()
        .map(|(name, fa)| {
            let schema = build_field_schema(fa, total_docs);
            (name, schema)
        })
        .collect();

    // Sort: _id first, then case-insensitive alphabetical
    entries.sort_by(|(a, _), (b, _)| {
        if a == "_id" {
            std::cmp::Ordering::Less
        } else if b == "_id" {
            std::cmp::Ordering::Greater
        } else {
            a.to_lowercase().cmp(&b.to_lowercase())
        }
    });

    let mut map = IndexMap::with_capacity(entries.len());
    for (k, v) in entries {
        map.insert(k, v);
    }
    map
}

fn build_field_schema(fa: FieldAcc, total_docs: u64) -> FieldSchema {
    let field_count = fa.count;
    let probability = if total_docs > 0 {
        field_count as f64 / total_docs as f64
    } else {
        0.0
    };

    let mut type_entries: Vec<(String, TypeSchema)> = fa
        .types
        .into_iter()
        .map(|(tname, ta)| {
            let schema = build_type_schema(ta, field_count, total_docs);
            (tname, schema)
        })
        .collect();

    // Add implicit Undefined if field was missing from some docs
    let undefined_count = total_docs.saturating_sub(field_count);
    if undefined_count > 0 {
        let undef_schema = TypeSchema {
            masking: None,
            probability: undefined_count as f64 / total_docs as f64,
            sampled: undefined_count,
            as_jsonb: false,
            ndistinct: None,
            object: None,
            array: None,
            values: None,
        };
        type_entries.push((TYPE_UNDEFINED.to_owned(), undef_schema));
    }

    // Sort type entries by descending count for determinism
    type_entries.sort_by(|(an, _), (bn, _)| {
        if an == TYPE_UNDEFINED {
            std::cmp::Ordering::Greater
        } else if bn == TYPE_UNDEFINED {
            std::cmp::Ordering::Less
        } else {
            an.cmp(bn)
        }
    });

    let mut types = IndexMap::with_capacity(type_entries.len());
    for (k, v) in type_entries {
        types.insert(k, v);
    }

    FieldSchema { probability, types }
}

fn build_type_schema(ta: TypeAcc, field_count: u64, total_docs: u64) -> TypeSchema {
    let probability = if field_count > 0 {
        ta.count as f64 / field_count as f64
    } else {
        0.0
    };

    // Compute ndistinct:
    // - Array:  average number of elements per document (avg cardinality).
    // - Object: None (ndistinct is meaningless for sub-documents).
    // - Scalar: number of distinct values observed (capped at DISTINCT_CAP).
    let has_object = ta.nested_object.is_some();
    let has_array = ta.array_items.is_some();

    let object = ta
        .nested_object
        .map(|nested| build_field_map(nested, ta.count));

    let ndistinct = if has_array {
        ta.array_items.as_ref().map(|items_fa| {
            if total_docs > 0 {
                items_fa.count as f64 / total_docs as f64
            } else {
                0.0
            }
        })
    } else if has_object {
        None
    } else {
        Some(ta.distinct_values.len() as f64)
    };

    // For the items FieldSchema, use the total number of items as the denominator
    // so that items.probability = fraction of items with each type (always ≤ 1.0).
    // Average cardinality is already captured in the parent Array TypeSchema's ndistinct.
    let array = ta.array_items.map(|items_fa| {
        let total_items = items_fa.count;
        Box::new(build_field_schema(*items_fa, total_items))
    });

    // For scalar types, output the first 20 distinct values in order of first appearance.
    // For Object/Array types (where bson_to_json_value returns None), fall back to
    // the reservoir samples (truncated to 20).
    let values = if !ta.first_distinct_values.is_empty() {
        Some(ta.first_distinct_values).filter(|v| !v.is_empty())
    } else {
        ta.values
            .map(|r| {
                let mut v = r.into_values();
                v.truncate(20);
                v
            })
            .filter(|v| !v.is_empty())
    };

    TypeSchema {
        masking: None,
        probability,
        sampled: ta.count,
        as_jsonb: false,
        ndistinct,
        object,
        array,
        values,
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Masking annotation
// ──────────────────────────────────────────────────────────────────────────────

/// Walk a finished [`CollectionSchema`] and attach masking rules to String-typed
/// fields whose names appear in `field_to_method` (by exact match or `_`-suffix).
pub fn annotate_masking(schema: &mut CollectionSchema, field_to_method: &HashMap<String, String>) {
    annotate_fields(&mut schema.object, field_to_method);
}

/// Walk a finished, already-annotated [`CollectionSchema`] and replace each
/// sampled string value with its masked counterpart.
///
/// Must be called **after** [`annotate_masking`].
pub fn mask_sampled_values(schema: &mut CollectionSchema) {
    mask_values_in_fields(&mut schema.object);
}

fn mask_values_in_fields(fields: &mut IndexMap<String, FieldSchema>) {
    for field_schema in fields.values_mut() {
        for type_schema in field_schema.types.values_mut() {
            // ── String (or any scalar with a masking block) ──────────────────
            if let Some(masking_cfg) = &type_schema.masking {
                if masking_cfg.enabled {
                    let method = masking_cfg.method.clone();
                    if let Some(values) = type_schema.values.as_mut() {
                        for v in values.iter_mut() {
                            mask_json_value(&method, v);
                        }
                    }
                }
            }

            // ── Nested Object ────────────────────────────────────────────────
            // First recurse so every leaf is masked, then fix the compound
            // document samples stored at this Object TypeSchema level.
            if let Some(obj) = type_schema.object.as_mut() {
                mask_values_in_fields(obj);
            }
            if type_schema.object.is_some() {
                mask_object_values(
                    type_schema.object.as_ref().unwrap(),
                    type_schema.values.as_mut(),
                );
            }

            // ── Array ────────────────────────────────────────────────────────
            // Collect the masking context from inner types *before* taking a
            // mutable borrow (borrow checker: array and values are separate
            // fields, but we need the inner data first).
            let inner_scalar_method: Option<String> = type_schema.array.as_ref().and_then(|arr| {
                arr.types.values().find_map(|ts| {
                    ts.masking
                        .as_ref()
                        .filter(|m| m.enabled)
                        .map(|m| m.method.clone())
                })
            });
            // Field→method map when the array contains embedded objects.
            let inner_obj_methods: Option<HashMap<String, String>> =
                type_schema.array.as_ref().and_then(|arr| {
                    arr.types
                        .values()
                        .find(|ts| ts.object.is_some())
                        .map(|ts| collect_field_methods(ts.object.as_ref().unwrap()))
                });

            if let Some(arr) = type_schema.array.as_mut() {
                for arr_type in arr.types.values_mut() {
                    // Mask scalar flat values on this inner type.
                    if let Some(masking_cfg) = &arr_type.masking {
                        if masking_cfg.enabled {
                            let method = masking_cfg.method.clone();
                            if let Some(arr_values) = arr_type.values.as_mut() {
                                for v in arr_values.iter_mut() {
                                    mask_json_value(&method, v);
                                }
                            }
                        }
                    }
                    // Recurse into object fields inside array items, then mask
                    // the compound document samples stored here.
                    if let Some(obj) = arr_type.object.as_mut() {
                        mask_values_in_fields(obj);
                    }
                    if arr_type.object.is_some() {
                        mask_object_values(
                            arr_type.object.as_ref().unwrap(),
                            arr_type.values.as_mut(),
                        );
                    }
                }
            }

            // Mask the outer Array-level `values`:
            // • scalars (e.g. coordinate pairs) → use the inner scalar method.
            // • object arrays (e.g. reviews)    → mask per field using inner_obj_methods.
            if let Some(outer_values) = type_schema.values.as_mut() {
                if let Some(method) = &inner_scalar_method {
                    for v in outer_values.iter_mut() {
                        mask_json_value(method, v);
                    }
                } else if let Some(ref field_methods) = inner_obj_methods {
                    if !field_methods.is_empty() {
                        for outer_val in outer_values.iter_mut() {
                            // Each outer_val is an array of embedded documents.
                            if let serde_json::Value::Array(docs) = outer_val {
                                for doc in docs.iter_mut() {
                                    if let serde_json::Value::Object(map) = doc {
                                        for (key, val) in map.iter_mut() {
                                            if let Some(method) = field_methods.get(key) {
                                                mask_json_value(method, val);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Apply the per-field masking methods collected from `fields` to every
/// compound document in `values` (Object-level sample list).
fn mask_object_values(
    fields: &IndexMap<String, FieldSchema>,
    values: Option<&mut Vec<serde_json::Value>>,
) {
    let field_methods = collect_field_methods(fields);
    if field_methods.is_empty() {
        return;
    }
    if let Some(values) = values {
        for doc in values.iter_mut() {
            if let serde_json::Value::Object(map) = doc {
                for (key, val) in map.iter_mut() {
                    if let Some(method) = field_methods.get(key) {
                        mask_json_value(method, val);
                    }
                }
            }
        }
    }
}

/// Apply `method` masking to a single `serde_json::Value` in-place.
///
/// * `String`  → run the masking function and replace the string.
/// * `Number`  → stringify, mask, parse back as f64 (for NOISY_POSITION etc.).
/// * `Array`   → recurse into every element (handles e.g. coordinate pairs).
/// * Anything else → leave unchanged.
fn mask_json_value(method: &str, v: &mut serde_json::Value) {
    match v {
        serde_json::Value::String(s) => {
            *s = masking::mask_value(method, s);
        }
        serde_json::Value::Number(_) => {
            let s = v.to_string();
            let masked = masking::mask_value(method, &s);
            if let Ok(n) = masked.parse::<f64>() {
                if let Some(jn) = serde_json::Number::from_f64(n) {
                    *v = serde_json::Value::Number(jn);
                }
            }
        }
        serde_json::Value::Array(arr) => {
            for elem in arr.iter_mut() {
                mask_json_value(method, elem);
            }
        }
        _ => {}
    }
}

/// Build a `field_name → masking_method` map from a set of field schemas.
///
/// Used to mask compound Object-level `values` entries (embedded documents)
/// after the individual per-field values have already been masked.
fn collect_field_methods(fields: &IndexMap<String, FieldSchema>) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for (field_name, field_schema) in fields {
        for type_schema in field_schema.types.values() {
            if let Some(masking) = &type_schema.masking {
                if masking.enabled {
                    map.insert(field_name.clone(), masking.method.clone());
                    break;
                }
            }
        }
    }
    map
}

fn lookup_method(field_name: &str, map: &HashMap<String, String>) -> Option<String> {
    if let Some(m) = map.get(field_name) {
        return Some(m.clone());
    }
    // Suffix match: `host_name` → `name`, `listing_url` → `url`, etc.
    if let Some(pos) = field_name.rfind('_') {
        let suffix = &field_name[pos + 1..];
        if let Some(m) = map.get(suffix) {
            return Some(m.clone());
        }
    }
    None
}

fn annotate_fields(
    fields: &mut IndexMap<String, FieldSchema>,
    field_to_method: &HashMap<String, String>,
) {
    for (field_name, field_schema) in fields.iter_mut() {
        let method = lookup_method(field_name, field_to_method);
        for (type_name, type_schema) in field_schema.types.iter_mut() {
            if let Some(ref m) = method {
                if type_name == TYPE_STRING {
                    // String fields: annotate the TypeSchema directly.
                    type_schema.masking = Some(MaskingConfig {
                        enabled: true,
                        method: m.clone(),
                    });
                } else if type_name == TYPE_ARRAY {
                    // Array fields: annotate the *item* types (Number, String)
                    // inside the array, not the Array TypeSchema itself.
                    // Clear any previously-set masking on the Array TypeSchema.
                    type_schema.masking = None;
                    if let Some(arr) = type_schema.array.as_mut() {
                        for (inner_type_name, inner_type_schema) in arr.types.iter_mut() {
                            if inner_type_name == TYPE_NUMBER || inner_type_name == TYPE_STRING {
                                inner_type_schema.masking = Some(MaskingConfig {
                                    enabled: true,
                                    method: m.clone(),
                                });
                            }
                        }
                    }
                }
            }
            // Recurse into nested objects
            if let Some(obj) = type_schema.object.as_mut() {
                annotate_fields(obj, field_to_method);
            }
            // Recurse into array item objects (arrays of embedded documents)
            if let Some(arr) = type_schema.array.as_mut() {
                for (_, arr_type) in arr.types.iter_mut() {
                    if let Some(obj) = arr_type.object.as_mut() {
                        annotate_fields(obj, field_to_method);
                    }
                }
            }
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// JSONB marking
// ──────────────────────────────────────────────────────────────────────────────

impl CollectionSchema {
    /// Mark every `Object`-typed field in the schema with `as_jsonb = true`.
    /// `to-pg` will then emit a `JSONB` column instead of a child table.
    /// Array-of-objects fields are left untouched (they still become child tables),
    /// but any Object sub-fields *inside* array items are also marked.
    pub fn mark_objects_as_jsonb(&mut self) {
        for field in self.object.values_mut() {
            mark_field_as_jsonb(field);
        }
    }
}

fn mark_field_as_jsonb(field: &mut FieldSchema) {
    for (type_name, type_schema) in field.types.iter_mut() {
        if type_name == TYPE_OBJECT {
            type_schema.as_jsonb = true;
            // No need to recurse: once the field is JSONB the sub-schema
            // is ignored by to-pg.
        } else if type_name == TYPE_ARRAY {
            // Recurse into array items so any Object sub-fields within
            // array rows are also marked.
            if let Some(items) = type_schema.array.as_mut() {
                mark_field_as_jsonb(items);
            }
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// BSON helpers
// ──────────────────────────────────────────────────────────────────────────────

/// Map a BSON value to an internal type-name string.
///
/// Int32, Int64, Double, and Float all map to `"Number"`.
/// `Decimal128` maps to `"Decimal128"` (distinct from `Number`).
pub fn bson_type_name(bson: &Bson) -> &'static str {
    match bson {
        Bson::Double(_) | Bson::Int32(_) | Bson::Int64(_) => TYPE_NUMBER,
        Bson::String(_) => TYPE_STRING,
        Bson::Document(_) => TYPE_OBJECT,
        Bson::Array(_) => TYPE_ARRAY,
        Bson::Binary(_) => TYPE_BINARY,
        Bson::ObjectId(_) => TYPE_OBJECTID,
        Bson::Boolean(_) => TYPE_BOOLEAN,
        Bson::DateTime(_) => TYPE_DATE,
        Bson::Null => TYPE_NULL,
        Bson::RegularExpression(_) => TYPE_REGEX,
        Bson::JavaScriptCode(_) => TYPE_CODE,
        Bson::JavaScriptCodeWithScope(_) => TYPE_CODE_W_SCOPE,
        Bson::Symbol(_) => TYPE_SYMBOL,
        Bson::Timestamp(_) => TYPE_TIMESTAMP,
        Bson::Decimal128(_) => TYPE_DECIMAL128,
        Bson::MaxKey => TYPE_MAXKEY,
        Bson::MinKey => TYPE_MINKEY,
        Bson::DbPointer(_) => TYPE_DBPOINTER,
        Bson::Undefined => TYPE_UNDEFINED,
    }
}

/// Reservoir capacity for a given internal type name.
fn reservoir_capacity(type_name: &str) -> usize {
    match type_name {
        TYPE_STRING | TYPE_BINARY | TYPE_CODE | TYPE_CODE_W_SCOPE => 100,
        _ => 10_000,
    }
}

/// Convert a BSON value to a JSON-compatible value for sample storage.
/// Returns `None` for values that cannot be meaningfully represented.
pub fn bson_to_json_value(bson: &Bson) -> Option<serde_json::Value> {
    match bson {
        Bson::Double(v) => Some(serde_json::json!(v)),
        Bson::Int32(v) => Some(serde_json::json!(v)),
        Bson::Int64(v) => Some(serde_json::json!(v)),
        Bson::String(s) => Some(serde_json::Value::String(s.clone())),
        Bson::Boolean(b) => Some(serde_json::Value::Bool(*b)),
        Bson::Null => Some(serde_json::Value::Null),
        Bson::ObjectId(oid) => Some(serde_json::Value::String(oid.to_hex())),
        Bson::DateTime(dt) => Some(serde_json::Value::String(dt.to_string())),
        Bson::Decimal128(d) => Some(serde_json::Value::String(d.to_string())),
        Bson::Binary(b) => {
            // Represent binary as hex string for sampling purposes
            let hex: String = b.bytes.iter().map(|byte| format!("{byte:02x}")).collect();
            Some(serde_json::Value::String(hex))
        }
        Bson::Document(doc) => {
            let mut map = serde_json::Map::new();
            for (k, v) in doc {
                if let Some(jv) = bson_to_json_value(v) {
                    map.insert(k.clone(), jv);
                }
            }
            Some(serde_json::Value::Object(map))
        }
        Bson::Array(arr) => {
            let vals: Vec<serde_json::Value> = arr.iter().filter_map(bson_to_json_value).collect();
            Some(serde_json::Value::Array(vals))
        }
        _ => None,
    }
}
