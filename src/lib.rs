mod action_ref;
mod cache;
mod cli;
mod config;
mod discover;
mod init;
mod metadata;
mod report;
mod rewrite;
mod scanner;

use anyhow::{Context, Result};
use clap::Parser;
use cli::Cli;
use report::RunReport;
use std::io;

pub fn run() -> Result<()> {
    let cli = Cli::parse();
    if cli.init {
        if let Err(error) = init::run(&cli).context("failed to initialize configuration") {
            eprintln!("{error:#}");
            std::process::exit(2);
        }
        return Ok(());
    }

    let settings = match config::Settings::resolve(&cli).context("failed to resolve configuration") {
        Ok(settings) => settings,
        Err(error) => {
            eprintln!("{error:#}");
            std::process::exit(2);
        }
    };
    if let Some(threads) = settings.threads {
        if let Err(error) = rayon::ThreadPoolBuilder::new().num_threads(threads).build_global() {
            eprintln!("failed to configure rayon thread pool: {error}");
            std::process::exit(2);
        }
    }
    let cache = match cache::CacheState::prepare(&settings).context("failed to prepare cache") {
        Ok(cache) => cache,
        Err(error) => {
            eprintln!("{error:#}");
            std::process::exit(2);
        }
    };
    let files = match discover::discover_files(&settings).context("failed to discover files") {
        Ok(files) => files,
        Err(error) => {
            eprintln!("{error:#}");
            std::process::exit(3);
        }
    };
    let mut scan = match scanner::scan_files(&files, &settings).context("failed to scan files") {
        Ok(scan) => scan,
        Err(error) => {
            eprintln!("{error:#}");
            std::process::exit(3);
        }
    };
    let has_scan_failures = scanner::has_scan_failure_diagnostics(&scan.diagnostics);
    if settings.strict_schema && scanner::has_schema_diagnostics(&scan.diagnostics) {
        let report = RunReport::from_scan(
            env!("CARGO_PKG_VERSION"),
            &settings,
            metadata::MetadataResolution {
                updates: Vec::new(),
                diagnostics: Vec::new(),
                cache: cache.report.clone(),
                resolved_ref_kinds: Vec::new(),
                has_metadata_failures: false,
            },
            files,
            scan,
        );
        report.write(&mut io::stdout(), &mut io::stderr())?;
        std::process::exit(3);
    }
    let resolution = match metadata::resolve_updates(&settings, cache, &scan.references)
        .context("failed to resolve action metadata")
    {
        Ok(resolution) => resolution,
        Err(error) => {
            eprintln!("{error:#}");
            std::process::exit(4);
        }
    };
    for (idx, kind) in &resolution.resolved_ref_kinds {
        if let Some(reference) = scan.references.get_mut(*idx) {
            reference.parsed.ref_kind = kind.clone();
        }
    }
    let exit_code = metadata::exit_code_for_resolution(&settings, &resolution);
    let has_metadata_failures = resolution.has_metadata_failures;
    let rewrite_result = if settings.update || settings.diff {
        match rewrite::apply_updates(&settings, &resolution.updates).context("failed to apply updates") {
            Ok(result) => result,
            Err(error) => {
                eprintln!("{error:#}");
                std::process::exit(5);
            }
        }
    } else {
        rewrite::RewriteResult::default()
    };

    let mut report = RunReport::from_scan(env!("CARGO_PKG_VERSION"), &settings, resolution, files, scan);
    report.set_rewrite_result(rewrite_result);
    report.write(&mut io::stdout(), &mut io::stderr())?;

    if has_scan_failures {
        std::process::exit(3);
    }

    if has_metadata_failures {
        std::process::exit(4);
    }

    if let Some(exit_code) = exit_code {
        std::process::exit(exit_code);
    }

    Ok(())
}
