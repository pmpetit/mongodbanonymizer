//! `manon` CLI – anonymize MongoDB collections.

use anyhow::Result;
use clap::Parser;

use mongodbanonymizer::args::{Cli, Command};
use mongodbanonymizer::commands::{
    apply::run_apply, infer::run_infer, init::run_init, mask::run_mask,
};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Init(args) => run_init(args),
        Command::Infer(args) => run_infer(args).await,
        Command::Mask(args) => run_mask(args),
        Command::Apply(args) => run_apply(args).await,
    }
}
