use crate::action_ref::ReferenceKind;
use crate::cache::{CacheKeyParts, CacheLookup, CacheReport, CacheState, cache_key};
use crate::cli::MissingRefPolicy;
use crate::config::Settings;
use crate::report::UpdateReport;
use crate::scanner::{Diagnostic, DiagnosticCategory, ReferenceReport};
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
    fn fetch_tags(&self, owner: &str, repo: &str, etag: Option<&str>) -> Result<TagFetch>;
}

#[derive(Debug, Clone)]
pub enum TagFetch {
    Fresh {
        tags: Vec<RemoteTag>,
        etag: Option<String>,
    },
    NotModified,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct TagCacheValue {
    provider: String,
    api_host: String,
    auth_fingerprint: String,
    etag: Option<String>,
    tags: Vec<RemoteTag>,
}

pub struct GitHubRestProvider {
    api_url: String,
    token: Option<String>,
    fallback: Option<GitLsRemoteProvider>,
}

impl GitHubRestProvider {
    pub fn new(settings: &Settings) -> Self {
        let api_url = settings.github_api_url.trim_end_matches('/').to_string();
        let fallback = if api_url == "https://api.github.com" && settings.github_token.is_none() {
            Some(GitLsRemoteProvider)
        } else {
            None
        };
        Self {
            api_url,
            token: settings.github_token.clone(),
            fallback,
        }
    }
}

#[derive(Debug, Deserialize)]
struct GitHubTag {
    name: String,
    commit: GitHubCommit,
}

#[derive(Debug, Deserialize)]
struct GitHubCommit {
    sha: String,
}

impl TagProvider for GitHubRestProvider {
    fn fetch_tags(&self, owner: &str, repo: &str, etag: Option<&str>) -> Result<TagFetch> {
        let mut url = format!("{}/repos/{owner}/{repo}/tags?per_page=100", self.api_url);
        let mut tags = Vec::new();
        let mut response_etag = None;
        let mut first_page = true;

        loop {
            let mut request = ureq::get(&url)
                .header("Accept", "application/vnd.github+json")
                .header(
                    "User-Agent",
                    concat!("gh-actions-updater/", env!("CARGO_PKG_VERSION")),
                );
            if let Some(token) = &self.token {
                request = request.header("Authorization", format!("Bearer {token}"));
            }
            if first_page && let Some(etag) = etag {
                request = request.header("If-None-Match", etag);
            }

            let response = match request.call() {
                Ok(response) if response.status() == 304 => return Ok(TagFetch::NotModified),
                Ok(response) => response,
                Err(ureq::Error::StatusCode(304)) => return Ok(TagFetch::NotModified),
                Err(error) => {
                    let Some(fallback) = &self.fallback else {
                        return Err(error).with_context(|| {
                            format!("GitHub REST metadata lookup failed for {owner}/{repo}")
                        });
                    };
                    return fallback.fetch_tags(owner, repo, None).with_context(|| {
                        format!("GitHub REST metadata lookup failed for {owner}/{repo}: {error}")
                    });
                }
            };
            if first_page {
                response_etag = response
                    .headers()
                    .get("etag")
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_string);
            }
            let next_url = response
                .headers()
                .get("link")
                .and_then(|value| value.to_str().ok())
                .and_then(next_link);
            let page_tags: Vec<GitHubTag> =
                response.into_body().read_json().with_context(|| {
                    format!("failed to parse GitHub tags response for {owner}/{repo}")
                })?;
            tags.extend(page_tags.into_iter().map(|tag| RemoteTag {
                name: tag.name,
                sha: tag.commit.sha,
            }));

            let Some(next_url) = next_url else {
                break;
            };
            url = next_url;
            first_page = false;
        }

        Ok(TagFetch::Fresh {
            tags,
            etag: response_etag,
        })
    }
}

pub struct GitLsRemoteProvider;

impl TagProvider for GitLsRemoteProvider {
    fn fetch_tags(&self, owner: &str, repo: &str, _etag: Option<&str>) -> Result<TagFetch> {
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

        Ok(TagFetch::Fresh {
            tags: parse_ls_remote_tags(&String::from_utf8_lossy(&output.stdout))?,
            etag: None,
        })
    }
}

