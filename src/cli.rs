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
    #[arg(value_name = "PATHS", help = "Files or directories to scan")]
    pub paths: Vec<String>,

    #[arg(short, long, help = "Load configuration from this file")]
    pub config: Option<PathBuf>,

    #[arg(long = "include", value_name = "GLOB", help = "Add an include glob")]
    pub include: Vec<String>,

    #[arg(long = "exclude", value_name = "GLOB", help = "Add an exclude glob")]
    pub exclude: Vec<String>,

    #[arg(short = 'r', long, help = "Scan nested GitHub Actions surfaces")]
    pub recursive: bool,

    #[arg(long = "threads", value_name = "N", help = "Override worker thread count")]
    pub threads: Option<usize>,

    #[arg(long, help = "Write a starter .gh-actions-updater.toml")]
    pub init: bool,

    #[arg(long = "force", help = "Overwrite existing files for --init")]
    pub force: bool,

    #[arg(long = "output", value_name = "PATH", help = "Output path for --init")]
    pub output: Option<PathBuf>,

    #[arg(long = "cache-dir", value_name = "PATH", help = "Override metadata cache directory")]
    pub cache_dir: Option<PathBuf>,

    #[arg(
        long = "cache-ttl",
        value_name = "DURATION",
        help = "Cache TTL such as 30m, 6h, 7d, 0, or never"
    )]
    pub cache_ttl: Option<String>,

    #[arg(long, help = "Refresh cache entries before using them")]
    pub refresh_cache: bool,

    #[arg(long, help = "Disable cache reads and writes")]
    pub no_cache: bool,

    #[arg(long, help = "Rewrite files with available updates")]
    pub update: bool,

    #[arg(long, help = "Pin selected update targets to commit SHAs")]
    pub latest_hash: bool,

    #[arg(long, value_enum, help = "Control how semver refs are formatted")]
    pub pin_style: Option<PinStyle>,

    #[arg(long, value_enum, help = "Policy for deleted or missing refs")]
    pub missing_ref: Option<MissingRefPolicy>,

    #[arg(long, help = "Exit non-zero when updates are available")]
    pub check: bool,

    #[arg(long, help = "Do not write files")]
    pub dry_run: bool,

    #[arg(long, help = "Print unified diffs for available rewrites")]
    pub diff: bool,

    #[arg(long, value_enum, help = "Output format")]
    pub format: Option<OutputFormat>,

    #[arg(short, long, help = "Suppress human report output")]
    pub quiet: bool,

    #[arg(short, long, help = "Print cache statistics")]
    pub verbose: bool,

    #[arg(long, value_enum, help = "When to use color in human output")]
    pub color: Option<ColorChoice>,

    #[arg(long, help = "GitHub token for metadata requests")]
    pub github_token: Option<String>,

    #[arg(long, help = "GitHub API base URL")]
    pub github_api_url: Option<String>,

    #[arg(long, help = "Fail on workflow/action schema diagnostics")]
    pub strict_schema: bool,

    #[arg(long, help = "Skip schema validation")]
    pub no_schema_validation: bool,

    #[arg(
        long,
        help = "Verify every reference exists upstream (tags, branches, SHAs) or on disk (local refs)"
    )]
    pub validate: bool,

    #[arg(
        long = "pin-floating-to-sha",
        help = "Rewrite branch and non-semver-tag references to the commit SHA they currently point at"
    )]
    pub pin_floating_to_sha: bool,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PinStyle {
    Preserve,
    Major,
    Minor,
    Full,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum UpdateMode {
    LatestTag,
    LatestHash,
}
