use crate::cli::{Cli, ColorChoice, MissingRefPolicy, OutputFormat, UpdateMode};
use anyhow::{Context, Result};
use serde::Deserialize;
use std::env;
use std::path::{Path, PathBuf};

pub const DEFAULT_INCLUDES: &[&str] = &[
    ".github/workflows/**/*.yml",
    ".github/workflows/**/*.yaml",
    ".github/actions/**/action.yml",
    ".github/actions/**/action.yaml",
    "action.yml",
    "action.yaml",
];

#[derive(Debug, Clone)]
pub struct Settings {
    pub paths: Vec<String>,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub cache_dir: PathBuf,
    pub cache_ttl: CacheTtl,
    pub cache_enabled: bool,
    pub refresh_cache: bool,
    pub update: bool,
    pub latest_hash: bool,
    pub missing_ref: MissingRefPolicy,
    pub include_prereleases: bool,
    pub preserve_major: bool,
    pub check: bool,
    pub dry_run: bool,
    pub diff: bool,
    pub format: OutputFormat,
    pub quiet: bool,
    pub verbose: bool,
    pub color: ColorChoice,
    pub github_token_present: bool,
    pub github_token: Option<String>,
    pub github_api_url: String,
    pub strict_schema: bool,
    pub schema_validation: bool,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum CacheTtl {
    Seconds(u64),
    Never,
}

#[derive(Debug, Deserialize, Default)]
struct FileConfig {
    #[serde(default)]
    scan: ScanConfig,
    #[serde(default)]
    cache: CacheConfig,
    #[serde(default)]
    update: UpdateConfig,
    #[serde(default)]
    output: OutputConfig,
    #[serde(default)]
    github: GithubConfig,
}

#[derive(Debug, Deserialize, Default)]
struct ScanConfig {
    include: Option<Vec<String>>,
    exclude: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Default)]
struct CacheConfig {
    enabled: Option<bool>,
    ttl: Option<String>,
    dir: Option<PathBuf>,
}

#[derive(Debug, Deserialize, Default)]
struct UpdateConfig {
    mode: Option<UpdateMode>,
    include_prereleases: Option<bool>,
    preserve_major: Option<bool>,
    missing_ref: Option<MissingRefPolicy>,
}

#[derive(Debug, Deserialize, Default)]
struct OutputConfig {
    format: Option<OutputFormatConfig>,
    color: Option<ColorChoiceConfig>,
}