pub fn resolve_updates(
    settings: &Settings,
    cache: CacheState,
    references: &[ReferenceReport],
) -> Result<MetadataResolution> {
    resolve_updates_with_provider(
        settings,
        cache,
        references,
        &GitHubRestProvider::new(settings),
    )
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
                category: DiagnosticCategory::General,
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
                ref_span: reference.ref_span,
                rewrite_supported: reference.rewrite_supported,
                rewrite_reason: reference.rewrite_reason.clone(),
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
    let auth_fingerprint = auth_fingerprint(settings.github_token.as_deref());
    let key = cache_key(CacheKeyParts {
        api_host: &settings.github_api_url,
        owner,
        repo,
        lookup_mode: "tags",
        auth_fingerprint: &auth_fingerprint,
    });

    match cache.read_json::<TagCacheValue>(&key)? {
        CacheLookup::Fresh(value) => {
            return Ok(TagLoad {
                tags: value.tags,
                warning: None,
            });
        }
        CacheLookup::Stale(value) => {
            match provider.fetch_tags(owner, repo, value.etag.as_deref()) {
                Ok(TagFetch::Fresh { tags, etag }) => {
                    cache.write_json(
                        &key,
                        &TagCacheValue {
                            provider: "github-rest-or-fallback".to_string(),
                            api_host: settings.github_api_url.clone(),
                            auth_fingerprint: auth_fingerprint.clone(),
                            etag,
                            tags: tags.clone(),
                        },
                    )?;
                    return Ok(TagLoad {
                        tags,
                        warning: None,
                    });
                }
                Ok(TagFetch::NotModified) => {
                    cache.write_json(&key, &value)?;
                    return Ok(TagLoad {
                        tags: value.tags,
                        warning: None,
                    });
                }
                Err(error) if !settings.check && !settings.update => {
                    return Ok(TagLoad {
                        tags: value.tags,
                        warning: Some(format!(
                            "using stale metadata for {owner}/{repo} after refresh failed: {error}"
                        )),
                    });
                }
                Err(error) => return Err(error),
            }
        }
        CacheLookup::Corrupt => {}
        CacheLookup::Miss => {}
    }

    let (tags, etag) = match provider.fetch_tags(owner, repo, None)? {
        TagFetch::Fresh { tags, etag } => (tags, etag),
        TagFetch::NotModified => {
            return Err(anyhow!(
                "metadata provider returned not-modified for {owner}/{repo} without cached tags"
            ));
        }
    };
    cache.write_json(
        &key,
        &TagCacheValue {
            provider: "github-rest-or-fallback".to_string(),
            api_host: settings.github_api_url.clone(),
            auth_fingerprint,
            etag,
            tags: tags.clone(),
        },
    )?;
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
        category: DiagnosticCategory::General,
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

fn auth_fingerprint(token: Option<&str>) -> String {
    let Some(token) = token else {
        return "anonymous".to_string();
    };
    let mut hasher = blake3::Hasher::new();
    hasher.update(token.as_bytes());
    format!("token:{}", hasher.finalize().to_hex())
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

fn next_link(link_header: &str) -> Option<String> {
    for link in link_header.split(',') {
        let mut parts = link.split(';').map(str::trim);
        let Some(url) = parts.next() else {
            continue;
        };
        if parts.any(|part| part == r#"rel="next""#) {
            return url
                .strip_prefix('<')
                .and_then(|value| value.strip_suffix('>'))
                .map(str::to_string);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{
        MetadataResolution, RemoteTag, TagFetch, TagProvider, exit_code_for_resolution,
        parse_ls_remote_tags, resolve_updates_with_provider,
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
        fn fetch_tags(&self, _owner: &str, _repo: &str, _etag: Option<&str>) -> Result<TagFetch> {
            self.calls.set(self.calls.get() + 1);
            Ok(TagFetch::Fresh {
                tags: self.tags.clone(),
                etag: Some("etag-test".to_string()),
            })
        }
    }

    impl TagProvider for FailingProvider {
        fn fetch_tags(&self, _owner: &str, _repo: &str, _etag: Option<&str>) -> Result<TagFetch> {
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
            github_token: None,
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
            ref_span: None,
            rewrite_supported: false,
            rewrite_reason: None,
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
                "value": {
                    "provider": "test",
                    "api_host": "https://api.github.com",
                    "auth_fingerprint": "anonymous",
                    "etag": "etag-test",
                    "tags": [tag("v4"), tag("v4.2.0")]
                }
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

    #[test]
    fn extracts_next_link_from_github_header() {
        let link = r#"<https://api.github.com/repos/o/r/tags?page=2>; rel="next", <https://api.github.com/repos/o/r/tags?page=4>; rel="last""#;
        assert_eq!(
            super::next_link(link).as_deref(),
            Some("https://api.github.com/repos/o/r/tags?page=2")
        );
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
