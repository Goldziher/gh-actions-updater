use crate::cache::CacheReport;
use crate::cli::{ColorChoice, OutputFormat};
use crate::config::Settings;
use crate::metadata::MetadataResolution;
use crate::scanner::{Diagnostic, FileReport, ReferenceReport, ScanOutput};
use anyhow::Result;
use serde::Serialize;
use std::io::{IsTerminal, Write};
use std::path::PathBuf;

#[derive(Debug, Serialize)]
pub struct RunReport {
    pub version: String,
    pub changed: bool,
    pub would_change: bool,
    pub summary: Summary,
    pub files: Vec<FileReport>,
    pub references: Vec<ReferenceReport>,
    pub updates: Vec<UpdateReport>,
    pub diagnostics: Vec<Diagnostic>,
    pub skips: Vec<SkipReport>,
    pub cache: CacheReport,
    pub diffs: Vec<String>,

    #[serde(skip)]
    format: OutputFormat,
    #[serde(skip)]
    quiet: bool,
    #[serde(skip)]
    verbose: bool,
    #[serde(skip)]
    color: bool,
    #[serde(skip)]
    diagnostic_color: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Summary {
    pub files_scanned: usize,
    pub references_found: usize,
    pub updates_available: usize,
    pub skipped: usize,
    pub failures: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct SkipReport {
    pub file: String,
    pub line: usize,
    pub code: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateReport {
    pub file: String,
    pub line: usize,
    pub current: String,
    pub target: Option<String>,
    #[serde(skip)]
    pub ref_span: Option<crate::scanner::ByteSpan>,
    #[serde(skip)]
    pub rewrite_supported: bool,
    #[serde(skip)]
    pub rewrite_reason: Option<String>,
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
            settings.github_token_present,
            settings.schema_validation,
            settings.missing_ref,
            settings.refresh_cache,
            settings.include_prereleases,
            settings.preserve_major,
            settings.pin_style,
            settings.recursive,
            settings.threads,
        );
        let color = match settings.color {
            ColorChoice::Always => true,
            ColorChoice::Never => false,
            ColorChoice::Auto => std::io::stdout().is_terminal(),
        };
        let diagnostic_color = match settings.color {
            ColorChoice::Always => true,
            ColorChoice::Never => false,
            ColorChoice::Auto => std::io::stderr().is_terminal(),
        };
        let would_change = !resolution.updates.is_empty();
        let mut skips: Vec<_> = scan
            .references
            .iter()
            .filter_map(|reference| {
                skip_code(reference, settings).map(|code| SkipReport {
                    file: reference.file.clone(),
                    line: reference.line,
                    code,
                })
            })
            .collect();
        skips.extend(
            resolution
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code == crate::scanner::DiagnosticCode::MetadataLookupFailed)
                .map(|diagnostic| SkipReport {
                    file: diagnostic.file.clone(),
                    line: diagnostic.line.unwrap_or_default(),
                    code: "metadata_lookup_failed",
                }),
        );
        let failures = scan
            .diagnostics
            .iter()
            .chain(resolution.diagnostics.iter())
            .filter(|diagnostic| {
                matches!(
                    diagnostic.category,
                    crate::scanner::DiagnosticCategory::Validation | crate::scanner::DiagnosticCategory::Metadata
                )
            })
            .count();
        Self {
            version: version.to_string(),
            changed: false,
            would_change,
            summary: Summary {
                files_scanned: scan.files.len(),
                references_found,
                updates_available: resolution.updates.len(),
                skipped: skips.len(),
                failures,
            },
            files: scan.files,
            references: scan.references,
            updates: resolution.updates,
            diagnostics: {
                let mut diagnostics = scan.diagnostics;
                diagnostics.extend(resolution.diagnostics);
                diagnostics
            },
            skips,
            cache: resolution.cache,
            diffs: Vec::new(),
            format: settings.format,
            quiet: settings.quiet,
            verbose: settings.verbose,
            color,
            diagnostic_color,
        }
    }

    pub fn set_rewrite_result(&mut self, result: crate::rewrite::RewriteResult) {
        self.changed = result.changed;
        self.would_change = result.would_change || !self.updates.is_empty();
        self.diagnostics.extend(result.diagnostics);
        self.diffs = result.diffs;
    }