#[derive(Debug, Deserialize, Default)]
struct GithubConfig {
    api_url: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum OutputFormatConfig {
    Human,
    Json,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum ColorChoiceConfig {
    Auto,
    Always,
    Never,
}

impl Settings {
    pub fn resolve(cli: &Cli) -> Result<Self> {
        let file_config = load_config(cli.config.as_deref())?;

        let paths = cli.paths.clone();
        let mut include = if let Some(include) = file_config.scan.include {
            include
        } else {
            DEFAULT_INCLUDES
                .iter()
                .map(|value| value.to_string())
                .collect()
        };
        include.extend(cli.include.iter().cloned());

        let exclude = if !cli.exclude.is_empty() {
            cli.exclude.clone()
        } else {
            file_config.scan.exclude.unwrap_or_default()
        };

        let cache_dir = cli
            .cache_dir
            .clone()
            .or_else(|| env::var_os("GHAU_CACHE_DIR").map(PathBuf::from))
            .or(file_config.cache.dir)
            .or_else(default_cache_dir)
            .unwrap_or_else(|| PathBuf::from(".gh-actions-updater-cache"));

        let ttl_raw = cli
            .cache_ttl
            .clone()
            .or_else(|| env::var("GHAU_CACHE_TTL").ok())
            .or(file_config.cache.ttl)
            .unwrap_or_else(|| "6h".to_string());

        let update_mode = if cli.latest_hash {
            UpdateMode::LatestHash
        } else {
            file_config.update.mode.unwrap_or(UpdateMode::LatestTag)
        };

        let github_token = cli
            .github_token
            .clone()
            .or_else(|| env::var("GHAU_GITHUB_TOKEN").ok())
            .or_else(|| env::var("GITHUB_TOKEN").ok())
            .or_else(|| env::var("GH_TOKEN").ok());
        let github_token_present = github_token.is_some();

        let format = if let Some(format) = cli.format {
            format
        } else {
            match file_config.output.format {
                Some(OutputFormatConfig::Json) => OutputFormat::Json,
                _ => OutputFormat::Human,
            }
        };

        let color = if let Some(color) = cli.color {
            color
        } else {
            match file_config.output.color {
                Some(ColorChoiceConfig::Always) => ColorChoice::Always,
                Some(ColorChoiceConfig::Never) => ColorChoice::Never,
                _ => ColorChoice::Auto,
            }
        };

        let cache_enabled = !cli.no_cache && file_config.cache.enabled.unwrap_or(true);

        let _ = (
            file_config.update.include_prereleases,
            file_config.update.preserve_major,
        );

        Ok(Self {
            paths,
            include,
            exclude,
            cache_dir,
            cache_ttl: parse_ttl(&ttl_raw)?,
            cache_enabled,
            refresh_cache: cli.refresh_cache,
            update: cli.update,
            latest_hash: matches!(update_mode, UpdateMode::LatestHash),
            missing_ref: cli
                .missing_ref
                .or(file_config.update.missing_ref)
                .unwrap_or(MissingRefPolicy::Warn),
            include_prereleases: file_config.update.include_prereleases.unwrap_or(false),
            preserve_major: file_config.update.preserve_major.unwrap_or(true),
            check: cli.check,
            dry_run: cli.dry_run || cli.check || !cli.update,
            diff: cli.diff,
            format,
            quiet: cli.quiet,
            verbose: cli.verbose,
            color,
            github_token_present,
            github_token,
            github_api_url: cli
                .github_api_url
                .clone()
                .or(file_config.github.api_url)
                .unwrap_or_else(|| "https://api.github.com".to_string()),
            strict_schema: cli.strict_schema,
            schema_validation: !cli.no_schema_validation,
        })
    }
}

fn load_config(explicit: Option<&Path>) -> Result<FileConfig> {
    let Some(path) = explicit.map(PathBuf::from).or_else(discover_config) else {
        return Ok(FileConfig::default());
    };

    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read config file {}", path.display()))?;
    toml::from_str(&content)
        .with_context(|| format!("failed to parse config file {}", path.display()))
}

fn discover_config() -> Option<PathBuf> {
    let mut dir = env::current_dir().ok()?;
    loop {
        let candidate = dir.join(".gh-actions-updater.toml");
        if candidate.is_file() {
            return Some(candidate);
        }
        if dir.join(".git").exists() {
            return None;
        }
        if !dir.pop() {
            return None;
        }
    }
}

fn default_cache_dir() -> Option<PathBuf> {
    dirs::cache_dir().map(|path| path.join("gh-actions-updater"))
}

pub fn parse_ttl(value: &str) -> Result<CacheTtl> {
    let trimmed = value.trim();
    if trimmed.eq_ignore_ascii_case("never") {
        return Ok(CacheTtl::Never);
    }

    let split_at = trimmed
        .find(|ch: char| !ch.is_ascii_digit())
        .unwrap_or(trimmed.len());
    let (number, unit) = trimmed.split_at(split_at);
    let amount: u64 = number
        .parse()
        .with_context(|| format!("invalid cache ttl: {value}"))?;
    let multiplier = match unit {
        "" | "s" => 1,
        "m" => 60,
        "h" => 60 * 60,
        "d" => 60 * 60 * 24,
        _ => anyhow::bail!("invalid cache ttl unit in {value}; use s, m, h, d, or never"),
    };
    Ok(CacheTtl::Seconds(amount.saturating_mul(multiplier)))
}

#[cfg(test)]
mod tests {
    use super::{CacheTtl, Settings, parse_ttl};
    use crate::cli::{Cli, ColorChoice, MissingRefPolicy, OutputFormat};
    use clap::Parser;

    #[test]
    fn parses_ttl_values() {
        assert_eq!(parse_ttl("0").unwrap(), CacheTtl::Seconds(0));
        assert_eq!(parse_ttl("15m").unwrap(), CacheTtl::Seconds(900));
        assert_eq!(parse_ttl("6h").unwrap(), CacheTtl::Seconds(21_600));
        assert_eq!(parse_ttl("7d").unwrap(), CacheTtl::Seconds(604_800));
        assert_eq!(parse_ttl("never").unwrap(), CacheTtl::Never);
    }

    #[test]
    fn cli_overrides_defaults() {
        let cli = Cli::parse_from([
            "gh-actions-updater",
            "--include",
            ".github/workflows/*.yml",
            "--exclude",
            "**/skip.yml",
            "--no-cache",
            "--cache-ttl",
            "0",
            "--missing-ref",
            "error",
            "--format",
            "json",
            "--color",
            "never",
        ]);
        let settings = Settings::resolve(&cli).unwrap();
        assert!(
            settings
                .include
                .contains(&".github/workflows/*.yml".to_string())
        );
        assert_eq!(settings.exclude, vec!["**/skip.yml"]);
        assert_eq!(settings.cache_ttl, CacheTtl::Seconds(0));
        assert!(!settings.cache_enabled);
        assert_eq!(settings.missing_ref, MissingRefPolicy::Error);
        assert_eq!(settings.format, OutputFormat::Json);
        assert_eq!(settings.color, ColorChoice::Never);
    }

    #[test]
    fn cli_output_options_override_config() {
        let temp = tempfile::tempdir().unwrap();
        let config = temp.path().join(".gh-actions-updater.toml");
        std::fs::write(
            &config,
            r#"
[output]
format = "json"
color = "always"
"#,
        )
        .unwrap();

        let cli = Cli::parse_from([
            "gh-actions-updater",
            "--config",
            config.to_str().unwrap(),
            "--format",
            "human",
            "--color",
            "never",
        ]);
        let settings = Settings::resolve(&cli).unwrap();
        assert_eq!(settings.format, OutputFormat::Human);
        assert_eq!(settings.color, ColorChoice::Never);
    }
}
