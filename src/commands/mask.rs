//! `manon mask` – re-apply masking rules to an existing YAML schema file.
//!
//! Reads a YAML file produced by `manon infer`, re-annotates every field
//! from the current identifier CSVs (so newly-added identifiers and the new
//! Array-field support are picked up automatically), then replaces sampled
//! `values` with their anonymised counterparts.  The result is written back
//! to the same file (or to `--output` when supplied).

use anyhow::{Context, Result};
use serde_yaml;

use crate::analyzer::{annotate_masking, mask_sampled_values};
use crate::args::MaskArgs;
use crate::commands::infer::field_method_map;
use crate::models::CollectionSchema;

pub fn run_mask(args: MaskArgs) -> Result<()> {
    let input = &args.input;

    let yaml_str = std::fs::read_to_string(input)
        .with_context(|| format!("Failed to read {}", input.display()))?;

    let mut schema: CollectionSchema = serde_yaml::from_str(&yaml_str)
        .with_context(|| format!("Failed to parse YAML from {}", input.display()))?;

    // Re-annotate from the current CSV definitions so any new identifiers or
    // newly-supported field types (e.g. Array) are picked up before masking.
    annotate_masking(&mut schema, field_method_map());

    mask_sampled_values(&mut schema);

    let output_yaml =
        serde_yaml::to_string(&schema).context("Failed to serialise schema to YAML")?;

    let dest = args.output.as_deref().unwrap_or(input);
    std::fs::write(dest, output_yaml)
        .with_context(|| format!("Failed to write {}", dest.display()))?;

    println!("Masked values written to {}", dest.display());
    Ok(())
}