    pub fn write(&self, stdout: &mut impl Write, stderr: &mut impl Write) -> Result<()> {
        match self.format {
            OutputFormat::Json => {
                serde_json::to_writer_pretty(&mut *stdout, self)?;
                writeln!(stdout)?;
            }
            OutputFormat::Human => {
                if !self.quiet {
                    self.write_human(stdout)?;
                }
            }
        }

        for diagnostic in &self.diagnostics {
            if let Some(line) = diagnostic.line {
                writeln!(
                    stderr,
                    "{} {}:{line}: {}",
                    self.style_with("diagnostic", Color::Yellow, self.diagnostic_color),
                    diagnostic.file,
                    diagnostic.message
                )?;
            } else {
                writeln!(
                    stderr,
                    "{} {}: {}",
                    self.style_with("diagnostic", Color::Yellow, self.diagnostic_color),
                    diagnostic.file,
                    diagnostic.message
                )?;
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

    fn write_human(&self, stdout: &mut impl Write) -> Result<()> {
        let status = if self.summary.updates_available == 0 && self.diagnostics.is_empty() {
            self.style("ok", Color::Green)
        } else if self.summary.updates_available > 0 {
            self.style("updates", Color::Yellow)
        } else {
            self.style("diagnostics", Color::Yellow)
        };
        writeln!(stdout, "{} {}", self.style("gh-actions-updater", Color::Cyan), status)?;
        writeln!(
            stdout,
            "  scanned {} file(s), found {} reference(s), {} update(s)",
            self.summary.files_scanned, self.summary.references_found, self.summary.updates_available
        )?;

        if !self.files.is_empty() {
            writeln!(stdout)?;
            writeln!(stdout, "{}", self.style("Files", Color::Bold))?;
            for file in &self.files {
                writeln!(
                    stdout,
                    "  {} {} ({:?}, {} ref{})",
                    self.style("scan", Color::Blue),
                    file.path,
                    file.kind,
                    file.references,
                    plural(file.references)
                )?;
            }
        }

        if !self.references.is_empty() {
            writeln!(stdout)?;
            writeln!(stdout, "{}", self.style("References", Color::Bold))?;
            for reference in &self.references {
                writeln!(
                    stdout,
                    "  {} {}:{}:{} {} ({:?})",
                    self.style("ref", Color::Blue),
                    reference.file,
                    reference.line,
                    reference.column,
                    reference.raw,
                    reference.parsed.kind
                )?;
            }
        }

        if !self.updates.is_empty() {
            writeln!(stdout)?;
            writeln!(stdout, "{}", self.style("Updates", Color::Bold))?;
            for update in &self.updates {
                let target = update.target.as_deref().unwrap_or("<unknown>");
                writeln!(
                    stdout,
                    "  {} {}:{} {} -> {}",
                    self.style("update", Color::Yellow),
                    update.file,
                    update.line,
                    update.current,
                    target
                )?;
            }
        } else {
            writeln!(stdout)?;
            writeln!(stdout, "{}", self.style("No updates available.", Color::Green))?;
        }

        if !self.diffs.is_empty() {
            writeln!(stdout)?;
            writeln!(stdout, "{}", self.style("Diffs", Color::Bold))?;
            for diff in &self.diffs {
                writeln!(stdout, "{diff}")?;
            }
        }

        Ok(())
    }

    fn style(&self, value: &str, color: Color) -> String {
        self.style_with(value, color, self.color)
    }

    fn style_with(&self, value: &str, color: Color, enabled: bool) -> String {
        if !enabled {
            return value.to_string();
        }
        let code = match color {
            Color::Bold => "1",
            Color::Blue => "34",
            Color::Cyan => "36",
            Color::Green => "32",
            Color::Yellow => "33",
        };
        format!("\x1b[{code}m{value}\x1b[0m")
    }
}

fn skip_code(reference: &ReferenceReport, settings: &Settings) -> Option<&'static str> {
    if reference.update_ignored {
        return Some("inline_ignore");
    }
    let repository = match (reference.parsed.owner.as_deref(), reference.parsed.repo.as_deref()) {
        (Some(owner), Some(repo)) => Some(format!("{owner}/{repo}")),
        _ => None,
    };
    if settings.update_exclude.iter().any(|pattern| {
        glob::Pattern::new(pattern).is_ok_and(|pattern| {
            pattern.matches(&reference.raw)
                || repository
                    .as_deref()
                    .is_some_and(|repository| pattern.matches(repository))
        })
    }) {
        return Some("update_excluded");
    }
    match reference.parsed.kind {
        crate::action_ref::ReferenceKind::LocalAction | crate::action_ref::ReferenceKind::LocalWorkflow => {
            return Some("local_reference");
        }
        crate::action_ref::ReferenceKind::DockerImage => return Some("docker_reference"),
        _ => {}
    }
    if !reference.rewrite_supported && reference.parsed.ref_name.is_some() {
        return Some("rewrite_unsafe");
    }
    if matches!(
        reference.parsed.ref_kind,
        crate::action_ref::RefKind::Branch
            | crate::action_ref::RefKind::BranchOrUnknown
            | crate::action_ref::RefKind::NonSemverTag
    ) && !settings.pin_floating_to_sha
    {
        return Some("floating_ref_requires_opt_in");
    }
    None
}

#[derive(Clone, Copy)]
enum Color {
    Bold,
    Blue,
    Cyan,
    Green,
    Yellow,
}

fn plural(count: usize) -> &'static str {
    if count == 1 { "" } else { "s" }
}
