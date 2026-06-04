use crate::config::Settings;
use crate::report::UpdateReport;
use crate::scanner::{ByteSpan, Diagnostic, DiagnosticCategory};
use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::fs;

#[derive(Debug, Default)]
pub struct RewriteResult {
    pub changed: bool,
    pub would_change: bool,
    pub diagnostics: Vec<Diagnostic>,
    pub diffs: Vec<String>,
}

#[derive(Debug, Clone)]
struct Replacement {
    span: ByteSpan,
    current: String,
    target: String,
    line: usize,
}

pub fn apply_updates(settings: &Settings, updates: &[UpdateReport]) -> Result<RewriteResult> {
    let mut result = RewriteResult::default();
    if updates.is_empty() {
        return Ok(result);
    }

    let mut by_file: BTreeMap<&str, Vec<Replacement>> = BTreeMap::new();
    for update in updates {
        let Some(target) = update.target.clone() else {
            continue;
        };
        let Some(span) = update.ref_span else {
            result.diagnostics.push(Diagnostic {
                file: update.file.clone(),
                line: Some(update.line),
                message: update.rewrite_reason.clone().unwrap_or_else(|| {
                    "update skipped because no safe source span was found".to_string()
                }),
                category: DiagnosticCategory::General,
            });
            continue;
        };
        if !update.rewrite_supported {
            result.diagnostics.push(Diagnostic {
                file: update.file.clone(),
                line: Some(update.line),
                message: update.rewrite_reason.clone().unwrap_or_else(|| {
                    "update skipped because the reference is not rewrite-safe".to_string()
                }),
                category: DiagnosticCategory::General,
            });
            continue;
        }

        by_file.entry(&update.file).or_default().push(Replacement {
            span,
            current: update.current.clone(),
            target,
            line: update.line,
        });
    }

    for (file, mut replacements) in by_file {
        replacements.sort_by_key(|replacement| replacement.span.start);
        if has_overlaps(&replacements) {
            result.diagnostics.push(Diagnostic {
                file: file.to_string(),
                line: None,
                message: "updates skipped because rewrite spans overlap".to_string(),
                category: DiagnosticCategory::General,
            });
            continue;
        }

        let original =
            fs::read_to_string(file).with_context(|| format!("failed to read {file}"))?;
        if !spans_are_valid(&original, &replacements) {
            for replacement in replacements {
                result.diagnostics.push(Diagnostic {
                    file: file.to_string(),
                    line: Some(replacement.line),
                    message: "update skipped because the source span no longer matches the file"
                        .to_string(),
                    category: DiagnosticCategory::General,
                });
            }
            continue;
        }

        let mut rewritten = original.clone();
        for replacement in replacements.iter().rev() {
            rewritten.replace_range(
                replacement.span.start..replacement.span.end,
                &replacement.target,
            );
        }

        if rewritten == original {
            continue;
        }

        result.would_change = true;
        if settings.diff {
            result
                .diffs
                .push(render_unified_diff(file, &original, &rewritten));
        }
        if !settings.dry_run {
            fs::write(file, rewritten).with_context(|| format!("failed to write {file}"))?;
            result.changed = true;
        }
    }

    Ok(result)
}

fn has_overlaps(replacements: &[Replacement]) -> bool {
    replacements
        .windows(2)
        .any(|window| window[0].span.end > window[1].span.start)
}

fn spans_are_valid(content: &str, replacements: &[Replacement]) -> bool {
    replacements.iter().all(|replacement| {
        replacement.span.end <= content.len()
            && content.is_char_boundary(replacement.span.start)
            && content.is_char_boundary(replacement.span.end)
            && content[replacement.span.start..replacement.span.end] == replacement.current
    })
}

fn render_unified_diff(file: &str, original: &str, rewritten: &str) -> String {
    let original_lines: Vec<&str> = original.lines().collect();
    let rewritten_lines: Vec<&str> = rewritten.lines().collect();
    let mut prefix = 0;
    while prefix < original_lines.len()
        && prefix < rewritten_lines.len()
        && original_lines[prefix] == rewritten_lines[prefix]
    {
        prefix += 1;
    }

    let mut suffix = 0;
    while suffix < original_lines.len().saturating_sub(prefix)
        && suffix < rewritten_lines.len().saturating_sub(prefix)
        && original_lines[original_lines.len() - 1 - suffix]
            == rewritten_lines[rewritten_lines.len() - 1 - suffix]
    {
        suffix += 1;
    }

    let context = 3;
    let original_change_end = original_lines.len() - suffix;
    let rewritten_change_end = rewritten_lines.len() - suffix;
    let hunk_start = prefix.saturating_sub(context);
    let original_hunk_end = (original_change_end + context).min(original_lines.len());
    let rewritten_hunk_end = (rewritten_change_end + context).min(rewritten_lines.len());
    let original_count = original_hunk_end.saturating_sub(hunk_start).max(1);
    let rewritten_count = rewritten_hunk_end.saturating_sub(hunk_start).max(1);
    let diff_path = diff_path(file);

    format!(
        "diff --git a/{diff_path} b/{diff_path}\n--- a/{diff_path}\n+++ b/{diff_path}\n@@ -{},{} +{},{} @@\n{}",
        hunk_start + 1,
        original_count,
        hunk_start + 1,
        rewritten_count,
        render_diff_hunk(
            &original_lines,
            &rewritten_lines,
            DiffWindow {
                hunk_start,
                change_start: prefix,
                original_change_end,
                rewritten_change_end,
                original_hunk_end,
                rewritten_hunk_end,
            },
        )
    )
}

