use crate::cli::Cli;
use anyhow::{Context, Result};
use std::path::PathBuf;

const DEFAULT_CONFIG: &str = r#"[scan]
include = [
  ".github/workflows/**/*.yml",
  ".github/workflows/**/*.yaml",
  ".github/actions/**/action.yml",
  ".github/actions/**/action.yaml",
  "action.yml",
  "action.yaml",
]
exclude = []
recursive = false

[cache]
enabled = true
ttl = "6h"

[update]
mode = "latest-tag"
pin_style = "preserve"
exclude = []
include_prereleases = false
preserve_major = true
missing_ref = "warn"

[output]
format = "human"
color = "auto"

[github]
api_url = "https://api.github.com"

[performance]
# Omit threads to use Rayon\'s available-core default.
# threads = 8
"#;

pub fn run(cli: &Cli) -> Result<()> {
    let path = cli
        .output
        .clone()
        .unwrap_or_else(|| PathBuf::from(".gh-actions-updater.toml"));
    if path.exists() && !cli.force {
        anyhow::bail!("{} already exists; pass --force to overwrite it", path.display());
    }

    if let Some(parent) = path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;
    }

    let mut config = DEFAULT_CONFIG.to_string();
    if cli.recursive {
        config = config.replace("recursive = false", "recursive = true");
    }
    std::fs::write(&path, config).with_context(|| format!("failed to write {}", path.display()))?;
    println!("Wrote {}", path.display());
    Ok(())
}
