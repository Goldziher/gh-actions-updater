//! Offline pin-policy lint: every remote `uses:` ref must be a bare `vN` major
//! tag, except a full version tag on an action listed in `[update] allow_full`.

use crate::action_ref::{RefKind, ReferenceKind};
use crate::cli::OutputFormat;
use crate::config::Settings;
use crate::scanner::{Diagnostic, DiagnosticCategory, ReferenceReport, ScanOutput};
use ahash::AHashSet;
use anyhow::{Context, Result};
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use serde::Serialize;
use std::io::Write;

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingKind {
    Sha,
    FullTag,
    BranchOrOther,
    Malformed,
}

#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    pub file: String,
    pub line: usize,
    pub uses: String,
    #[serde(rename = "ref")]
    pub ref_name: Option<String>,
    pub kind: FindingKind,
    pub reason: String,
}

#[derive(Debug, Serialize)]
pub struct Summary {
    pub files_scanned: usize,
    pub references_checked: usize,
    pub findings: usize,
}

#[derive(Debug, Serialize)]
pub struct LintReport {
    pub version: &'static str,
    pub summary: Summary,
    pub findings: Vec<Finding>,
    pub diagnostics: Vec<Diagnostic>,
}

pub fn lint(settings: &Settings, scan: &ScanOutput) -> Result<LintReport> {
    let allow_full = build_allow_full(&settings.allow_full)?;
    let mut references_checked = 0;
    let mut findings = Vec::new();
    // A YAML alias (`- *step`) resolves to the anchor's node, so the same ref is
    // collected again at the anchor's line.
    let mut seen = AHashSet::new();
    for reference in &scan.references {
        if !matches!(
            reference.parsed.kind,
            ReferenceKind::RemoteAction | ReferenceKind::ReusableWorkflow | ReferenceKind::Malformed
        ) {
            continue;
        }
        if !seen.insert((reference.file.as_str(), reference.line, reference.raw.as_str())) {
            continue;
        }
        references_checked += 1;
        if reference.update_ignored {
            continue;
        }
        if let Some(finding) = check_reference(reference, &allow_full) {
            findings.push(finding);
        }
    }
    findings.sort_by(|left, right| left.file.cmp(&right.file).then(left.line.cmp(&right.line)));
    Ok(LintReport {
        version: env!("CARGO_PKG_VERSION"),
        summary: Summary {
            files_scanned: scan.files.len(),
            references_checked,
            findings: findings.len(),
        },
        findings,
        diagnostics: scan
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.category == DiagnosticCategory::ScanFailure)
            .cloned()
            .collect(),
    })
}

fn check_reference(reference: &ReferenceReport, allow_full: &GlobSet) -> Option<Finding> {
    let parsed = &reference.parsed;
    let finding = |kind: FindingKind, reason: String| Finding {
        file: reference.file.clone(),
        line: reference.line,
        uses: reference.raw.clone(),
        ref_name: parsed.ref_name.clone(),
        kind,
        reason,
    };
    let Some(ref_name) = parsed.ref_name.as_deref() else {
        return Some(finding(
            FindingKind::Malformed,
            "missing or malformed @ref; pin a vN major tag".to_string(),
        ));
    };
    if is_bare_major(ref_name) {
        return None;
    }
    match parsed.ref_kind {
        RefKind::Sha => Some(finding(FindingKind::Sha, "commit SHA; pin a vN major tag".to_string())),
        RefKind::SemverLikeTag if ref_name.contains('.') => {
            if is_allowed_full(allow_full, &reference.raw) {
                return None;
            }
            let major = ref_name.split('.').next().unwrap_or(ref_name);
            Some(finding(
                FindingKind::FullTag,
                format!("full version tag; pin the major tag {major}"),
            ))
        }
        _ => Some(finding(
            FindingKind::BranchOrOther,
            "branch or non-version ref; pin a vN major tag".to_string(),
        )),
    }
}

