use crate::config::Settings;
use ahash::AHashSet;
use anyhow::{Context, Result};
use glob::glob;
use globset::{Glob, GlobSet, GlobSetBuilder};
use ignore::{
    DirEntry, Match, WalkBuilder,
    gitignore::{Gitignore, GitignoreBuilder},
};
use rayon::prelude::*;
use std::env;
use std::path::{Path, PathBuf};

pub fn discover_files(settings: &Settings) -> Result<Vec<PathBuf>> {
    let include = build_globset(&settings.include)?;
    let exclude = build_globset(&settings.exclude)?;
    let roots: Vec<PathBuf> = if settings.paths.is_empty() {
        vec![PathBuf::from(".")]
    } else {
        settings.paths.iter().map(PathBuf::from).collect()
    };

    let discovered: Result<Vec<Vec<PathBuf>>> = roots
        .par_iter()
        .map(|root| discover_root(root, &include, &exclude, settings.recursive))
        .collect();
    let mut files: Vec<PathBuf> = discovered?.into_iter().flatten().collect();

    let mut seen = AHashSet::with_capacity(files.len());
    files.retain(|path| seen.insert(normalize(path)));
    files.sort();
    Ok(files)
}

fn discover_root(
    root: &Path,
    include: &GlobSet,
    exclude: &GlobSet,
    recursive: bool,
) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    if root.is_file() {
        push_explicit_file(&mut files, root, exclude);
        return Ok(files);
    }

    if !root.exists() && has_glob_meta(root) {
        for entry in glob(&root.to_string_lossy())
            .with_context(|| format!("invalid glob {}", root.display()))?
        {
            let path = entry.with_context(|| format!("failed to read glob {}", root.display()))?;
            if path.is_file() {
                push_explicit_file(&mut files, &path, exclude);
            }
        }
        return Ok(files);
    }

    let match_base = match_base(root);
    let parent_ignores = build_parent_ignores(root)?;
    let root_for_matching = absolutize(root)?;
    let mut builder = WalkBuilder::new(root);
    builder
        .hidden(false)
        .parents(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .require_git(false);
    if !parent_ignores.is_empty() {
        builder.filter_entry(move |entry| {
            !is_ignored_by_parent(entry, &root_for_matching, &parent_ignores)
        });
    }
    let walker = builder.build();

    for entry in walker {
        let entry = entry.with_context(|| format!("failed to walk {}", root.display()))?;
        if entry
            .file_type()
            .is_some_and(|file_type| file_type.is_file())
        {
            push_if_match(
                &mut files,
                entry.path(),
                &match_base,
                include,
                exclude,
                recursive,
            );
        }
    }
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
    recursive: bool,
) {
    let root_relative = normalize(path.strip_prefix(base).unwrap_or(path));
    let normalized = normalize(path);
    if is_candidate_file(path)
        && (include.is_match(&root_relative)
            || (recursive
                && (matches_glob_or_suffix(include, &root_relative)
                    || matches_glob_or_suffix(include, &normalized))))
        && !exclude.is_match(&root_relative)
        && !matches_glob_or_suffix(exclude, &normalized)
    {
        files.push(path.to_path_buf());
    }
}

fn match_base(root: &Path) -> PathBuf {
    if root.file_name().and_then(|name| name.to_str()) == Some(".github") {
        root.parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    } else {
        root.to_path_buf()
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

#[derive(Clone, Debug)]
struct ParentIgnore {
    matcher: Gitignore,
    root_ignore_globs: AHashSet<String>,
}

fn build_parent_ignores(root: &Path) -> Result<Vec<ParentIgnore>> {
    let root = absolutize(root)?;
    let mut ignores = Vec::new();

    for parent in root.ancestors().skip(1) {
        let ignore_file = parent.join(".gitignore");
        if !ignore_file.is_file() {
            continue;
        }

        let mut builder = GitignoreBuilder::new(parent);
        if let Some(error) = builder.add(&ignore_file) {
            return Err(error).with_context(|| format!("failed to read {}", ignore_file.display()));
        }
        let matcher = builder
            .build()
            .with_context(|| format!("failed to build ignore matcher for {}", parent.display()))?;
        let mut root_ignore_globs = AHashSet::new();
        if let Match::Ignore(glob) = matcher.matched(&root, true) {
            root_ignore_globs.insert(glob.original().to_string());
        }
        ignores.push(ParentIgnore {
            matcher,
            root_ignore_globs,
        });
    }

    Ok(ignores)
}

fn is_ignored_by_parent(entry: &DirEntry, root: &Path, parent_ignores: &[ParentIgnore]) -> bool {
    let path = absolutize(entry.path()).unwrap_or_else(|_| entry.path().to_path_buf());
    if path == root {
        return false;
    }

    let is_dir = entry
        .file_type()
        .is_some_and(|file_type| file_type.is_dir());
    for parent_ignore in parent_ignores {
        match parent_ignore.matcher.matched(&path, is_dir) {
            Match::Ignore(glob) if !parent_ignore.root_ignore_globs.contains(glob.original()) => {
                return true;
            }
            Match::Ignore(_) | Match::Whitelist(_) => return false,
            Match::None => {}
        }
    }

    false
}

fn absolutize(path: &Path) -> Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        env::current_dir()
            .context("failed to resolve current directory")?
            .join(path)
    };
    Ok(absolute.components().collect())
}

