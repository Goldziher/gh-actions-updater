use crate::action_ref::ReferenceKind;
use crate::cache::{CacheKeyParts, CacheLookup, CacheReport, CacheState, cache_key};
use crate::cli::MissingRefPolicy;
use crate::config::Settings;
use crate::report::UpdateReport;
use crate::scanner::{Diagnostic, ReferenceReport};
use ahash::AHashMap;
use anyhow::{Context, Result, anyhow};
use semver::Version;
use serde::{Deserialize, Serialize};
use std::process::Command;

#[derive(Debug)]
pub struct MetadataResolution {
    pub updates: Vec<UpdateReport>,
    pub diagnostics: Vec<Diagnostic>,
    pub cache: CacheReport,
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq, Serialize)]
pub struct RemoteTag {
    pub name: String,
    pub sha: String,
}

pub trait TagProvider {
    fn fetch_tags(&self, owner: &str, repo: &str) -> Result<Vec<RemoteTag>>;
}

pub struct GitLsRemoteProvider;

impl TagProvider for GitLsRemoteProvider {
    fn fetch_tags(&self, owner: &str, repo: &str) -> Result<Vec<RemoteTag>> {
        let url = format!("https://github.com/{owner}/{repo}.git");
        let output = Command::new("git")
            .args(["ls-remote", "--tags", "--refs", &url])
            .output()
            .with_context(|| format!("failed to run git ls-remote for {owner}/{repo}"))?;

        if !output.status.success() {
            return Err(anyhow!(
                "git ls-remote failed for {owner}/{repo}: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }

        parse_ls_remote_tags(&String::from_utf8_lossy(&output.stdout))
    }
}

pub fn resolve_updates(
    settings: &Settings,
    cache: CacheState,
    references: &[ReferenceReport],
) -> Result<MetadataResolution> {
    resolve_updates_with_provider(settings, cache, references, &GitLsRemoteProvider)
}

pub fn resolve_updates_with_provider(
    settings: &Settings,
    mut cache: CacheState,
    references: &[ReferenceReport],
    provider: &impl TagProvider,
) -> Result<MetadataResolution> {
    let mut diagnostics = Vec::new();
    let mut updates = Vec::new();
    let mut tags_by_repo: AHashMap<(String, String), TagLoad> = AHashMap::new();

    for reference in references {
        if !matches!(
            reference.parsed.kind,
            ReferenceKind::RemoteAction | ReferenceKind::ReusableWorkflow
        ) {
            continue;
        }

        let (Some(owner), Some(repo), Some(current)) = (
            reference.parsed.owner.as_deref(),
            reference.parsed.repo.as_deref(),
            reference.parsed.ref_name.as_deref(),
        ) else {
            continue;
        };

        let repo_key = (owner.to_string(), repo.to_string());
        if !tags_by_repo.contains_key(&repo_key) {
            let tags = load_tags(settings, &mut cache, provider, owner, repo)?;
            tags_by_repo.insert(repo_key.clone(), tags);
        }

        let tag_load = tags_by_repo
            .get(&repo_key)
            .expect("repo tags should be inserted before lookup");
        if let Some(warning) = &tag_load.warning {
            diagnostics.push(Diagnostic {
                file: reference.file.clone(),
                line: Some(reference.line),
                message: warning.clone(),
            });
        }

        if !reference.parsed.updatable {
            continue;
        }

        let tags = &tag_load.tags;
        let decision = select_update_target(settings, current, tags);
        let Some(target) = decision.target else {
            if decision.current_missing {
                handle_missing_ref(settings, reference, &mut diagnostics)?;
            }
            continue;
        };

        if target.name != current
            && (!decision.current_missing || settings.missing_ref == MissingRefPolicy::Fallback)
        {
            updates.push(UpdateReport {
                file: reference.file.clone(),
                line: reference.line,
                current: current.to_string(),
                target: Some(target.name),
            });
        }
    }

    Ok(MetadataResolution {
        updates,
        diagnostics,
        cache: cache.report,
    })
}

fn load_tags(
    settings: &Settings,
    cache: &mut CacheState,
    provider: &impl TagProvider,
    owner: &str,
    repo: &str,
) -> Result<TagLoad> {
    let key = cache_key(CacheKeyParts {
        api_host: &settings.github_api_url,
        owner,
        repo,
        lookup_mode: "tags",
        auth_fingerprint: "anonymous",
    });

    match cache.read_json::<Vec<RemoteTag>>(&key)? {
        CacheLookup::Fresh(tags) => {
            return Ok(TagLoad {
                tags,
                warning: None,
            });
        }
        CacheLookup::Stale(tags) => match provider.fetch_tags(owner, repo) {
            Ok(fresh) => {
                cache.write_json(&key, &fresh)?;
                return Ok(TagLoad {
                    tags: fresh,
                    warning: None,
                });
            }
            Err(error) if !settings.check && !settings.update => {
                return Ok(TagLoad {
                    tags,
                    warning: Some(format!(
                        "using stale metadata for {owner}/{repo} after refresh failed: {error}"
                    )),
                });
            }
            Err(error) => return Err(error),
        },
        CacheLookup::Corrupt => {}
        CacheLookup::Miss => {}
    }

    let tags = provider.fetch_tags(owner, repo)?;
    cache.write_json(&key, &tags)?;
    Ok(TagLoad {
        tags,
        warning: None,
    })
}

#[derive(Debug, Clone)]
struct TagLoad {
    tags: Vec<RemoteTag>,
    warning: Option<String>,
}

fn handle_missing_ref(
    settings: &Settings,
    reference: &ReferenceReport,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<()> {
    if settings.missing_ref == MissingRefPolicy::Ignore {
        return Ok(());
    }

    let message = format!("remote ref no longer exists: {}", reference.raw);
    if settings.missing_ref == MissingRefPolicy::Error && !settings.check {
        return Err(anyhow!(message));
    }

    diagnostics.push(Diagnostic {
        file: reference.file.clone(),
        line: Some(reference.line),
        message,
    });
    Ok(())
}

#[derive(Debug)]
struct TargetDecision {
    target: Option<RemoteTag>,
    current_missing: bool,
}

fn select_update_target(settings: &Settings, current: &str, tags: &[RemoteTag]) -> TargetDecision {
    let Some(current_version) = parse_version_tag(current) else {
        return TargetDecision {
            target: None,
            current_missing: false,
        };
    };
    let current_exists = tags.iter().any(|tag| tag.name == current);
    if !current_exists && settings.missing_ref != MissingRefPolicy::Fallback {
        return TargetDecision {
            target: None,
            current_missing: true,
        };
    }

    let target = tags
        .iter()
        .filter_map(|tag| parse_version_tag(&tag.name).map(|version| (tag, version)))
        .filter(|(_, version)| !settings.preserve_major || version.major == current_version.major)
        .filter(|(_, version)| settings.include_prereleases || version.pre.is_empty())
        .max_by(|(_, left), (_, right)| left.cmp(right))
        .map(|(tag, _)| tag.clone());

    TargetDecision {
        target,
        current_missing: !current_exists,
    }
}

pub fn exit_code_for_resolution(
    settings: &Settings,
    resolution: &MetadataResolution,
) -> Option<i32> {
    if settings.check && !resolution.updates.is_empty() {
        return Some(1);
    }

    if settings.check
        && settings.missing_ref == MissingRefPolicy::Error
        && resolution
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("remote ref no longer exists"))
    {
        return Some(1);
    }

    None
}

fn parse_version_tag(tag: &str) -> Option<Version> {
    let version = tag.strip_prefix('v').unwrap_or(tag);
    let parts: Vec<_> = version.split('.').collect();
    match parts.as_slice() {
        [major] => Version::parse(&format!("{major}.0.0")).ok(),
        [major, minor] => Version::parse(&format!("{major}.{minor}.0")).ok(),
        [major, minor, patch] => Version::parse(&format!("{major}.{minor}.{patch}")).ok(),
        _ => Version::parse(version).ok(),
    }
}

fn parse_ls_remote_tags(output: &str) -> Result<Vec<RemoteTag>> {
    let mut tags = Vec::new();
    for line in output.lines() {
        let Some((sha, reference)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        let Some(name) = reference.trim().strip_prefix("refs/tags/") else {
            continue;
        };
        tags.push(RemoteTag {
            name: name.to_string(),
            sha: sha.to_string(),
        });
    }
    Ok(tags)
}

#[cfg(test)]
mod tests {
    use super::{
        MetadataResolution, RemoteTag, TagProvider, exit_code_for_resolution, parse_ls_remote_tags,
        resolve_updates_with_provider,
    };
    use crate::cache::{CacheKeyParts, CacheState, cache_key};
    use crate::cli::{ColorChoice, MissingRefPolicy, OutputFormat};
    use crate::config::{CacheTtl, Settings};
    use crate::scanner::ReferenceReport;
    use anyhow::Result;
    use std::cell::Cell;
    use std::path::Path;

    struct FakeProvider {
        tags: Vec<RemoteTag>,
        calls: Cell<usize>,
    }

    struct FailingProvider {
        calls: Cell<usize>,
    }

    impl TagProvider for FakeProvider {
        fn fetch_tags(&self, _owner: &str, _repo: &str) -> Result<Vec<RemoteTag>> {
            self.calls.set(self.calls.get() + 1);
            Ok(self.tags.clone())
        }
    }

    impl TagProvider for FailingProvider {
        fn fetch_tags(&self, _owner: &str, _repo: &str) -> Result<Vec<RemoteTag>> {
            self.calls.set(self.calls.get() + 1);
            anyhow::bail!("network unavailable")
        }
    }

    fn settings(root: &Path) -> Settings {
        Settings {
            paths: Vec::new(),
            include: Vec::new(),
            exclude: Vec::new(),
            cache_dir: root.join("cache"),
            cache_ttl: CacheTtl::Seconds(3600),
            cache_enabled: true,
            refresh_cache: false,
            update: false,
            latest_hash: false,
            missing_ref: MissingRefPolicy::Warn,
            include_prereleases: false,
            preserve_major: true,
            check: false,
            dry_run: true,
            diff: false,
            format: OutputFormat::Json,
            quiet: false,
            verbose: false,
            color: ColorChoice::Auto,
            github_token_present: false,
            github_api_url: "https://api.github.com".to_string(),
            strict_schema: false,
            schema_validation: false,
        }
    }

    fn reference(raw: &str) -> ReferenceReport {
        ReferenceReport {
            file: ".github/workflows/ci.yml".to_string(),
            line: 7,
            column: 9,
            raw: raw.to_string(),
            parsed: crate::action_ref::parse_uses(raw),
        }
    }

    fn tag(name: &str) -> RemoteTag {
        RemoteTag {
            name: name.to_string(),
            sha: format!("sha-{name}"),
        }
    }

    #[test]
    fn selects_latest_same_major_tag() {
        let temp = tempfile::tempdir().unwrap();
        let settings = settings(temp.path());
        let provider = FakeProvider {
            tags: vec![tag("v3"), tag("v4"), tag("v4.1.0"), tag("v5")],
            calls: Cell::new(0),
        };

        let resolution = resolve_updates_with_provider(
            &settings,
            CacheState::prepare(&settings).unwrap(),
            &[reference("actions/checkout@v4")],
            &provider,
        )
        .unwrap();

        assert_eq!(resolution.updates.len(), 1);
        assert_eq!(resolution.updates[0].target.as_deref(), Some("v4.1.0"));
        assert_eq!(provider.calls.get(), 1);
    }

    #[test]
    fn reuses_cached_tags() {
        let temp = tempfile::tempdir().unwrap();
        let settings = settings(temp.path());
        let provider = FakeProvider {
            tags: vec![tag("v4"), tag("v4.2.0")],
            calls: Cell::new(0),
        };

        let cache = CacheState::prepare(&settings).unwrap();
        let first = resolve_updates_with_provider(
            &settings,
            cache,
            &[reference("actions/checkout@v4")],
            &provider,
        )
        .unwrap();
        assert_eq!(first.cache.misses, 1);

        let second = resolve_updates_with_provider(
            &settings,
            CacheState::prepare(&settings).unwrap(),
            &[reference("actions/checkout@v4")],
            &provider,
        )
        .unwrap();
        assert_eq!(second.cache.fresh_hits, 1);
        assert_eq!(provider.calls.get(), 1);
    }

    #[test]
    fn corrupt_cache_is_replaced() {
        let temp = tempfile::tempdir().unwrap();
        let settings = settings(temp.path());
        std::fs::create_dir_all(&settings.cache_dir).unwrap();
        let key = tags_cache_key("actions", "checkout");
        std::fs::write(settings.cache_dir.join(format!("{key}.json")), b"not json").unwrap();
        let provider = FakeProvider {
            tags: vec![tag("v4"), tag("v4.2.0")],
            calls: Cell::new(0),
        };

        let resolution = resolve_updates_with_provider(
            &settings,
            CacheState::prepare(&settings).unwrap(),
            &[reference("actions/checkout@v4")],
            &provider,
        )
        .unwrap();

        assert_eq!(resolution.cache.refreshes, 1);
        assert_eq!(resolution.updates[0].target.as_deref(), Some("v4.2.0"));
        assert_eq!(provider.calls.get(), 1);
    }

    #[test]
    fn stale_cache_fallback_reports_warning() {
        let temp = tempfile::tempdir().unwrap();
        let mut settings = settings(temp.path());
        settings.cache_ttl = CacheTtl::Seconds(0);
        std::fs::create_dir_all(&settings.cache_dir).unwrap();
        let key = tags_cache_key("actions", "checkout");
        std::fs::write(
            settings.cache_dir.join(format!("{key}.json")),
            serde_json::json!({
                "fetched_at": 1,
                "value": [tag("v4"), tag("v4.2.0")]
            })
            .to_string(),
        )
        .unwrap();
        let provider = FailingProvider {
            calls: Cell::new(0),
        };

        let resolution = resolve_updates_with_provider(
            &settings,
            CacheState::prepare(&settings).unwrap(),
            &[reference("actions/checkout@v4")],
            &provider,
        )
        .unwrap();

        assert_eq!(resolution.cache.stale_hits, 1);
        assert_eq!(resolution.updates[0].target.as_deref(), Some("v4.2.0"));
        assert!(
            resolution.diagnostics[0]
                .message
                .contains("using stale metadata")
        );
    }

    #[test]
    fn no_cache_does_not_record_cache_misses() {
        let temp = tempfile::tempdir().unwrap();
        let mut settings = settings(temp.path());
        settings.cache_enabled = false;
        let provider = FakeProvider {
            tags: vec![tag("v4")],
            calls: Cell::new(0),
        };

        let resolution = resolve_updates_with_provider(
            &settings,
            CacheState::prepare(&settings).unwrap(),
            &[reference("actions/checkout@v4")],
            &provider,
        )
        .unwrap();

        assert!(!resolution.cache.enabled);
        assert_eq!(resolution.cache.misses, 0);
        assert_eq!(provider.calls.get(), 1);
    }

    #[test]
    fn reports_missing_current_ref() {
        let temp = tempfile::tempdir().unwrap();
        let settings = settings(temp.path());
        let provider = FakeProvider {
            tags: vec![tag("v4.1.0")],
            calls: Cell::new(0),
        };

        let resolution: MetadataResolution = resolve_updates_with_provider(
            &settings,
            CacheState::prepare(&settings).unwrap(),
            &[reference("actions/checkout@v4")],
            &provider,
        )
        .unwrap();

        assert!(resolution.updates.is_empty());
        assert_eq!(resolution.diagnostics.len(), 1);
    }

    #[test]
    fn missing_ref_fallback_counts_update() {
        let temp = tempfile::tempdir().unwrap();
        let mut settings = settings(temp.path());
        settings.missing_ref = MissingRefPolicy::Fallback;
        let provider = FakeProvider {
            tags: vec![tag("v4.1.0")],
            calls: Cell::new(0),
        };

        let resolution = resolve_updates_with_provider(
            &settings,
            CacheState::prepare(&settings).unwrap(),
            &[reference("actions/checkout@v4")],
            &provider,
        )
        .unwrap();

        assert_eq!(resolution.updates.len(), 1);
        assert_eq!(resolution.updates[0].target.as_deref(), Some("v4.1.0"));
        assert!(resolution.diagnostics.is_empty());
    }

    #[test]
    fn missing_ref_error_fails_check() {
        let temp = tempfile::tempdir().unwrap();
        let mut settings = settings(temp.path());
        settings.check = true;
        settings.missing_ref = MissingRefPolicy::Error;
        let provider = FakeProvider {
            tags: vec![tag("v4.1.0")],
            calls: Cell::new(0),
        };

        let resolution = resolve_updates_with_provider(
            &settings,
            CacheState::prepare(&settings).unwrap(),
            &[reference("actions/checkout@v4")],
            &provider,
        )
        .unwrap();

        assert_eq!(exit_code_for_resolution(&settings, &resolution), Some(1));
    }

    #[test]
    fn prereleases_are_ignored_by_default() {
        let temp = tempfile::tempdir().unwrap();
        let settings = settings(temp.path());
        let provider = FakeProvider {
            tags: vec![tag("v4"), tag("v4.1.0-alpha.1")],
            calls: Cell::new(0),
        };

        let resolution = resolve_updates_with_provider(
            &settings,
            CacheState::prepare(&settings).unwrap(),
            &[reference("actions/checkout@v4")],
            &provider,
        )
        .unwrap();

        assert!(resolution.updates.is_empty());
    }

    #[test]
    fn parses_ls_remote_output() {
        let tags = parse_ls_remote_tags(
            "abc123\trefs/tags/v1\n\
             def456\trefs/tags/v1.2.0\n",
        )
        .unwrap();

        assert_eq!(
            tags,
            vec![
                tag_with_sha("v1", "abc123"),
                tag_with_sha("v1.2.0", "def456")
            ]
        );
    }

    fn tag_with_sha(name: &str, sha: &str) -> RemoteTag {
        RemoteTag {
            name: name.to_string(),
            sha: sha.to_string(),
        }
    }

    fn tags_cache_key(owner: &str, repo: &str) -> String {
        cache_key(CacheKeyParts {
            api_host: "https://api.github.com",
            owner,
            repo,
            lookup_mode: "tags",
            auth_fingerprint: "anonymous",
        })
    }
}
