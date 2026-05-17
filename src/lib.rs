mod action_ref;
mod cache;
mod cli;
mod config;
mod discover;
mod metadata;
mod report;
mod scanner;

use anyhow::{Context, Result};
use clap::Parser;
use cli::Cli;
use report::RunReport;
use std::io;

pub fn run() -> Result<()> {
    let cli = Cli::parse();
    let settings = config::Settings::resolve(&cli).context("failed to resolve configuration")?;
    validate_supported_iteration(&cli, &settings);
    let cache = cache::CacheState::prepare(&settings).context("failed to prepare cache")?;
    let files = discover::discover_files(&settings).context("failed to discover files")?;
    let scan = scanner::scan_files(&files, &settings).context("failed to scan files")?;
    let resolution = match metadata::resolve_updates(&settings, cache, &scan.references)
        .context("failed to resolve action metadata")
    {
        Ok(resolution) => resolution,
        Err(error) => {
            eprintln!("{error:#}");
            std::process::exit(4);
        }
    };
    let exit_code = metadata::exit_code_for_resolution(&settings, &resolution);

    let report = RunReport::from_scan(
        env!("CARGO_PKG_VERSION"),
        &settings,
        resolution,
        files,
        scan,
    );
    report.write(&mut io::stdout(), &mut io::stderr())?;

    if let Some(exit_code) = exit_code {
        std::process::exit(exit_code);
    }

    Ok(())
}

fn validate_supported_iteration(cli: &Cli, settings: &config::Settings) {
    let mut unsupported = Vec::new();
    if settings.update {
        unsupported.push("--update");
    }
    if settings.latest_hash {
        unsupported.push("--latest-hash");
    }
    if settings.diff {
        unsupported.push("--diff");
    }
    if settings.strict_schema {
        unsupported.push("--strict-schema");
    }
    if cli.github_token.is_some() {
        unsupported.push("--github-token");
    }
    if cli.github_api_url.is_some() {
        unsupported.push("--github-api-url");
    }
    if settings.github_api_url != "https://api.github.com" {
        unsupported.push("github.api_url");
    }

    if !unsupported.is_empty() {
        eprintln!(
            "unsupported in this scanner iteration: {}",
            unsupported.join(", ")
        );
        std::process::exit(2);
    }
}
