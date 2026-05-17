use crate::config::Settings;
use ahash::AHashSet;
use anyhow::{Context, Result};
use glob::glob;
use globset::{Glob, GlobSet, GlobSetBuilder};
use ignore::WalkBuilder;
use std::path::{Path, PathBuf};

pub fn discover_files(settings: &Settings) -> Result<Vec<PathBuf>> {
    let include = build_globset(&settings.include)?;
    let exclude = build_globset(&settings.exclude)?;
    let roots: Vec<PathBuf> = if settings.paths.is_empty() {
        vec![PathBuf::from(".")]
    } else {
        settings.paths.iter().map(PathBuf::from).collect()
    };

    let mut files = Vec::new();
    for root in roots {
        if root.is_file() {
            push_explicit_file(&mut files, &root, &exclude);
            continue;
        }

        if !root.exists() && has_glob_meta(&root) {
            for entry in glob(&root.to_string_lossy())
                .with_context(|| format!("invalid glob {}", root.display()))?
            {
                let path =
                    entry.with_context(|| format!("failed to read glob {}", root.display()))?;
                if path.is_file() {
                    push_explicit_file(&mut files, &path, &exclude);
                }
            }
            continue;
        }

        let walker = WalkBuilder::new(&root)
            .hidden(false)
            .git_ignore(true)
            .git_global(true)
            .git_exclude(true)
            .build();

        for entry in walker {
            let entry = entry.with_context(|| format!("failed to walk {}", root.display()))?;
            if entry
                .file_type()
                .is_some_and(|file_type| file_type.is_file())
            {
                push_if_match(&mut files, entry.path(), &root, &include, &exclude);
            }
        }
    }

    let mut seen = AHashSet::with_capacity(files.len());
    files.retain(|path| seen.insert(normalize(path)));
    files.sort();
    Ok(files)
}

fn push_explicit_file(files: &mut Vec<PathBuf>, path: &Path, exclude: &GlobSet) {
    let normalized = normalize(path);
    if is_candidate_file(path) && !matches_glob_or_suffix(exclude, &normalized) {
        files.push(path.to_path_buf());
    }
}

fn push_if_match(
    files: &mut Vec<PathBuf>,
    path: &Path,
    base: &Path,
    include: &GlobSet,
    exclude: &GlobSet,
) {
    let normalized = normalize(path.strip_prefix(base).unwrap_or(path));
    if is_candidate_file(path)
        && include.is_match(&normalized)
        && !matches_glob_or_suffix(exclude, &normalized)
    {
        files.push(path.to_path_buf());
    }
}

fn build_globset(patterns: &[String]) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        builder.add(Glob::new(pattern).with_context(|| format!("invalid glob {pattern}"))?);
    }
    builder.build().context("failed to build glob set")
}

fn normalize(path: &Path) -> String {
    path.strip_prefix(".")
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn is_candidate_file(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(|name| name.to_str()),
        Some("action.yml" | "action.yaml")
    ) || matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("yml" | "yaml")
    )
}

fn has_glob_meta(path: &Path) -> bool {
    path.to_string_lossy()
        .chars()
        .any(|ch| matches!(ch, '*' | '?' | '['))
}

fn matches_glob_or_suffix(globset: &GlobSet, normalized: &str) -> bool {
    if globset.is_match(normalized) {
        return true;
    }

    let mut start = 0;
    while let Some(offset) = normalized[start..].find('/') {
        start += offset + 1;
        if globset.is_match(&normalized[start..]) {
            return true;
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::discover_files;
    use crate::cli::{ColorChoice, MissingRefPolicy, OutputFormat};
    use crate::config::{CacheTtl, DEFAULT_INCLUDES, Settings};
    use std::fs;
    use std::path::Path;

    fn settings(root: &Path) -> Settings {
        Settings {
            paths: vec![root.display().to_string()],
            include: DEFAULT_INCLUDES
                .iter()
                .map(|value| value.to_string())
                .collect(),
            exclude: Vec::new(),
            cache_dir: root.join(".cache"),
            cache_ttl: CacheTtl::Seconds(0),
            cache_enabled: false,
            refresh_cache: false,
            update: false,
            latest_hash: false,
            missing_ref: MissingRefPolicy::Warn,
            include_prereleases: false,
            preserve_major: true,
            check: false,
            dry_run: true,
            diff: false,
            format: OutputFormat::Human,
            quiet: false,
            verbose: false,
            color: ColorChoice::Auto,
            github_token_present: false,
            github_api_url: "https://api.github.com".to_string(),
            strict_schema: false,
            schema_validation: false,
        }
    }

    #[test]
    fn discovers_default_files_under_explicit_directory() {
        let temp = tempfile::tempdir().unwrap();
        let workflow = temp.path().join(".github/workflows/ci.yml");
        let action = temp.path().join(".github/actions/build/action.yaml");
        fs::create_dir_all(workflow.parent().unwrap()).unwrap();
        fs::create_dir_all(action.parent().unwrap()).unwrap();
        fs::write(&workflow, "name: ci").unwrap();
        fs::write(&action, "name: build").unwrap();
        fs::write(temp.path().join("other.yml"), "nope").unwrap();

        let files = discover_files(&settings(temp.path())).unwrap();
        assert_eq!(files, vec![action, workflow]);
    }

    #[test]
    fn explicit_file_scans_even_when_outside_default_includes() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("custom.yml");
        fs::write(&file, "jobs: {}").unwrap();
        let mut settings = settings(temp.path());
        settings.paths = vec![file.display().to_string()];

        let files = discover_files(&settings).unwrap();
        assert_eq!(files, vec![file]);
    }

    #[test]
    fn exclude_wins_and_output_is_sorted_deduped() {
        let temp = tempfile::tempdir().unwrap();
        let a = temp.path().join(".github/workflows/a.yml");
        let b = temp.path().join(".github/workflows/b.yml");
        fs::create_dir_all(a.parent().unwrap()).unwrap();
        fs::write(&a, "name: a").unwrap();
        fs::write(&b, "name: b").unwrap();
        let mut settings = settings(temp.path());
        settings.paths = vec![
            temp.path().display().to_string(),
            temp.path()
                .join(".github/workflows/*.yml")
                .display()
                .to_string(),
        ];
        settings.exclude = vec![".github/workflows/b.yml".to_string()];

        let files = discover_files(&settings).unwrap();
        assert_eq!(files, vec![a]);
    }
}
