use clap::{Parser, ValueEnum};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(
    name = "gau",
    version,
    about = "Find and update GitHub Actions references",
    long_about = "Scan GitHub Actions workflow and action metadata files for remote action and reusable workflow references."
)]
pub struct Cli {
    #[arg(value_name = "PATHS")]
    pub paths: Vec<String>,

    #[arg(short, long)]
    pub config: Option<PathBuf>,

    #[arg(long = "include", value_name = "GLOB")]
    pub include: Vec<String>,

    #[arg(long = "exclude", value_name = "GLOB")]
    pub exclude: Vec<String>,

    #[arg(short = 'r', long)]
    pub recursive: bool,

    #[arg(long = "threads", value_name = "N")]
    pub threads: Option<usize>,

    #[arg(long)]
    pub init: bool,

    #[arg(long = "force")]
    pub force: bool,

    #[arg(long = "output", value_name = "PATH")]
    pub output: Option<PathBuf>,

    #[arg(long = "cache-dir", value_name = "PATH")]
    pub cache_dir: Option<PathBuf>,

    #[arg(long = "cache-ttl", value_name = "DURATION")]
    pub cache_ttl: Option<String>,

    #[arg(long)]
    pub refresh_cache: bool,

    #[arg(long)]
    pub no_cache: bool,

    #[arg(long)]
    pub update: bool,

    #[arg(long)]
    pub latest_hash: bool,

    #[arg(long, value_enum)]
    pub missing_ref: Option<MissingRefPolicy>,

    #[arg(long)]
    pub check: bool,

    #[arg(long)]
    pub dry_run: bool,

    #[arg(long)]
    pub diff: bool,

    #[arg(long, value_enum)]
    pub format: Option<OutputFormat>,

    #[arg(short, long)]
    pub quiet: bool,

    #[arg(short, long)]
    pub verbose: bool,

    #[arg(long, value_enum)]
    pub color: Option<ColorChoice>,

    #[arg(long)]
    pub github_token: Option<String>,

    #[arg(long)]
    pub github_api_url: Option<String>,

    #[arg(long)]
    pub strict_schema: bool,

    #[arg(long)]
    pub no_schema_validation: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum OutputFormat {
    Human,
    Json,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum ColorChoice {
    Auto,
    Always,
    Never,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum MissingRefPolicy {
    Warn,
    Error,
    Ignore,
    Fallback,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum UpdateMode {
    LatestTag,
    LatestHash,
}
