use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};

use crate::args::InitArgs;
use crate::models::ConfData;

pub fn run_init(args: InitArgs) -> Result<()> {
    let project_root = args.project_cluster.join(&args.project_dbname);

    let dirs = [
        project_root.join("source").join("collections"),
        project_root.join("config"),
    ];

    for dir in &dirs {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("Failed to create directory {}", dir.display()))?;
    }

    let conf_path = project_root
        .join("config")
        .join(format!("{}.conf", args.project_dbname));
    let conf_content = format!(
        "BASE_DIR = {}\nPROJECT_DIR = {}\n{}{}\n# NUMBER = 1000\n# PERCENT = 10\n",
        args.project_cluster.display(),
        args.project_dbname,
        args.source_uri
            .as_deref()
            .map(|u| format!("URI = {}\n", u))
            .unwrap_or_else(|| "# URI = mongodb://localhost:27017\n".to_owned()),
        args.namespace
            .as_deref()
            .map(|ns| format!("NAMESPACE = {}\n", ns))
            .unwrap_or_else(|| "# NAMESPACE = mydb\n".to_owned()),
    );
    std::fs::write(&conf_path, conf_content)
        .with_context(|| format!("Failed to write {}", conf_path.display()))?;

    println!(
        "Project '{}' initialised at {}",
        args.project_dbname,
        project_root.display()
    );
    for dir in &dirs {
        println!("  {}", dir.display());
    }
    println!("  {}", conf_path.display());
    Ok(())
}

/// Parse a `.conf` file produced by `manon init`.
pub fn read_conf(path: &Path) -> Result<ConfData> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read config file {}", path.display()))?;

    let mut base_dir: Option<PathBuf> = None;
    let mut project_dir: Option<String> = None;
    let mut uri: Option<String> = None;
    let mut namespace: Option<String> = None;
    let mut number: Option<u64> = None;
    let mut percent: Option<f64> = None;

    for line in content.lines() {
        if let Some((key, val)) = line.split_once('=') {
            match key.trim() {
                "BASE_DIR" => base_dir = Some(PathBuf::from(val.trim())),
                "PROJECT_DIR" => project_dir = Some(val.trim().to_owned()),
                "URI" => uri = Some(val.trim().to_owned()),
                "NAMESPACE" => namespace = Some(val.trim().to_owned()),
                "NUMBER" => number = val.trim().parse().ok(),
                "PERCENT" => percent = val.trim().parse().ok(),
                _ => {}
            }
        }
    }

    let base_dir = base_dir.ok_or_else(|| anyhow!("BASE_DIR not found in {}", path.display()))?;
    let project_dir =
        project_dir.ok_or_else(|| anyhow!("PROJECT_DIR not found in {}", path.display()))?;

    Ok(ConfData {
        base_dir,
        project_dir,
        uri,
        namespace,
        number,
        percent,
    })
}
