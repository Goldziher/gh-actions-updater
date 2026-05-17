use crate::cache::CacheReport;
use crate::cli::OutputFormat;
use crate::config::Settings;
use crate::metadata::MetadataResolution;
use crate::scanner::{Diagnostic, FileReport, ReferenceReport, ScanOutput};
use anyhow::Result;
use serde::Serialize;
use std::io::Write;
use std::path::PathBuf;

#[derive(Debug, Serialize)]
pub struct RunReport {
    pub version: String,
    pub changed: bool,
    pub summary: Summary,
    pub files: Vec<FileReport>,
    pub references: Vec<ReferenceReport>,
    pub updates: Vec<UpdateReport>,
    pub diagnostics: Vec<Diagnostic>,
    pub cache: CacheReport,

    #[serde(skip)]
    format: OutputFormat,
    #[serde(skip)]
    quiet: bool,
    #[serde(skip)]
    verbose: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Summary {
    pub files_scanned: usize,
    pub references_found: usize,
    pub updates_available: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateReport {
    pub file: String,
    pub line: usize,
    pub current: String,
    pub target: Option<String>,
}

impl RunReport {
    pub fn from_scan(
        version: &str,
        settings: &Settings,
        resolution: MetadataResolution,
        files: Vec<PathBuf>,
        scan: ScanOutput,
    ) -> Self {
        let _ = files;
        let references_found = scan.references.len();
        let _ = (
            settings.dry_run,
            settings.color,
            settings.github_token_present,
            settings.schema_validation,
            settings.missing_ref,
            settings.refresh_cache,
            settings.include_prereleases,
            settings.preserve_major,
        );
        Self {
            version: version.to_string(),
            changed: false,
            summary: Summary {
                files_scanned: scan.files.len(),
                references_found,
                updates_available: resolution.updates.len(),
            },
            files: scan.files,
            references: scan.references,
            updates: resolution.updates,
            diagnostics: {
                let mut diagnostics = scan.diagnostics;
                diagnostics.extend(resolution.diagnostics);
                diagnostics
            },
            cache: resolution.cache,
            format: settings.format,
            quiet: settings.quiet,
            verbose: settings.verbose,
        }
    }

    pub fn write(&self, stdout: &mut impl Write, stderr: &mut impl Write) -> Result<()> {
        match self.format {
            OutputFormat::Json => {
                serde_json::to_writer_pretty(&mut *stdout, self)?;
                writeln!(stdout)?;
            }
            OutputFormat::Human => {
                if !self.quiet {
                    writeln!(
                        stdout,
                        "Scanned {} file(s), found {} reference(s), {} update(s) available.",
                        self.summary.files_scanned,
                        self.summary.references_found,
                        self.summary.updates_available
                    )?;

                    for reference in &self.references {
                        writeln!(
                            stdout,
                            "{}:{}:{} {} ({:?})",
                            reference.file,
                            reference.line,
                            reference.column,
                            reference.raw,
                            reference.parsed.kind
                        )?;
                    }

                    for update in &self.updates {
                        let target = update.target.as_deref().unwrap_or("<unknown>");
                        writeln!(
                            stdout,
                            "update {}:{} {} -> {}",
                            update.file, update.line, update.current, target
                        )?;
                    }
                }
            }
        }

        for diagnostic in &self.diagnostics {
            if let Some(line) = diagnostic.line {
                writeln!(stderr, "{}:{line}: {}", diagnostic.file, diagnostic.message)?;
            } else {
                writeln!(stderr, "{}: {}", diagnostic.file, diagnostic.message)?;
            }
        }

        if self.verbose {
            writeln!(
                stderr,
                "cache enabled={}, fresh_hits={}, stale_hits={}, misses={}, refreshes={}",
                self.cache.enabled,
                self.cache.fresh_hits,
                self.cache.stale_hits,
                self.cache.misses,
                self.cache.refreshes
            )?;
        }

        Ok(())
    }
}