/// A floating major tag: `vN`, or `N` for upstreams that tag without the `v`.
/// The full-tag suggestion keeps the ref's own prefix, so whatever it suggests
/// passes this check.
fn is_bare_major(ref_name: &str) -> bool {
    let digits = ref_name.strip_prefix('v').unwrap_or(ref_name);
    !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
}

fn build_allow_full(patterns: &[String]) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        let glob = GlobBuilder::new(pattern)
            .literal_separator(true)
            .case_insensitive(true)
            .build()
            .with_context(|| format!("invalid allow_full glob {pattern}"))?;
        builder.add(glob);
    }
    builder.build().context("failed to build allow_full glob set")
}

/// Matches the action target without its ref (`owner/repo` or
/// `owner/repo/path`), and also `owner/repo` alone so an allowlisted repo covers
/// its sub-actions and reusable workflows. `*` does not cross `/`, and GitHub
/// owner and repo names are case-insensitive.
fn is_allowed_full(allow_full: &GlobSet, raw: &str) -> bool {
    let target = raw.rsplit_once('@').map_or(raw, |(target, _)| target);
    let repo = target.splitn(3, '/').take(2).collect::<Vec<_>>().join("/");
    allow_full.is_match(target) || allow_full.is_match(&repo)
}

impl LintReport {
    pub fn write(&self, settings: &Settings, stdout: &mut impl Write, stderr: &mut impl Write) -> Result<()> {
        for diagnostic in &self.diagnostics {
            match diagnostic.line {
                Some(line) => writeln!(stderr, "{}:{line}: {}", diagnostic.file, diagnostic.message)?,
                None => writeln!(stderr, "{}: {}", diagnostic.file, diagnostic.message)?,
            }
        }
        if settings.format == OutputFormat::Json {
            serde_json::to_writer_pretty(&mut *stdout, self)?;
            writeln!(stdout)?;
            return Ok(());
        }
        for finding in &self.findings {
            writeln!(
                stdout,
                "{}:{}: {}: {}",
                finding.file, finding.line, finding.uses, finding.reason
            )?;
        }
        if !settings.quiet {
            writeln!(
                stdout,
                "{} pin finding(s) in {} remote reference(s) across {} file(s)",
                self.summary.findings, self.summary.references_checked, self.summary.files_scanned
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{build_allow_full, is_allowed_full, is_bare_major};

    #[test]
    fn allow_full_star_does_not_cross_path_separators() {
        let set = build_allow_full(&["*/setup-*".to_string()]).unwrap();
        assert!(is_allowed_full(&set, "astral-sh/setup-uv@v10.3.3"));
        assert!(!is_allowed_full(&set, "o/r/x/setup-y@v1.2.3"));
    }

    #[test]
    fn bare_major_accepts_v_prefixed_and_unprefixed_digits() {
        assert!(is_bare_major("v4"));
        assert!(is_bare_major("v10"));
        assert!(!is_bare_major("v4.1"));
        assert!(is_bare_major("4"));
        assert!(!is_bare_major(""));
        assert!(!is_bare_major("v"));
        assert!(!is_bare_major("v4-beta"));
        assert!(!is_bare_major("main"));
    }

    #[test]
    fn allow_full_matches_repo_path_and_globs() {
        let set = build_allow_full(&["owner/repo".to_string(), "other/*".to_string()]).unwrap();
        assert!(is_allowed_full(&set, "owner/repo@v1.2.3"));
        assert!(is_allowed_full(&set, "owner/repo/sub/path@v1.2.3"));
        assert!(is_allowed_full(&set, "other/thing/.github/workflows/x.yml@v1.0"));
        assert!(!is_allowed_full(&set, "owner/repository@v1.2.3"));

        let path_only = build_allow_full(&["owner/repo/sub".to_string()]).unwrap();
        assert!(is_allowed_full(&path_only, "Owner/Repo/sub@v1.2.3"));
        assert!(is_allowed_full(&path_only, "owner/repo/sub@v1.2.3"));
        assert!(!is_allowed_full(&path_only, "owner/repo@v1.2.3"));
    }
}
