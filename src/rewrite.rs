use crate::config::Settings;
use crate::report::UpdateReport;
use crate::scanner::{ByteSpan, Diagnostic, DiagnosticCategory};
use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::fs;

#[derive(Debug, Default)]
pub struct RewriteResult {
    pub changed: bool,
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

        result.changed = true;
        if settings.diff {
            result
                .diffs
                .push(render_full_file_diff(file, &original, &rewritten));
        }
        if !settings.dry_run {
            fs::write(file, rewritten).with_context(|| format!("failed to write {file}"))?;
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

fn render_full_file_diff(file: &str, original: &str, rewritten: &str) -> String {
    let original_line_count = original.lines().count().max(1);
    let rewritten_line_count = rewritten.lines().count().max(1);
    format!(
        "diff --git a/{file} b/{file}\n--- a/{file}\n+++ b/{file}\n@@ -1,{original_line_count} +1,{rewritten_line_count} @@\n{}",
        render_diff_body(original, rewritten)
    )
}

fn render_diff_body(original: &str, rewritten: &str) -> String {
    let mut body = String::new();
    for line in original.lines() {
        body.push('-');
        body.push_str(line);
        body.push('\n');
    }
    for line in rewritten.lines() {
        body.push('+');
        body.push_str(line);
        body.push('\n');
    }
    body
}

#[cfg(test)]
mod tests {
    use super::apply_updates;
    use crate::cli::{ColorChoice, MissingRefPolicy, OutputFormat};
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

        assert!(result.changed);
        assert_eq!(fs::read_to_string(path).unwrap(), content);
        assert!(result.diffs[0].contains("actions/checkout@v4.2.0"));
    }
}