#[cfg(test)]
mod tests {
    use super::discover_files;
    use crate::cli::{ColorChoice, MissingRefPolicy, OutputFormat, PinStyle};
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
            pin_style: PinStyle::Preserve,
            update_exclude: Vec::new(),
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
    fn explicit_dot_github_directory_scans_workflows() {
        let temp = tempfile::tempdir().unwrap();
        let workflow = temp.path().join(".github/workflows/ci.yml");
        fs::create_dir_all(workflow.parent().unwrap()).unwrap();
        fs::write(&workflow, "name: ci").unwrap();
        let mut settings = settings(temp.path());
        settings.paths = vec![temp.path().join(".github").display().to_string()];

        let files = discover_files(&settings).unwrap();
        assert_eq!(files, vec![workflow]);
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

    #[test]
    fn recursive_directory_scan_skips_ignored_nested_repo_actions_surface() {
        let temp = tempfile::tempdir().unwrap();
        let workflow = temp.path().join("nested-repo/.github/workflows/ci.yml");
        let action = temp
            .path()
            .join("nested-repo/.github/actions/build/action.yml");
        fs::create_dir_all(workflow.parent().unwrap()).unwrap();
        fs::create_dir_all(action.parent().unwrap()).unwrap();
        fs::write(&workflow, "name: ci").unwrap();
        fs::write(&action, "name: build").unwrap();
        fs::write(temp.path().join(".gitignore"), "/nested-repo/\n").unwrap();
        let mut settings = settings(temp.path());
        settings.recursive = true;

        let files = discover_files(&settings).unwrap();

        assert!(files.is_empty());
    }

    #[test]
    fn recursive_directory_scan_allows_explicit_ignored_nested_repo() {
        let temp = tempfile::tempdir().unwrap();
        let nested = temp.path().join("nested-repo");
        let workflow = nested.join(".github/workflows/ci.yml");
        let action = nested.join(".github/actions/build/action.yml");
        fs::create_dir_all(workflow.parent().unwrap()).unwrap();
        fs::create_dir_all(action.parent().unwrap()).unwrap();
        fs::write(&workflow, "name: ci").unwrap();
        fs::write(&action, "name: build").unwrap();
        fs::write(temp.path().join(".gitignore"), "/nested-repo/\n").unwrap();
        let mut settings = settings(&nested);
        settings.recursive = true;

        let files = discover_files(&settings).unwrap();

        assert_eq!(files, vec![action, workflow]);
    }

    #[test]
    fn recursive_directory_scan_respects_parent_gitignore_for_explicit_subdirectory() {
        let temp = tempfile::tempdir().unwrap();
        let subdir = temp.path().join("subdir");
        let workflow = subdir.join(".github/workflows/ci.yml");
        let ignored_workflow = subdir.join("vendor/pkg/.github/workflows/ci.yml");
        fs::create_dir_all(workflow.parent().unwrap()).unwrap();
        fs::create_dir_all(ignored_workflow.parent().unwrap()).unwrap();
        fs::write(&workflow, "name: ci").unwrap();
        fs::write(&ignored_workflow, "name: ignored").unwrap();
        fs::write(temp.path().join(".gitignore"), "subdir/vendor/\n").unwrap();
        let mut settings = settings(&subdir);
        settings.recursive = true;

        let files = discover_files(&settings).unwrap();

        assert_eq!(files, vec![workflow]);
    }

    #[test]
    fn default_directory_scan_does_not_recurse_into_nested_dot_github() {
        let temp = tempfile::tempdir().unwrap();
        let root_workflow = temp.path().join(".github/workflows/root.yml");
        let nested_workflow = temp
            .path()
            .join("vendor/project/.github/workflows/nested.yml");
        fs::create_dir_all(root_workflow.parent().unwrap()).unwrap();
        fs::create_dir_all(nested_workflow.parent().unwrap()).unwrap();
        fs::write(&root_workflow, "name: root").unwrap();
        fs::write(&nested_workflow, "name: nested").unwrap();

        let files = discover_files(&settings(temp.path())).unwrap();
        assert_eq!(files, vec![root_workflow]);
    }

    #[test]
    fn recursive_directory_scan_includes_nested_dot_github() {
        let temp = tempfile::tempdir().unwrap();
        let root_workflow = temp.path().join(".github/workflows/root.yml");
        let nested_workflow = temp
            .path()
            .join("tools/project/.github/workflows/nested.yml");
        fs::create_dir_all(root_workflow.parent().unwrap()).unwrap();
        fs::create_dir_all(nested_workflow.parent().unwrap()).unwrap();
        fs::write(&root_workflow, "name: root").unwrap();
        fs::write(&nested_workflow, "name: nested").unwrap();
        let mut settings = settings(temp.path());
        settings.recursive = true;

        let files = discover_files(&settings).unwrap();
        assert_eq!(files, vec![root_workflow, nested_workflow]);
    }

    #[test]
    fn recursive_directory_scan_skips_ignored_dependency_and_vendor_trees() {
        let temp = tempfile::tempdir().unwrap();
        let root_workflow = temp.path().join(".github/workflows/root.yml");
        let ignored_workflows = [
            temp.path()
                .join("node_modules/pkg/.github/workflows/ci.yml"),
            temp.path().join("vendor/pkg/.github/workflows/ci.yml"),
            temp.path()
                .join("packages/ruby/vendor/bundle/pkg/.github/workflows/ci.yml"),
        ];
        fs::create_dir_all(root_workflow.parent().unwrap()).unwrap();
        fs::write(&root_workflow, "name: root").unwrap();
        for workflow in &ignored_workflows {
            fs::create_dir_all(workflow.parent().unwrap()).unwrap();
            fs::write(workflow, "name: ignored").unwrap();
        }
        fs::write(
            temp.path().join(".gitignore"),
            "node_modules/\nvendor/\npackages/ruby/vendor/bundle/\n",
        )
        .unwrap();
        let mut settings = settings(temp.path());
        settings.recursive = true;

        let files = discover_files(&settings).unwrap();

        assert_eq!(files, vec![root_workflow]);
    }
}
