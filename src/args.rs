use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

#[derive(Args, Debug, Clone)]
pub struct UriArg {
    /// Manon connection URI (e.g. mongodb://localhost:27017) – required unless -c is given;
    /// overrides the URI stored in the config file when -c is also provided
    #[arg(long = "source-uri", short = 's')]
    pub source_uri: Option<String>,
}

// ──────────────────────────────────────────────────────────────────────────────
// CLI definition
// ──────────────────────────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(
    name = "manon",
    about = "Anonymize MongoDB collections",
    version,
    // Allow bare `manon <URI> <NS>` without an explicit subcommand.
    args_conflicts_with_subcommands = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Initialize a new anonymization project directory structure
    Init(InitArgs),
    /// Sample a MongoDB collection and infer its YAML Schema
    Infer(InferArgs),
    /// Re-apply masking rules to the `values` in an existing YAML schema file
    Mask(MaskArgs),
    /// Apply anonymization rules to a MongoDB collection
    Apply(ApplyArgs),
}

#[derive(Parser, Debug)]
pub struct InferArgs {
    #[command(flatten)]
    pub mongo: UriArg,

    /// Namespace: either <db>.<collection> to infer one collection, or just <db> to infer all
    /// collections in the database. When omitted (and -c is not given) all user databases on the
    /// server are enumerated and inferred (admin, local, and config are skipped). Can also be set
    /// via NAMESPACE in the config file.
    #[arg(long = "namespace")]
    pub namespace: Option<String>,

    /// Number of documents to sample (mutually exclusive with --percent); default 1000
    #[arg(short = 'n', long = "number", conflicts_with = "percent")]
    pub number: Option<u64>,

    /// Percentage of the collection to sample, e.g. 10 for 10% (mutually exclusive with --number)
    #[arg(short = 'p', long = "percent", conflicts_with = "number", value_parser = clap::value_parser!(f64))]
    pub percent: Option<f64>,

    /// Suppress schema output to stdout
    #[arg(long = "no-output", action = clap::ArgAction::SetTrue)]
    pub no_output: bool,

    /// Write <name>.json and <name>.stats.txt into <output_dir>/<name>/ for each collection
    #[arg(short = 'o', long = "output-dir", conflicts_with = "config")]
    pub output_dir: Option<PathBuf>,

    /// Path to a .conf file (created by `manon init`) to derive the output directory
    #[arg(short = 'c', long = "config", conflicts_with = "output_dir")]
    pub config: Option<PathBuf>,
}

#[derive(Parser, Debug)]
pub struct InitArgs {
    /// Base directory under which the project folder will be created
    #[arg(long)]
    pub project_cluster: PathBuf,

    /// Name of the project (becomes a sub-folder inside project_cluster)
    #[arg(long)]
    pub project_dbname: String,

    /// MongoDB connection URI to store in the project config
    #[arg(long)]
    pub source_uri: Option<String>,

    /// Namespace to store in the project config (e.g. mydb or mydb.mycoll); when omitted,
    /// NAMESPACE is not written to the config file so `infer` will enumerate all databases
    #[arg(long)]
    pub namespace: Option<String>,
}

#[derive(Parser, Debug)]
pub struct MaskArgs {
    /// Path to a YAML schema file produced by `manon infer`.
    /// The file is updated in-place: sampled `values` for every field
    /// that carries a `masking` block are replaced with their anonymised form.
    pub input: PathBuf,

    /// Write the result to this file instead of updating the input in-place
    #[arg(short = 'o', long = "output")]
    pub output: Option<PathBuf>,
}

#[derive(Parser, Debug)]
pub struct ApplyArgs {
    #[command(flatten)]
    pub mongo: UriArg,

    /// Path to a YAML schema file produced by `manon infer` (contains masking rules),
    /// or a directory of per-collection YAML files for a DB-level apply.
    /// When omitted and -c is given, defaults to <project>/source/collections/.
    #[arg(short = 'm', long = "masking-rules")]
    pub masking_rules: Option<PathBuf>,

    /// Source namespace to read from, in the form <db>.<collection>.
    /// Can be set via NAMESPACE in the config file.
    #[arg(short = 'n', long = "namespace")]
    pub namespace: Option<String>,

    /// Target MongoDB URI to write anonymised documents to
    #[arg(short = 't', long = "target-uri")]
    pub target_uri: String,

    /// Target namespace to write to (default: same as --namespace)
    #[arg(long = "target-namespace")]
    pub target_namespace: Option<String>,

    /// Export only this percentage of each source collection, e.g. 10 for 10%.
    /// Useful for ephemeral environments where a full copy is not needed.
    #[arg(short = 'p', long = "percent", value_parser = clap::value_parser!(f64))]
    pub percent: Option<f64>,

    /// Path to a .conf file (created by `manon init`) to supply source-uri and namespace
    #[arg(short = 'c', long = "config")]
    pub config: Option<PathBuf>,
}