struct DiffWindow {
    hunk_start: usize,
    change_start: usize,
    original_change_end: usize,
    rewritten_change_end: usize,
    original_hunk_end: usize,
    rewritten_hunk_end: usize,
}

fn render_diff_hunk(
    original_lines: &[&str],
    rewritten_lines: &[&str],
    window: DiffWindow,
) -> String {
    let mut body = String::new();
    for line in &original_lines[window.hunk_start..window.change_start] {
        body.push(' ');
        body.push_str(line);
        body.push('\n');
    }
    for line in &original_lines[window.change_start..window.original_change_end] {
        body.push('-');
        body.push_str(line);
        body.push('\n');
    }
    for line in &rewritten_lines[window.change_start..window.rewritten_change_end] {
        body.push('+');
        body.push_str(line);
        body.push('\n');
    }
    let original_tail = &original_lines[window.original_change_end..window.original_hunk_end];
    let rewritten_tail = &rewritten_lines[window.rewritten_change_end..window.rewritten_hunk_end];
    for line in original_tail.iter().take(rewritten_tail.len()) {
        body.push(' ');
        body.push_str(line);
        body.push('\n');
    }
    body
}

fn diff_path(file: &str) -> &str {
    file.trim_start_matches('/')
}

#[cfg(test)]
mod tests {
    use super::apply_updates;
    use crate::cli::{ColorChoice, MissingRefPolicy, OutputFormat, PinStyle};
    use crate::config::{CacheTtl, Settings};
    use crate::report::UpdateReport;
    use crate::scanner::ByteSpan;
    use std::fs;
    use std::path::Path;

    fn settings(root: &Path, dry_run: bool, diff: bool) -> Settings {
        Settings {
            paths: vec![root.display().to_string()],
            include: Vec::new(),
            exclude: Vec::new(),
            cache_dir: root.join(".cache"),
            cache_ttl: CacheTtl::Seconds(3600),
            cache_enabled: false,
            refresh_cache: false,
            update: !dry_run,
            latest_hash: false,
            pin_style: PinStyle::Preserve,
            update_exclude: Vec::new(),
            missing_ref: MissingRefPolicy::Warn,
            include_prereleases: false,
            preserve_major: true,
            check: false,
            dry_run,
            diff,
            format: OutputFormat::Human,
            quiet: false,
            verbose: false,
            color: ColorChoice::Auto,
            github_token_present: false,
            github_token: None,
            github_api_url: "https://api.github.com".to_string(),
            strict_schema: false,
            schema_validation: false,
            recursive: false,
            threads: None,
            validate: false,
            pin_floating_to_sha: false,
        }
    }

    #[test]
    fn rewrites_only_ref_span() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("ci.yml");
        let content = "jobs:\n  test:\n    steps:\n      - uses: actions/checkout@v4\n";
        fs::write(&path, content).unwrap();
        let start = content.find("v4").unwrap();
        let update = UpdateReport {
            file: path.display().to_string(),
            line: 4,
            current: "v4".to_string(),
            target: Some("v4.2.0".to_string()),
            ref_span: Some(ByteSpan {
                start,
                end: start + 2,
            }),
            rewrite_supported: true,
            rewrite_reason: None,
        };

        let result = apply_updates(&settings(temp.path(), false, false), &[update]).unwrap();

        assert!(result.changed);
        assert!(result.would_change);
        assert_eq!(
            fs::read_to_string(path).unwrap(),
            "jobs:\n  test:\n    steps:\n      - uses: actions/checkout@v4.2.0\n"
        );
    }

    #[test]
    fn dry_run_diff_does_not_write() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("ci.yml");
        let content = "jobs:\n  test:\n    steps:\n      - uses: actions/checkout@v4\n";
        fs::write(&path, content).unwrap();
        let start = content.find("v4").unwrap();
        let update = UpdateReport {
            file: path.display().to_string(),
            line: 4,
            current: "v4".to_string(),
            target: Some("v4.2.0".to_string()),
            ref_span: Some(ByteSpan {
                start,
                end: start + 2,
            }),
            rewrite_supported: true,
            rewrite_reason: None,
        };

        let result = apply_updates(&settings(temp.path(), true, true), &[update]).unwrap();

        assert!(!result.changed);
        assert!(result.would_change);
        assert_eq!(fs::read_to_string(path).unwrap(), content);
        assert!(result.diffs[0].contains("actions/checkout@v4.2.0"));
        assert!(!result.diffs[0].contains("a//"));
    }
}
