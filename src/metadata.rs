use crate::action_ref::{RefKind, ReferenceKind};
use crate::cache::{CacheKeyParts, CacheLookup, CacheReport, CacheState, cache_key};
use crate::cli::{MissingRefPolicy, PinStyle, UpdateMode};
use crate::config::Settings;
use crate::report::UpdateReport;
use crate::scanner::{Diagnostic, DiagnosticCategory, DiagnosticCode, ReferenceReport};
use ahash::AHashMap;
use anyhow::{Context, Result, anyhow};
use globset::{Glob, GlobSet, GlobSetBuilder};
use semver::Version;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::process::Command;

#[derive(Debug)]
pub struct MetadataResolution {
    pub updates: Vec<UpdateReport>,
    pub diagnostics: Vec<Diagnostic>,
    pub cache: CacheReport,
    /// `(reference_index, resolved_kind)` pairs for refs whose `ref_kind`
    /// changed from `BranchOrUnknown` to `Branch` or `NonSemverTag` after
    /// upstream resolution. Callers apply these back to scanner output before
    /// reporting.
    pub resolved_ref_kinds: Vec<(usize, RefKind)>,
    pub has_metadata_failures: bool,
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq, Serialize)]
pub struct RemoteTag {
    pub name: String,
    pub sha: String,
}

pub trait TagProvider {
    fn fetch_tags(&self, owner: &str, repo: &str, etag: Option<&str>) -> Result<TagFetch>;

    fn fetch_branch(&self, _owner: &str, _repo: &str, _branch: &str, _etag: Option<&str>) -> Result<BranchFetch> {
        Err(anyhow!("branch metadata lookup is not supported by this provider"))
    }

    fn fetch_commit(&self, _owner: &str, _repo: &str, _sha: &str, _etag: Option<&str>) -> Result<CommitFetch> {
        Err(anyhow!("commit metadata lookup is not supported by this provider"))
    }
}

type RepositoryKey = (String, String);
type ReferenceKey = (String, String, String);
type Memoized<T> = RefCell<AHashMap<ReferenceKey, Result<T, String>>>;

struct MemoizingProvider<'a, P> {
    provider: &'a P,
    tags: RefCell<AHashMap<RepositoryKey, Result<TagFetch, String>>>,
    branches: Memoized<BranchFetch>,
    commits: Memoized<CommitFetch>,
}

impl<'a, P> MemoizingProvider<'a, P> {
    fn new(provider: &'a P) -> Self {
        Self {
            provider,
            tags: RefCell::new(AHashMap::new()),
            branches: RefCell::new(AHashMap::new()),
            commits: RefCell::new(AHashMap::new()),
        }
    }
}

impl<P: TagProvider> TagProvider for MemoizingProvider<'_, P> {
    fn fetch_tags(&self, owner: &str, repo: &str, etag: Option<&str>) -> Result<TagFetch> {
        let key = (owner.to_string(), repo.to_string());
        if let Some(result) = self.tags.borrow().get(&key) {
            return result.clone().map_err(|message| anyhow!(message));
        }
        let result = self
            .provider
            .fetch_tags(owner, repo, etag)
            .map_err(|error| error.to_string());
        self.tags.borrow_mut().insert(key, result.clone());
        result.map_err(|message| anyhow!(message))
    }

    fn fetch_branch(&self, owner: &str, repo: &str, branch: &str, etag: Option<&str>) -> Result<BranchFetch> {
        let key = (owner.to_string(), repo.to_string(), branch.to_string());
        if let Some(result) = self.branches.borrow().get(&key) {
            return result.clone().map_err(|message| anyhow!(message));
        }
        let result = self
            .provider
            .fetch_branch(owner, repo, branch, etag)
            .map_err(|error| error.to_string());
        self.branches.borrow_mut().insert(key, result.clone());
        result.map_err(|message| anyhow!(message))
    }

    fn fetch_commit(&self, owner: &str, repo: &str, sha: &str, etag: Option<&str>) -> Result<CommitFetch> {
        let key = (owner.to_string(), repo.to_string(), sha.to_string());
        if let Some(result) = self.commits.borrow().get(&key) {
            return result.clone().map_err(|message| anyhow!(message));
        }
        let result = self
            .provider
            .fetch_commit(owner, repo, sha, etag)
            .map_err(|error| error.to_string());
        self.commits.borrow_mut().insert(key, result.clone());
        result.map_err(|message| anyhow!(message))
    }
}

#[derive(Debug, Clone)]
pub enum TagFetch {
    Fresh { tags: Vec<RemoteTag>, etag: Option<String> },
    NotModified,
}

#[derive(Debug, Clone)]
pub enum CommitFetch {
    Fresh { exists: bool, etag: Option<String> },
    NotModified,
}

#[derive(Debug, Clone)]
pub enum BranchFetch {
    Fresh {
        exists: bool,
        sha: Option<String>,
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

#[derive(Debug, Clone, Deserialize, Serialize)]
struct CommitCacheValue {
    provider: String,
    api_host: String,
    auth_fingerprint: String,
    etag: Option<String>,
    sha: String,
    exists: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct BranchCacheValue {
    provider: String,
    api_host: String,
    auth_fingerprint: String,
    etag: Option<String>,
    branch: String,
    exists: bool,
    #[serde(default)]
    sha: Option<String>,
}

pub struct GitHubRestProvider {
    api_url: String,
    token: Option<String>,
    fallback: Option<GitLsRemoteProvider>,
    agent: ureq::Agent,
}

const MAX_GITHUB_REDIRECTS: u32 = 5;

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
            agent: ureq::Agent::config_builder()
                .https_only(true)
                .max_redirects(MAX_GITHUB_REDIRECTS)
                .redirect_auth_headers(ureq::config::RedirectAuthHeaders::SameHost)
                .build()
                .into(),
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
            let mut request = self
                .agent
                .get(&url)
                .header("Accept", "application/vnd.github+json")
                .header("User-Agent", concat!("gh-actions-updater/", env!("CARGO_PKG_VERSION")));
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
                        return Err(error)
                            .with_context(|| format!("GitHub REST metadata lookup failed for {owner}/{repo}"));
                    };
                    return fallback
                        .fetch_tags(owner, repo, None)
                        .with_context(|| format!("GitHub REST metadata lookup failed for {owner}/{repo}: {error}"));
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
            let page_tags: Vec<GitHubTag> = response
                .into_body()
                .read_json()
                .with_context(|| format!("failed to parse GitHub tags response for {owner}/{repo}"))?;
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

    fn fetch_branch(&self, owner: &str, repo: &str, branch: &str, etag: Option<&str>) -> Result<BranchFetch> {
        let encoded_branch = branch.replace('/', "%2F");
        let url = format!("{}/repos/{owner}/{repo}/branches/{encoded_branch}", self.api_url);
        let mut request = self
            .agent
            .get(&url)
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", concat!("gh-actions-updater/", env!("CARGO_PKG_VERSION")));
        if let Some(token) = &self.token {
            request = request.header("Authorization", format!("Bearer {token}"));
        }
        if let Some(etag) = etag {
            request = request.header("If-None-Match", etag);
        }

        match request.call() {
            Ok(response) if response.status() == 304 => Ok(BranchFetch::NotModified),
            Ok(mut response) => {
                let etag = response
                    .headers()
                    .get("etag")
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_string);
                let body: serde_json::Value = response
                    .body_mut()
                    .read_json()
                    .with_context(|| format!("failed to parse branch metadata for {owner}/{repo}@{branch}"))?;
                let sha = body
                    .get("commit")
                    .and_then(|commit| commit.get("sha"))
                    .and_then(|sha| sha.as_str())
                    .map(str::to_string);
                Ok(BranchFetch::Fresh {
                    exists: true,
                    sha,
                    etag,
                })
            }
            Err(ureq::Error::StatusCode(304)) => Ok(BranchFetch::NotModified),
            Err(ureq::Error::StatusCode(404)) => Ok(BranchFetch::Fresh {
                exists: false,
                sha: None,
                etag: None,
            }),
            Err(error) => {
                let Some(fallback) = &self.fallback else {
                    return Err(error)
                        .with_context(|| format!("GitHub REST branch lookup failed for {owner}/{repo}@{branch}"));
                };
                fallback
                    .fetch_branch(owner, repo, branch, None)
                    .with_context(|| format!("GitHub REST branch lookup failed for {owner}/{repo}@{branch}: {error}"))
            }
        }
    }

    fn fetch_commit(&self, owner: &str, repo: &str, sha: &str, etag: Option<&str>) -> Result<CommitFetch> {
        let url = format!("{}/repos/{owner}/{repo}/commits/{sha}", self.api_url);
        let mut request = self
            .agent
            .get(&url)
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", concat!("gh-actions-updater/", env!("CARGO_PKG_VERSION")));
        if let Some(token) = &self.token {
            request = request.header("Authorization", format!("Bearer {token}"));
        }
        if let Some(etag) = etag {
            request = request.header("If-None-Match", etag);
        }

        match request.call() {
            Ok(response) if response.status() == 304 => Ok(CommitFetch::NotModified),
            Ok(response) => Ok(CommitFetch::Fresh {
                exists: true,
                etag: response
                    .headers()
                    .get("etag")
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_string),
            }),
            Err(ureq::Error::StatusCode(304)) => Ok(CommitFetch::NotModified),
            Err(ureq::Error::StatusCode(404)) => Ok(CommitFetch::Fresh {
                exists: false,
                etag: None,
            }),
            Err(error) => {
                Err(error).with_context(|| format!("GitHub REST commit lookup failed for {owner}/{repo}@{sha}"))
            }
        }
    }
}

pub struct GitLsRemoteProvider;

impl TagProvider for GitLsRemoteProvider {
    fn fetch_tags(&self, owner: &str, repo: &str, _etag: Option<&str>) -> Result<TagFetch> {
        let url = format!("https://github.com/{owner}/{repo}.git");
        let output = Command::new("git")
            .args(["ls-remote", "--tags", &url])
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

    fn fetch_branch(&self, owner: &str, repo: &str, branch: &str, _etag: Option<&str>) -> Result<BranchFetch> {
        let url = format!("https://github.com/{owner}/{repo}.git");
        let reference = format!("refs/heads/{branch}");
        let output = Command::new("git")
            .args(["ls-remote", "--heads", &url, &reference])
            .output()
            .with_context(|| format!("failed to run git ls-remote for {owner}/{repo}"))?;

        if !output.status.success() {
            return Err(anyhow!(
                "git ls-remote failed for {owner}/{repo}: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let sha = stdout
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().next())
            .map(str::to_string);
        Ok(BranchFetch::Fresh {
            exists: sha.is_some(),
            sha,
            etag: None,
        })
    }
}

pub fn resolve_updates(
    settings: &Settings,
    cache: CacheState,
    references: &[ReferenceReport],
) -> Result<MetadataResolution> {
    resolve_updates_with_provider(settings, cache, references, &GitHubRestProvider::new(settings))
}

pub fn resolve_updates_with_provider(
    settings: &Settings,
    mut cache: CacheState,
    references: &[ReferenceReport],
    provider: &impl TagProvider,
) -> Result<MetadataResolution> {
    let memoizing_provider = MemoizingProvider::new(provider);
    let provider = &memoizing_provider;
    let mut diagnostics = Vec::new();
    let mut updates = Vec::new();
    let mut resolved_ref_kinds: Vec<(usize, RefKind)> = Vec::new();
    let mut has_metadata_failures = false;
    let mut tags_by_repo: AHashMap<(String, String), TagLoad> = AHashMap::new();
    let update_exclude = build_update_exclude_globset(&settings.update_exclude)?;

    for (idx, reference) in references.iter().enumerate() {
        if !matches!(
            reference.parsed.kind,
            ReferenceKind::RemoteAction | ReferenceKind::ReusableWorkflow
        ) {
            if settings.validate {
                validate_local_reference(reference, &mut diagnostics);
            }
            continue;
        }

        let (Some(owner), Some(repo), Some(current)) = (
            reference.parsed.owner.as_deref(),
            reference.parsed.repo.as_deref(),
            reference.parsed.ref_name.as_deref(),
        ) else {
            continue;
        };

        let excluded =
            reference.update_ignored || is_update_excluded(&update_exclude, reference.raw.as_str(), owner, repo);

        let semver_update = matches!(reference.parsed.ref_kind, RefKind::SemverLikeTag);
        let use_hash = mode_uses_hash(settings, &reference.parsed.ref_kind);
        let sha_update = use_hash && reference.parsed.ref_kind == RefKind::Sha;
        let sha_advisory = !use_hash && reference.parsed.ref_kind == RefKind::Sha;
        let needs_classification = matches!(
            reference.parsed.ref_kind,
            RefKind::BranchOrUnknown | RefKind::Branch | RefKind::NonSemverTag
        ) && (settings.validate || settings.pin_floating_to_sha);

        let needs_tags =
            (!excluded && (semver_update || sha_update || sha_advisory || needs_classification)) || settings.validate;

        let mut effective_ref_kind = reference.parsed.ref_kind.clone();
        let mut classification_sha: Option<String> = None;

        if needs_tags {
            let repo_key = (owner.to_string(), repo.to_string());
            if !tags_by_repo.contains_key(&repo_key) {
                let tags = match load_tags(settings, &mut cache, provider, owner, repo) {
                    Ok(tags) => tags,
                    Err(error) => {
                        diagnostics.push(Diagnostic {
                            file: reference.file.clone(),
                            line: Some(reference.line),
                            message: format!("metadata lookup failed for {owner}/{repo}: {error:#}"),
                            code: DiagnosticCode::MetadataLookupFailed,
                            category: DiagnosticCategory::Metadata,
                        });
                        has_metadata_failures = true;
                        continue;
                    }
                };
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
                    code: DiagnosticCode::MetadataLookupFailed,
                    category: DiagnosticCategory::Metadata,
                });
            }
            let tags = &tag_load.tags;

            if needs_classification {
                if effective_ref_kind == RefKind::BranchOrUnknown {
                    if let Some(tag) = tags.iter().find(|tag| tag.name == current) {
                        effective_ref_kind = RefKind::NonSemverTag;
                        classification_sha = Some(tag.sha.clone());
                    } else {
                        let branch_lookup =
                            match load_branch_exists(settings, &mut cache, provider, owner, repo, current) {
                                Ok(lookup) => lookup,
                                Err(error) => {
                                    push_metadata_failure(reference, &error, &mut diagnostics);
                                    has_metadata_failures = true;
                                    continue;
                                }
                            };
                        if let Some(warning) = branch_lookup.warning {
                            diagnostics.push(Diagnostic {
                                file: reference.file.clone(),
                                line: Some(reference.line),
                                message: warning,
                                code: DiagnosticCode::MetadataLookupFailed,
                                category: DiagnosticCategory::Metadata,
                            });
                        }
                        if let Some(sha) = branch_lookup.value {
                            effective_ref_kind = RefKind::Branch;
                            classification_sha = Some(sha);
                        }
                    }
                    if effective_ref_kind != RefKind::BranchOrUnknown {
                        resolved_ref_kinds.push((idx, effective_ref_kind.clone()));
                    }
                } else if matches!(effective_ref_kind, RefKind::Branch | RefKind::NonSemverTag) {
                    if let Some(tag) = tags.iter().find(|tag| tag.name == current) {
                        classification_sha = Some(tag.sha.clone());
                    } else {
                        let branch_lookup =
                            match load_branch_exists(settings, &mut cache, provider, owner, repo, current) {
                                Ok(lookup) => lookup,
                                Err(error) => {
                                    push_metadata_failure(reference, &error, &mut diagnostics);
                                    has_metadata_failures = true;
                                    continue;
                                }
                            };
                        classification_sha = branch_lookup.value;
                    }
                }
            }

            if settings.validate {
                if effective_ref_kind == RefKind::SemverLikeTag && !tags.iter().any(|tag| tag.name == current) {
                    let branch_lookup = match load_branch_exists(settings, &mut cache, provider, owner, repo, current) {
                        Ok(lookup) => lookup,
                        Err(error) => {
                            push_metadata_failure(reference, &error, &mut diagnostics);
                            has_metadata_failures = true;
                            continue;
                        }
                    };
                    if let Some(sha) = branch_lookup.value {
                        effective_ref_kind = RefKind::Branch;
                        classification_sha = Some(sha);
                        resolved_ref_kinds.push((idx, RefKind::Branch));
                    }
                }
                let exists_upstream = match effective_ref_kind {
                    RefKind::SemverLikeTag | RefKind::NonSemverTag => tags.iter().any(|tag| tag.name == current),
                    RefKind::Branch => classification_sha.is_some(),
                    RefKind::BranchOrUnknown => false,
                    RefKind::Sha => match load_commit_exists(settings, &mut cache, provider, owner, repo, current) {
                        Ok(exists) => exists,
                        Err(error) => {
                            push_metadata_failure(reference, &error, &mut diagnostics);
                            has_metadata_failures = true;
                            continue;
                        }
                    },
                    RefKind::None => true,
                };
                if !exists_upstream {
                    handle_missing_ref(settings, reference, &mut diagnostics)?;
                }
            }
        }

        if excluded {
            continue;
        }

        let update_candidate =
            if settings.pin_floating_to_sha && matches!(effective_ref_kind, RefKind::Branch | RefKind::NonSemverTag) {
                classification_sha.is_some()
            } else if use_hash {
                matches!(effective_ref_kind, RefKind::SemverLikeTag | RefKind::Sha)
            } else {
                matches!(effective_ref_kind, RefKind::SemverLikeTag | RefKind::Sha)
            };

        let use_hash = mode_uses_hash(settings, &effective_ref_kind);
        let sha_advisory = !use_hash && !settings.pin_floating_to_sha && effective_ref_kind == RefKind::Sha;

        let semver_update = !sha_advisory
            && (matches!(effective_ref_kind, RefKind::SemverLikeTag)
                || (use_hash && effective_ref_kind == RefKind::Sha));

        if !update_candidate && !sha_advisory {
            continue;
        }

        let tag_load = tags_by_repo.get(&(owner.to_string(), repo.to_string()));
        let tags: &[RemoteTag] = tag_load.map(|load| load.tags.as_slice()).unwrap_or(&[]);

        if settings.pin_floating_to_sha && matches!(effective_ref_kind, RefKind::Branch | RefKind::NonSemverTag) {
            let Some(sha) = classification_sha.clone() else {
                continue;
            };
            if sha != current {
                updates.push(UpdateReport {
                    file: reference.file.clone(),
                    line: reference.line,
                    current: current.to_string(),
                    target: Some(sha),
                    ref_span: reference.ref_span,
                    rewrite_supported: reference.rewrite_supported,
                    rewrite_reason: reference.rewrite_reason.clone(),
                });
            }
            continue;
        }

        if sha_advisory {
            let decision = match select_hash_update_target(
                &Settings {
                    latest_hash: true,
                    ..settings.clone()
                },
                &mut cache,
                provider,
                reference,
                current,
                tags,
            ) {
                Ok(decision) => decision,
                Err(error) => {
                    push_metadata_failure(reference, &error, &mut diagnostics);
                    has_metadata_failures = true;
                    continue;
                }
            };
            if let Some(target) = decision.target {
                if !target.sha.is_empty() && !target.sha.eq_ignore_ascii_case(current) {
                    let short = if current.len() >= 8 { &current[..8] } else { current };
                    diagnostics.push(Diagnostic {
                        file: reference.file.clone(),
                        line: Some(reference.line),
                        message: format!(
                            "SHA pin {short} tracks tag {tag}; newer SHA {newer_short} available — run with --latest-hash to update",
                            tag = target.name,
                            newer_short = if target.sha.len() >= 8 { &target.sha[..8] } else { target.sha.as_str() },
                        ),
                        code: DiagnosticCode::General,
                        category: DiagnosticCategory::General,
                    });
                }
            }
            if decision.current_missing {
                handle_missing_ref(settings, reference, &mut diagnostics)?;
            }
            if let Some(message) = decision.diagnostic {
                diagnostics.push(Diagnostic {
                    file: reference.file.clone(),
                    line: Some(reference.line),
                    message,
                    code: DiagnosticCode::General,
                    category: DiagnosticCategory::General,
                });
            }
            continue;
        }

        if !semver_update {
            continue;
        }

        let decision_result = if use_hash {
            select_hash_update_target(settings, &mut cache, provider, reference, current, tags)
        } else {
            select_tag_update_target(settings, &mut cache, provider, reference, current, tags)
        };
        let decision = match decision_result {
            Ok(decision) => decision,
            Err(error) => {
                push_metadata_failure(reference, &error, &mut diagnostics);
                has_metadata_failures = true;
                continue;
            }
        };
        let Some(target) = decision.target else {
            if decision.current_missing {
                handle_missing_ref(settings, reference, &mut diagnostics)?;
            }
            if let Some(message) = decision.diagnostic {
                diagnostics.push(Diagnostic {
                    file: reference.file.clone(),
                    line: Some(reference.line),
                    message,
                    code: DiagnosticCode::General,
                    category: DiagnosticCategory::General,
                });
            }
            continue;
        };

        let target_ref = if use_hash { target.sha } else { target.name };

        if target_ref != current && (!decision.current_missing || settings.missing_ref == MissingRefPolicy::Fallback) {
            if decision.current_missing && settings.missing_ref == MissingRefPolicy::Fallback {
                diagnostics.retain(|diagnostic| {
                    diagnostic.code != DiagnosticCode::RemoteReferenceMissing
                        || diagnostic.file != reference.file
                        || diagnostic.line != Some(reference.line)
                });
            }
            updates.push(UpdateReport {
                file: reference.file.clone(),
                line: reference.line,
                current: current.to_string(),
                target: Some(target_ref),
                ref_span: reference.ref_span,
                rewrite_supported: reference.rewrite_supported,
                rewrite_reason: reference.rewrite_reason.clone(),
            });
        } else if let Some(message) = decision.diagnostic {
            diagnostics.push(Diagnostic {
                file: reference.file.clone(),
                line: Some(reference.line),
                message,
                code: DiagnosticCode::General,
                category: DiagnosticCategory::General,
            });
        }
    }

    let mut metadata_failure_keys = ahash::AHashSet::new();
    let mut missing_reference_keys = ahash::AHashSet::new();
    diagnostics.retain(|diagnostic| {
        if diagnostic.code == DiagnosticCode::MetadataLookupFailed {
            return metadata_failure_keys.insert((
                diagnostic.file.clone(),
                diagnostic.line,
                diagnostic.message.clone(),
            ));
        }
        if diagnostic.code == DiagnosticCode::RemoteReferenceMissing {
            return missing_reference_keys.insert((
                diagnostic.file.clone(),
                diagnostic.line,
                diagnostic.message.clone(),
            ));
        }
        true
    });

    Ok(MetadataResolution {
        updates,
        diagnostics,
        resolved_ref_kinds,
        has_metadata_failures,
        cache: cache.report,
    })
}

fn mode_uses_hash(settings: &Settings, ref_kind: &RefKind) -> bool {
    if settings.latest_hash {
        return true;
    }
    match settings.update_mode {
        UpdateMode::LatestHash => true,
        UpdateMode::LatestTag => false,
        UpdateMode::Latest => *ref_kind == RefKind::Sha,
    }
}

fn push_metadata_failure(reference: &ReferenceReport, error: &anyhow::Error, diagnostics: &mut Vec<Diagnostic>) {
    diagnostics.push(Diagnostic {
        file: reference.file.clone(),
        line: Some(reference.line),
        message: format!("metadata lookup failed for {}: {error:#}", reference.raw),
        code: DiagnosticCode::MetadataLookupFailed,
        category: DiagnosticCategory::Metadata,
    });
}

fn build_update_exclude_globset(patterns: &[String]) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        builder.add(Glob::new(pattern).with_context(|| format!("invalid update exclude glob {pattern}"))?);
    }
    builder.build().context("failed to build update exclude glob set")
}

fn is_update_excluded(globset: &GlobSet, raw: &str, owner: &str, repo: &str) -> bool {
    globset.is_match(raw) || globset.is_match(format!("{owner}/{repo}"))
}

fn load_tags(
    settings: &Settings,
    cache: &mut CacheState,
    provider: &impl TagProvider,
    owner: &str,
    repo: &str,
) -> Result<TagLoad> {
    let auth_fingerprint = auth_fingerprint(settings.github_token.as_deref());
    let lookup_mode = if settings.latest_hash {
        "tags-commit-sha-v2"
    } else {
        "tags"
    };
    let key = cache_key(CacheKeyParts {
        api_host: &settings.github_api_url,
        owner,
        repo,
        lookup_mode,
        auth_fingerprint: &auth_fingerprint,
    });

    match cache.read_json::<TagCacheValue>(&key)? {
        CacheLookup::Fresh(value) => {
            return Ok(TagLoad {
                tags: value.tags,
                warning: None,
            });
        }
        CacheLookup::Stale(value) => match provider.fetch_tags(owner, repo, value.etag.as_deref()) {
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
                return Ok(TagLoad { tags, warning: None });
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
        },
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
    Ok(TagLoad { tags, warning: None })
}

fn load_commit_exists(
    settings: &Settings,
    cache: &mut CacheState,
    provider: &impl TagProvider,
    owner: &str,
    repo: &str,
    sha: &str,
) -> Result<bool> {
    let auth_fingerprint = auth_fingerprint(settings.github_token.as_deref());
    let lookup_mode = format!("commit:{sha}");
    let key = cache_key(CacheKeyParts {
        api_host: &settings.github_api_url,
        owner,
        repo,
        lookup_mode: &lookup_mode,
        auth_fingerprint: &auth_fingerprint,
    });

    match cache.read_json::<CommitCacheValue>(&key)? {
        CacheLookup::Fresh(value) => return Ok(value.exists),
        CacheLookup::Stale(value) => match provider.fetch_commit(owner, repo, sha, value.etag.as_deref()) {
            Ok(CommitFetch::Fresh { exists, etag }) => {
                cache.write_json(
                    &key,
                    &CommitCacheValue {
                        provider: "github-rest".to_string(),
                        api_host: settings.github_api_url.clone(),
                        auth_fingerprint: auth_fingerprint.clone(),
                        etag,
                        sha: sha.to_string(),
                        exists,
                    },
                )?;
                return Ok(exists);
            }
            Ok(CommitFetch::NotModified) => {
                cache.write_json(&key, &value)?;
                return Ok(value.exists);
            }
            Err(_error) if !settings.check && !settings.update => return Ok(value.exists),
            Err(error) => return Err(error),
        },
        CacheLookup::Corrupt => {}
        CacheLookup::Miss => {}
    }

    let (exists, etag) = match provider.fetch_commit(owner, repo, sha, None)? {
        CommitFetch::Fresh { exists, etag } => (exists, etag),
        CommitFetch::NotModified => {
            return Err(anyhow!(
                "metadata provider returned not-modified for {owner}/{repo}@{sha} without cached commit metadata"
            ));
        }
    };
    cache.write_json(
        &key,
        &CommitCacheValue {
            provider: "github-rest".to_string(),
            api_host: settings.github_api_url.clone(),
            auth_fingerprint,
            etag,
            sha: sha.to_string(),
            exists,
        },
    )?;
    Ok(exists)
}

fn load_branch_exists(
    settings: &Settings,
    cache: &mut CacheState,
    provider: &impl TagProvider,
    owner: &str,
    repo: &str,
    branch: &str,
) -> Result<LookupLoad<Option<String>>> {
    let auth_fingerprint = auth_fingerprint(settings.github_token.as_deref());
    let lookup_mode = format!("branch-sha-v2:{branch}");
    let key = cache_key(CacheKeyParts {
        api_host: &settings.github_api_url,
        owner,
        repo,
        lookup_mode: &lookup_mode,
        auth_fingerprint: &auth_fingerprint,
    });

    match cache.read_json::<BranchCacheValue>(&key)? {
        CacheLookup::Fresh(value) => {
            return Ok(LookupLoad {
                value: value.exists.then_some(value.sha).flatten(),
                warning: None,
            });
        }
        CacheLookup::Stale(value) => match provider.fetch_branch(owner, repo, branch, value.etag.as_deref()) {
            Ok(BranchFetch::Fresh { exists, sha, etag }) => {
                cache.write_json(
                    &key,
                    &BranchCacheValue {
                        provider: "github-rest-or-fallback".to_string(),
                        api_host: settings.github_api_url.clone(),
                        auth_fingerprint: auth_fingerprint.clone(),
                        etag,
                        branch: branch.to_string(),
                        exists,
                        sha: sha.clone(),
                    },
                )?;
                return Ok(LookupLoad {
                    value: if exists { sha } else { None },
                    warning: None,
                });
            }
            Ok(BranchFetch::NotModified) => {
                cache.write_json(&key, &value)?;
                return Ok(LookupLoad {
                    value: value.exists.then_some(value.sha).flatten(),
                    warning: None,
                });
            }
            Err(error) if !settings.check && !settings.update => {
                return Ok(LookupLoad {
                    value: value.exists.then_some(value.sha).flatten(),
                    warning: Some(format!(
                        "using stale branch metadata for {owner}/{repo}@{branch} after refresh failed: {error}"
                    )),
                });
            }
            Err(error) => return Err(error),
        },
        CacheLookup::Corrupt => {}
        CacheLookup::Miss => {}
    }

    let (exists, sha, etag) = match provider.fetch_branch(owner, repo, branch, None)? {
        BranchFetch::Fresh { exists, sha, etag } => (exists, sha, etag),
        BranchFetch::NotModified => {
            return Err(anyhow!(
                "metadata provider returned not-modified for {owner}/{repo}@{branch} without cached branch metadata"
            ));
        }
    };
    cache.write_json(
        &key,
        &BranchCacheValue {
            provider: "github-rest-or-fallback".to_string(),
            api_host: settings.github_api_url.clone(),
            auth_fingerprint,
            etag,
            branch: branch.to_string(),
            exists,
            sha: sha.clone(),
        },
    )?;
    Ok(LookupLoad {
        value: if exists { sha } else { None },
        warning: None,
    })
}

#[derive(Debug, Clone)]
struct TagLoad {
    tags: Vec<RemoteTag>,
    warning: Option<String>,
}

#[derive(Debug, Clone)]
struct LookupLoad<T> {
    value: T,
    warning: Option<String>,
}

fn validate_local_reference(reference: &ReferenceReport, diagnostics: &mut Vec<Diagnostic>) {
    if !matches!(
        reference.parsed.kind,
        ReferenceKind::LocalAction | ReferenceKind::LocalWorkflow
    ) {
        return;
    }
    let Some(rel) = reference.parsed.path.as_deref() else {
        return;
    };
    let workflow_path = std::path::Path::new(&reference.file);
    let base = repository_root_for(workflow_path);
    let Some(target_path) = local_target_within_repository(base, std::path::Path::new(rel)) else {
        diagnostics.push(Diagnostic {
            file: reference.file.clone(),
            line: Some(reference.line),
            message: format!("local reference escapes repository root: {rel}"),
            code: DiagnosticCode::LocalReferenceEscapesRepository,
            category: DiagnosticCategory::Validation,
        });
        return;
    };
    let Some(candidate) = local_reference_candidate(&reference.parsed.kind, &target_path) else {
        push_local_missing(reference, rel, diagnostics, "expected action metadata or workflow file");
        return;
    };
    let canonical_root = match std::fs::canonicalize(base) {
        Ok(path) => path,
        Err(error) => {
            push_local_missing(
                reference,
                rel,
                diagnostics,
                &format!("failed to resolve repository root: {error}"),
            );
            return;
        }
    };
    let canonical_target = match std::fs::canonicalize(&target_path) {
        Ok(path) => path,
        Err(error) => {
            push_local_missing(
                reference,
                rel,
                diagnostics,
                &format!("failed to resolve local target: {error}"),
            );
            return;
        }
    };
    let canonical_candidate = match std::fs::canonicalize(&candidate) {
        Ok(path) => path,
        Err(error) => {
            push_local_missing(
                reference,
                rel,
                diagnostics,
                &format!("failed to resolve local reference: {error}"),
            );
            return;
        }
    };
    if !canonical_target.starts_with(&canonical_root) || !canonical_candidate.starts_with(&canonical_root) {
        diagnostics.push(Diagnostic {
            file: reference.file.clone(),
            line: Some(reference.line),
            message: format!("local reference escapes repository root through symlink: {rel}"),
            code: DiagnosticCode::LocalReferenceEscapesRepository,
            category: DiagnosticCategory::Validation,
        });
    }
}

fn local_reference_candidate(kind: &ReferenceKind, target: &std::path::Path) -> Option<std::path::PathBuf> {
    match kind {
        ReferenceKind::LocalAction if target.is_dir() => [target.join("action.yml"), target.join("action.yaml")]
            .into_iter()
            .find(|candidate| candidate.is_file()),
        ReferenceKind::LocalWorkflow if target.is_file() => Some(target.to_path_buf()),
        _ => None,
    }
}

fn push_local_missing(reference: &ReferenceReport, relative: &str, diagnostics: &mut Vec<Diagnostic>, context: &str) {
    diagnostics.push(Diagnostic {
        file: reference.file.clone(),
        line: Some(reference.line),
        message: format!("local reference does not exist on disk: {relative} ({context})"),
        code: DiagnosticCode::LocalReferenceMissing,
        category: DiagnosticCategory::Validation,
    });
}

fn local_target_within_repository(base: &std::path::Path, relative: &std::path::Path) -> Option<std::path::PathBuf> {
    let mut normalized = std::path::PathBuf::new();
    for component in relative.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::Normal(part) => normalized.push(part),
            std::path::Component::ParentDir => {
                if !normalized.pop() {
                    return None;
                }
            }
            std::path::Component::RootDir | std::path::Component::Prefix(_) => return None,
        }
    }
    Some(base.join(normalized))
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
    diagnostics.push(Diagnostic {
        file: reference.file.clone(),
        line: Some(reference.line),
        message,
        code: DiagnosticCode::RemoteReferenceMissing,
        category: DiagnosticCategory::Validation,
    });
    Ok(())
}

fn repository_root_for(path: &std::path::Path) -> &std::path::Path {
    let directory = path.parent().unwrap_or(std::path::Path::new("."));
    directory
        .ancestors()
        .find(|ancestor| ancestor.join(".git").exists())
        .or_else(|| directory.ancestors().find(|ancestor| ancestor.join(".github").is_dir()))
        .unwrap_or(directory)
}

#[derive(Debug)]
struct TargetDecision {
    target: Option<RemoteTag>,
    current_missing: bool,
    diagnostic: Option<String>,
}

fn select_tag_update_target(
    settings: &Settings,
    cache: &mut CacheState,
    provider: &impl TagProvider,
    reference: &ReferenceReport,
    current: &str,
    tags: &[RemoteTag],
) -> Result<TargetDecision> {
    let Some(current_ref) = parse_version_ref(current) else {
        return Ok(TargetDecision {
            target: None,
            current_missing: false,
            diagnostic: None,
        });
    };
    let current_exists = tags.iter().any(|tag| tag.name == current);
    if settings.preserve_major
        && current_exists
        && settings.pin_style == PinStyle::Preserve
        && current_ref.precision != VersionPrecision::Full
    {
        return Ok(TargetDecision {
            target: tags.iter().find(|tag| tag.name == current).cloned(),
            current_missing: false,
            diagnostic: None,
        });
    }
    let mut diagnostic = None;
    if !current_exists {
        let branch_lookup = load_branch_exists(
            settings,
            cache,
            provider,
            reference.parsed.owner.as_deref().unwrap_or_default(),
            reference.parsed.repo.as_deref().unwrap_or_default(),
            current,
        )?;
        diagnostic = branch_lookup.warning;
        if branch_lookup.value.is_some() {
            return Ok(TargetDecision {
                target: None,
                current_missing: false,
                diagnostic,
            });
        }
    }

    let latest = latest_semver_tag(settings, tags, Some(current_ref.version.major));
    let latest = latest.filter(|tag| {
        if current_ref.precision != VersionPrecision::Full {
            return true;
        }
        parse_version_tag(&tag.name).is_some_and(|target_version| target_version > current_ref.version)
    });
    let mut target = latest
        .as_ref()
        .map(|tag| format_pin_style(settings.pin_style, current, &current_ref, tag));
    if !current_exists && settings.missing_ref == MissingRefPolicy::Fallback {
        target = latest.as_ref().map(|tag| tag.name.clone());
    }
    let floating_ref_is_resolved = current_exists;

    if settings.latest_hash
        && !current_exists
        && current_ref.precision != VersionPrecision::Full
        && target.as_deref() == Some(current)
    {
        return Ok(TargetDecision {
            target: None,
            current_missing: false,
            diagnostic,
        });
    }

    if !current_exists && !floating_ref_is_resolved && settings.missing_ref != MissingRefPolicy::Fallback {
        return Ok(TargetDecision {
            target: None,
            current_missing: true,
            diagnostic,
        });
    }

    if let (Some(target), Some(latest)) = (&target, &latest)
        && target != current
        && target != &latest.name
        && !tags.iter().any(|tag| tag.name == *target)
    {
        let branch_lookup = load_branch_exists(
            settings,
            cache,
            provider,
            reference.parsed.owner.as_deref().unwrap_or_default(),
            reference.parsed.repo.as_deref().unwrap_or_default(),
            target,
        )?;
        if let Some(warning) = branch_lookup.warning {
            diagnostic = Some(warning);
        }
        if branch_lookup.value.is_none() {
            return Ok(TargetDecision {
                target: None,
                current_missing: false,
                diagnostic: diagnostic
                    .or_else(|| Some(format!("pin-style target does not exist as a tag or branch: {target}"))),
            });
        }
    }

    let target = target.map(|target| RemoteTag {
        name: target,
        sha: latest.map(|tag| tag.sha).unwrap_or_default(),
    });

    Ok(TargetDecision {
        target,
        current_missing: !current_exists && !floating_ref_is_resolved,
        diagnostic,
    })
}

fn select_hash_update_target(
    settings: &Settings,
    cache: &mut CacheState,
    provider: &impl TagProvider,
    reference: &ReferenceReport,
    current: &str,
    tags: &[RemoteTag],
) -> Result<TargetDecision> {
    match reference.parsed.ref_kind {
        RefKind::SemverLikeTag => {
            let mut decision = select_tag_update_target(settings, cache, provider, reference, current, tags)?;
            if decision.target.as_ref().is_some_and(|target| target.sha.is_empty()) {
                let name = decision
                    .target
                    .as_ref()
                    .map(|target| target.name.clone())
                    .unwrap_or_default();
                decision.target = None;
                decision.diagnostic = Some(format!("selected tag has no commit SHA metadata: {name}"));
            }
            Ok(decision)
        }
        RefKind::Sha => {
            let current_tag_version = tags
                .iter()
                .filter(|tag| tag.sha.eq_ignore_ascii_case(current))
                .filter_map(|tag| parse_version_tag(&tag.name))
                .filter(|version| settings.include_prereleases || version.pre.is_empty())
                .max();

            let current_exists = if current_tag_version.is_some() {
                true
            } else {
                load_commit_exists(
                    settings,
                    cache,
                    provider,
                    reference.parsed.owner.as_deref().unwrap_or_default(),
                    reference.parsed.repo.as_deref().unwrap_or_default(),
                    current,
                )?
            };

            if !current_exists && settings.missing_ref != MissingRefPolicy::Fallback {
                return Ok(TargetDecision {
                    target: None,
                    current_missing: true,
                    diagnostic: None,
                });
            }

            let major = current_tag_version.map(|version| version.major);
            Ok(TargetDecision {
                target: latest_semver_tag(settings, tags, major),
                current_missing: !current_exists,
                diagnostic: None,
            })
        }
        _ => Ok(TargetDecision {
            target: None,
            current_missing: false,
            diagnostic: None,
        }),
    }
}

fn latest_semver_tag(settings: &Settings, tags: &[RemoteTag], major: Option<u64>) -> Option<RemoteTag> {
    tags.iter()
        .filter_map(|tag| parse_version_tag(&tag.name).map(|version| (tag, version)))
        .filter(|(_, version)| !settings.preserve_major || major.is_none_or(|major| version.major == major))
        .filter(|(_, version)| settings.include_prereleases || version.pre.is_empty())
        .max_by(|(_, left), (_, right)| left.cmp(right))
        .map(|(tag, _)| tag.clone())
}

pub fn exit_code_for_resolution(settings: &Settings, resolution: &MetadataResolution) -> Option<i32> {
    if settings.check && !resolution.updates.is_empty() {
        return Some(1);
    }

    if settings.check
        && resolution.diagnostics.iter().any(|diagnostic| {
            diagnostic.category == DiagnosticCategory::Validation
                && diagnostic.code != DiagnosticCode::RemoteReferenceMissing
        })
    {
        return Some(1);
    }

    if settings.missing_ref == MissingRefPolicy::Error
        && resolution
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == DiagnosticCode::RemoteReferenceMissing)
    {
        return Some(1);
    }

    None
}

fn parse_version_tag(tag: &str) -> Option<Version> {
    parse_version_ref(tag).map(|version_ref| version_ref.version)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum VersionPrecision {
    Major,
    Minor,
    Full,
}

#[derive(Clone, Debug)]
struct VersionRef {
    version: Version,
    precision: VersionPrecision,
}

fn parse_version_ref(tag: &str) -> Option<VersionRef> {
    let version = tag.strip_prefix('v').unwrap_or(tag);
    let parts: Vec<_> = version.split('.').collect();
    let (version, precision) = match parts.as_slice() {
        [major] => (Version::parse(&format!("{major}.0.0")).ok()?, VersionPrecision::Major),
        [major, minor] => (
            Version::parse(&format!("{major}.{minor}.0")).ok()?,
            VersionPrecision::Minor,
        ),
        [major, minor, patch] => (
            Version::parse(&format!("{major}.{minor}.{patch}")).ok()?,
            VersionPrecision::Full,
        ),
        _ => (Version::parse(version).ok()?, VersionPrecision::Full),
    };
    Some(VersionRef { version, precision })
}

fn format_pin_style(pin_style: PinStyle, current: &str, current_ref: &VersionRef, target: &RemoteTag) -> String {
    match pin_style {
        PinStyle::Preserve => format_version_ref(current_ref.precision, current, target),
        PinStyle::Major => format_version_ref(VersionPrecision::Major, current, target),
        PinStyle::Minor => format_version_ref(VersionPrecision::Minor, current, target),
        PinStyle::Full => target.name.clone(),
    }
}

fn format_version_ref(precision: VersionPrecision, current: &str, target: &RemoteTag) -> String {
    let Some(target_ref) = parse_version_ref(&target.name) else {
        return target.name.clone();
    };
    let prefix = if current.starts_with('v') || target.name.starts_with('v') {
        "v"
    } else {
        ""
    };
    match precision {
        VersionPrecision::Major => format!("{prefix}{}", target_ref.version.major),
        VersionPrecision::Minor => {
            format!("{prefix}{}.{}", target_ref.version.major, target_ref.version.minor)
        }
        VersionPrecision::Full => target.name.clone(),
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
    let mut tags_by_name: AHashMap<String, RemoteTag> = AHashMap::new();
    for line in output.lines() {
        let Some((sha, reference)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        let Some(raw_name) = reference.trim().strip_prefix("refs/tags/") else {
            continue;
        };
        let (name, peeled) = raw_name
            .strip_suffix("^{}")
            .map_or((raw_name, false), |name| (name, true));
        if peeled || !tags_by_name.contains_key(name) {
            tags_by_name.insert(
                name.to_string(),
                RemoteTag {
                    name: name.to_string(),
                    sha: sha.to_string(),
                },
            );
        }
    }
    let mut tags = tags_by_name.into_values().collect::<Vec<_>>();
    tags.sort_by(|left, right| left.name.cmp(&right.name));
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
        BranchFetch, CommitFetch, MetadataResolution, RemoteTag, TagFetch, TagProvider, exit_code_for_resolution,
        parse_ls_remote_tags, resolve_updates_with_provider,
    };
    use crate::cache::{CacheKeyParts, CacheState, cache_key};
    use crate::cli::{Cli, ColorChoice, MissingRefPolicy, OutputFormat, PinStyle, UpdateMode};
    use crate::config::{CacheTtl, Settings};
    use crate::scanner::{DiagnosticCategory, DiagnosticCode, ReferenceReport, scan_files};
    use anyhow::Result;
    use clap::Parser;
    use std::cell::Cell;
    use std::path::Path;

    struct FakeProvider {
        tags: Vec<RemoteTag>,
        calls: Cell<usize>,
    }

    struct FailingProvider {
        calls: Cell<usize>,
    }

    struct CommitCheckingProvider {
        tags: Vec<RemoteTag>,
        commit_exists: bool,
        tag_calls: Cell<usize>,
        commit_calls: Cell<usize>,
    }

    impl TagProvider for FakeProvider {
        fn fetch_tags(&self, _owner: &str, _repo: &str, _etag: Option<&str>) -> Result<TagFetch> {
            self.calls.set(self.calls.get() + 1);
            Ok(TagFetch::Fresh {
                tags: self.tags.clone(),
                etag: Some("etag-test".to_string()),
            })
        }

        fn fetch_branch(&self, _owner: &str, _repo: &str, _branch: &str, _etag: Option<&str>) -> Result<BranchFetch> {
            Ok(BranchFetch::Fresh {
                exists: false,
                sha: None,
                etag: Some("etag-branch".to_string()),
            })
        }
    }

    impl TagProvider for FailingProvider {
        fn fetch_tags(&self, _owner: &str, _repo: &str, _etag: Option<&str>) -> Result<TagFetch> {
            self.calls.set(self.calls.get() + 1);
            anyhow::bail!("network unavailable")
        }
    }

    impl TagProvider for CommitCheckingProvider {
        fn fetch_tags(&self, _owner: &str, _repo: &str, _etag: Option<&str>) -> Result<TagFetch> {
            self.tag_calls.set(self.tag_calls.get() + 1);
            Ok(TagFetch::Fresh {
                tags: self.tags.clone(),
                etag: Some("etag-tags".to_string()),
            })
        }

        fn fetch_commit(&self, _owner: &str, _repo: &str, _sha: &str, _etag: Option<&str>) -> Result<CommitFetch> {
            self.commit_calls.set(self.commit_calls.get() + 1);
            Ok(CommitFetch::Fresh {
                exists: self.commit_exists,
                etag: Some("etag-commit".to_string()),
            })
        }
    }

    struct BranchCheckingProvider {
        tags: Vec<RemoteTag>,
        branch_exists: bool,
        tag_calls: Cell<usize>,
        branch_calls: Cell<usize>,
    }

    struct SelectiveFailureProvider {
        branch_calls: Cell<usize>,
        commit_calls: Cell<usize>,
    }

    impl TagProvider for SelectiveFailureProvider {
        fn fetch_tags(&self, owner: &str, _repo: &str, _etag: Option<&str>) -> Result<TagFetch> {
            let tags = if owner == "working" {
                vec![
                    tag_with_sha("v1.0.0", "1111111111111111111111111111111111111111"),
                    tag_with_sha("v1.1.0", "2222222222222222222222222222222222222222"),
                ]
            } else {
                Vec::new()
            };
            Ok(TagFetch::Fresh { tags, etag: None })
        }

        fn fetch_branch(&self, _owner: &str, _repo: &str, _branch: &str, _etag: Option<&str>) -> Result<BranchFetch> {
            self.branch_calls.set(self.branch_calls.get() + 1);
            anyhow::bail!("branch lookup unavailable")
        }

        fn fetch_commit(&self, _owner: &str, _repo: &str, _sha: &str, _etag: Option<&str>) -> Result<CommitFetch> {
            self.commit_calls.set(self.commit_calls.get() + 1);
            anyhow::bail!("commit lookup unavailable")
        }
    }

    impl TagProvider for BranchCheckingProvider {
        fn fetch_tags(&self, _owner: &str, _repo: &str, _etag: Option<&str>) -> Result<TagFetch> {
            self.tag_calls.set(self.tag_calls.get() + 1);
            Ok(TagFetch::Fresh {
                tags: self.tags.clone(),
                etag: Some("etag-tags".to_string()),
            })
        }

        fn fetch_branch(&self, _owner: &str, _repo: &str, _branch: &str, _etag: Option<&str>) -> Result<BranchFetch> {
            self.branch_calls.set(self.branch_calls.get() + 1);
            Ok(BranchFetch::Fresh {
                exists: self.branch_exists,
                sha: self
                    .branch_exists
                    .then_some("0123456789abcdef0123456789abcdef01234567".to_string()),
                etag: Some("etag-branch".to_string()),
            })
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
            update_mode: UpdateMode::LatestTag,
            latest_hash: false,
            pin_style: PinStyle::Preserve,
            update_exclude: Vec::new(),
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
            recursive: false,
            threads: None,
            validate: false,
            pin_floating_to_sha: false,
        }
    }

    #[test]
    fn aggregates_and_deduplicates_branch_failures_while_preserving_updates() {
        let temp = tempfile::tempdir().unwrap();
        let mut settings = settings(temp.path());
        settings.validate = true;
        let provider = SelectiveFailureProvider {
            branch_calls: Cell::new(0),
            commit_calls: Cell::new(0),
        };
        let first_failure = reference("broken/action@v1");
        let mut second_failure = reference("broken/action@v1");
        second_failure.file = ".github/workflows/release.yml".to_string();
        second_failure.line = 19;

        let resolution = resolve_updates_with_provider(
            &settings,
            CacheState::prepare(&settings).unwrap(),
            &[first_failure, second_failure, reference("working/action@v1.0.0")],
            &provider,
        )
        .unwrap();

        assert!(resolution.has_metadata_failures);
        assert_eq!(provider.branch_calls.get(), 1);
        assert_eq!(resolution.updates[0].target.as_deref(), Some("v1.1.0"));
        assert_eq!(
            resolution
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code == DiagnosticCode::MetadataLookupFailed)
                .count(),
            2
        );
    }

    #[test]
    fn aggregates_and_deduplicates_commit_failures_while_preserving_updates() {
        let temp = tempfile::tempdir().unwrap();
        let mut settings = settings(temp.path());
        settings.update_mode = UpdateMode::LatestHash;
        let provider = SelectiveFailureProvider {
            branch_calls: Cell::new(0),
            commit_calls: Cell::new(0),
        };
        let sha = "9999999999999999999999999999999999999999";

        let resolution = resolve_updates_with_provider(
            &settings,
            CacheState::prepare(&settings).unwrap(),
            &[
                reference(&format!("broken/action@{sha}")),
                reference(&format!("broken/action@{sha}")),
                reference("working/action@v1.0.0"),
            ],
            &provider,
        )
        .unwrap();

        assert!(resolution.has_metadata_failures);
        assert_eq!(provider.commit_calls.get(), 1);
        assert_eq!(
            resolution.updates[0].target.as_deref(),
            Some("2222222222222222222222222222222222222222")
        );
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
            update_ignored: false,
        }
    }

    fn tag(name: &str) -> RemoteTag {
        RemoteTag {
            name: name.to_string(),
            sha: format!("sha-{name}"),
        }
    }

    #[test]
    fn accepts_existing_major_only_tag() {
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

        assert!(resolution.updates.is_empty());
        assert_eq!(provider.calls.get(), 1);
    }

    #[test]
    fn default_settings_report_newer_major_action_tags() {
        let cases = [
            ("actions/checkout@v3", "v5"),
            ("actions/setup-node@v2", "v6"),
            ("actions/cache@v3", "v4"),
        ];

        for (current, expected) in cases {
            let temp = tempfile::tempdir().unwrap();
            let cli = Cli::parse_from(["gau", temp.path().to_str().unwrap()]);
            let mut settings = Settings::resolve(&cli).unwrap();
            settings.cache_dir = temp.path().join(".cache");
            settings.cache_enabled = false;
            let provider = FakeProvider {
                tags: vec![tag(current.rsplit_once('@').unwrap().1), tag(expected)],
                calls: Cell::new(0),
            };

            let resolution = resolve_updates_with_provider(
                &settings,
                CacheState::prepare(&settings).unwrap(),
                &[reference(current)],
                &provider,
            )
            .unwrap();

            assert_eq!(resolution.updates.len(), 1, "expected an update for {current}");
            assert_eq!(resolution.updates[0].target.as_deref(), Some(expected));
            assert_eq!(provider.calls.get(), 1);
        }
    }

    #[test]
    fn reports_update_for_major_only_workflow_ref_when_major_preservation_is_disabled() {
        let temp = tempfile::tempdir().unwrap();
        let workflow = temp.path().join(".github/workflows/ci.yml");
        std::fs::create_dir_all(workflow.parent().unwrap()).unwrap();
        std::fs::write(
            &workflow,
            "jobs:\n  test:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: actions/checkout@v3\n",
        )
        .unwrap();
        let mut settings = settings(temp.path());
        settings.preserve_major = false;
        let scan = scan_files(&[workflow], &settings).unwrap();
        let provider = FakeProvider {
            tags: vec![tag("v3"), tag("v4")],
            calls: Cell::new(0),
        };

        let resolution = resolve_updates_with_provider(
            &settings,
            CacheState::prepare(&settings).unwrap(),
            &scan.references,
            &provider,
        )
        .unwrap();

        assert_eq!(scan.references.len(), 1);
        assert_eq!(scan.references[0].raw, "actions/checkout@v3");
        assert_eq!(resolution.updates.len(), 1);
        assert_eq!(resolution.updates[0].target.as_deref(), Some("v4"));
    }

    #[test]
    fn preserve_keeps_missing_major_float_when_same_major_tags_exist() {
        let temp = tempfile::tempdir().unwrap();
        let settings = settings(temp.path());
        let provider = FakeProvider {
            tags: vec![tag("v1.0.1"), tag("v1.0.2")],
            calls: Cell::new(0),
        };

        let resolution = resolve_updates_with_provider(
            &settings,
            CacheState::prepare(&settings).unwrap(),
            &[reference("actions/setup-example@v1")],
            &provider,
        )
        .unwrap();

        assert!(resolution.updates.is_empty());
        assert_eq!(resolution.diagnostics.len(), 1);
        assert_eq!(resolution.diagnostics[0].code, DiagnosticCode::RemoteReferenceMissing);
    }

    #[test]
    fn preserve_keeps_minor_float_when_patch_tag_exists() {
        let temp = tempfile::tempdir().unwrap();
        let settings = settings(temp.path());
        let provider = FakeProvider {
            tags: vec![tag("v1.24"), tag("v1.24.0")],
            calls: Cell::new(0),
        };

        let resolution = resolve_updates_with_provider(
            &settings,
            CacheState::prepare(&settings).unwrap(),
            &[reference("erlef/setup-beam@v1.24")],
            &provider,
        )
        .unwrap();

        assert!(resolution.updates.is_empty());
        assert!(resolution.diagnostics.is_empty());
    }

    #[test]
    fn preserve_does_not_demote_full_pin_to_equivalent_minor_tag() {
        let temp = tempfile::tempdir().unwrap();
        let settings = settings(temp.path());
        let provider = FakeProvider {
            tags: vec![tag("v1.23"), tag("v1.23.0"), tag("v1.24"), tag("v1.24.0")],
            calls: Cell::new(0),
        };

        let resolution = resolve_updates_with_provider(
            &settings,
            CacheState::prepare(&settings).unwrap(),
            &[reference("erlef/setup-beam@v1.24.0")],
            &provider,
        )
        .unwrap();

        assert!(
            resolution.updates.is_empty(),
            "must not propose v1.24.0 -> v1.24 (same semver, lower precision); got {:?}",
            resolution.updates
        );
        assert!(resolution.diagnostics.is_empty());
    }

    #[test]
    fn preserve_still_updates_full_pin_to_strictly_newer_tag() {
        let temp = tempfile::tempdir().unwrap();
        let settings = settings(temp.path());
        let provider = FakeProvider {
            tags: vec![tag("v1.24"), tag("v1.24.0"), tag("v1.25"), tag("v1.25.1")],
            calls: Cell::new(0),
        };

        let resolution = resolve_updates_with_provider(
            &settings,
            CacheState::prepare(&settings).unwrap(),
            &[reference("erlef/setup-beam@v1.24.0")],
            &provider,
        )
        .unwrap();

        assert_eq!(resolution.updates.len(), 1);
        assert_eq!(resolution.updates[0].target.as_deref(), Some("v1.25.1"));
    }

    #[test]
    fn full_pin_style_does_not_convert_missing_floating_tag_without_fallback() {
        let temp = tempfile::tempdir().unwrap();
        let mut settings = settings(temp.path());
        settings.pin_style = PinStyle::Full;
        let provider = FakeProvider {
            tags: vec![tag("v1.24.0"), tag("v1.25.0")],
            calls: Cell::new(0),
        };

        let resolution = resolve_updates_with_provider(
            &settings,
            CacheState::prepare(&settings).unwrap(),
            &[reference("erlef/setup-beam@v1.24")],
            &provider,
        )
        .unwrap();

        assert!(resolution.updates.is_empty());
        assert_eq!(resolution.diagnostics[0].code, DiagnosticCode::RemoteReferenceMissing);
    }

    #[test]
    fn major_pin_style_converts_full_refs_to_major_float() {
        let temp = tempfile::tempdir().unwrap();
        let mut settings = settings(temp.path());
        settings.pin_style = PinStyle::Major;
        let provider = FakeProvider {
            tags: vec![tag("v4"), tag("v4.1.0"), tag("v4.2.0")],
            calls: Cell::new(0),
        };

        let resolution = resolve_updates_with_provider(
            &settings,
            CacheState::prepare(&settings).unwrap(),
            &[reference("actions/cache@v4.1.0")],
            &provider,
        )
        .unwrap();

        assert_eq!(resolution.updates.len(), 1);
        assert_eq!(resolution.updates[0].target.as_deref(), Some("v4"));
    }

    #[test]
    fn minor_pin_style_converts_full_refs_to_minor_float() {
        let temp = tempfile::tempdir().unwrap();
        let mut settings = settings(temp.path());
        settings.pin_style = PinStyle::Minor;
        let provider = FakeProvider {
            tags: vec![tag("v4.1.0"), tag("v4.2"), tag("v4.2.0")],
            calls: Cell::new(0),
        };

        let resolution = resolve_updates_with_provider(
            &settings,
            CacheState::prepare(&settings).unwrap(),
            &[reference("actions/cache@v4.1.0")],
            &provider,
        )
        .unwrap();

        assert_eq!(resolution.updates.len(), 1);
        assert_eq!(resolution.updates[0].target.as_deref(), Some("v4.2"));
    }

    #[test]
    fn pin_style_does_not_rewrite_to_missing_float_target() {
        let temp = tempfile::tempdir().unwrap();
        let mut settings = settings(temp.path());
        settings.pin_style = PinStyle::Major;
        let provider = BranchCheckingProvider {
            tags: vec![tag("v4.1.0"), tag("v4.2.0")],
            branch_exists: false,
            tag_calls: Cell::new(0),
            branch_calls: Cell::new(0),
        };

        let resolution = resolve_updates_with_provider(
            &settings,
            CacheState::prepare(&settings).unwrap(),
            &[reference("actions/cache@v4.1.0")],
            &provider,
        )
        .unwrap();

        assert!(resolution.updates.is_empty());
        assert!(
            resolution.diagnostics[0]
                .message
                .contains("pin-style target does not exist")
        );
    }

    #[test]
    fn selects_latest_same_major_tag_for_full_version() {
        let temp = tempfile::tempdir().unwrap();
        let settings = settings(temp.path());
        let provider = FakeProvider {
            tags: vec![tag("v3"), tag("v4"), tag("v4.0.1"), tag("v4.1.0"), tag("v5")],
            calls: Cell::new(0),
        };

        let resolution = resolve_updates_with_provider(
            &settings,
            CacheState::prepare(&settings).unwrap(),
            &[reference("actions/checkout@v4.0.1")],
            &provider,
        )
        .unwrap();

        assert_eq!(resolution.updates.len(), 1);
        assert_eq!(resolution.updates[0].target.as_deref(), Some("v4.1.0"));
        assert_eq!(provider.calls.get(), 1);
    }

    #[test]
    fn skips_non_candidate_refs_before_metadata_fetch() {
        let temp = tempfile::tempdir().unwrap();
        let settings = settings(temp.path());
        let provider = FakeProvider {
            tags: vec![tag("v4")],
            calls: Cell::new(0),
        };

        let resolution = resolve_updates_with_provider(
            &settings,
            CacheState::prepare(&settings).unwrap(),
            &[reference("actions/checkout@main")],
            &provider,
        )
        .unwrap();

        assert!(resolution.updates.is_empty());
        assert_eq!(provider.calls.get(), 0);
    }

    #[test]
    fn config_update_exclude_skips_matching_action_before_metadata_fetch() {
        let temp = tempfile::tempdir().unwrap();
        let mut settings = settings(temp.path());
        settings.update_exclude = vec!["actions/checkout".to_string()];
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

        assert!(resolution.updates.is_empty());
        assert_eq!(provider.calls.get(), 0);
    }

    #[test]
    fn inline_ignore_skips_update_before_metadata_fetch() {
        let temp = tempfile::tempdir().unwrap();
        let settings = settings(temp.path());
        let provider = FakeProvider {
            tags: vec![tag("v4.1.0")],
            calls: Cell::new(0),
        };
        let mut reference = reference("actions/checkout@v4");
        reference.update_ignored = true;

        let resolution = resolve_updates_with_provider(
            &settings,
            CacheState::prepare(&settings).unwrap(),
            &[reference],
            &provider,
        )
        .unwrap();

        assert!(resolution.updates.is_empty());
        assert_eq!(provider.calls.get(), 0);
    }

    #[test]
    fn latest_hash_pins_existing_major_only_tag_sha() {
        let temp = tempfile::tempdir().unwrap();
        let mut settings = settings(temp.path());
        settings.latest_hash = true;
        let provider = FakeProvider {
            tags: vec![
                tag_with_sha("v4", "1111111111111111111111111111111111111111"),
                tag_with_sha("v4.2.0", "2222222222222222222222222222222222222222"),
            ],
            calls: Cell::new(0),
        };

        let resolution = resolve_updates_with_provider(
            &settings,
            CacheState::prepare(&settings).unwrap(),
            &[reference("actions/checkout@v4")],
            &provider,
        )
        .unwrap();

        assert_eq!(
            resolution.updates[0].target.as_deref(),
            Some("1111111111111111111111111111111111111111")
        );
    }

    #[test]
    fn latest_hash_crosses_majors_for_tag_when_major_preservation_is_disabled() {
        let temp = tempfile::tempdir().unwrap();
        let mut settings = settings(temp.path());
        settings.latest_hash = true;
        settings.preserve_major = false;
        let provider = FakeProvider {
            tags: vec![
                tag_with_sha("v3", "3333333333333333333333333333333333333333"),
                tag_with_sha("v4", "4444444444444444444444444444444444444444"),
            ],
            calls: Cell::new(0),
        };

        let resolution = resolve_updates_with_provider(
            &settings,
            CacheState::prepare(&settings).unwrap(),
            &[reference("actions/checkout@v3")],
            &provider,
        )
        .unwrap();

        assert_eq!(
            resolution.updates[0].target.as_deref(),
            Some("4444444444444444444444444444444444444444")
        );
    }

    #[test]
    fn latest_hash_updates_sha_by_matching_tag_major() {
        let temp = tempfile::tempdir().unwrap();
        let mut settings = settings(temp.path());
        settings.latest_hash = true;
        let provider = FakeProvider {
            tags: vec![
                tag_with_sha("v4", "1111111111111111111111111111111111111111"),
                tag_with_sha("v4.2.0", "2222222222222222222222222222222222222222"),
                tag_with_sha("v5", "5555555555555555555555555555555555555555"),
            ],
            calls: Cell::new(0),
        };

        let resolution = resolve_updates_with_provider(
            &settings,
            CacheState::prepare(&settings).unwrap(),
            &[reference("actions/checkout@1111111111111111111111111111111111111111")],
            &provider,
        )
        .unwrap();

        assert_eq!(
            resolution.updates[0].target.as_deref(),
            Some("2222222222222222222222222222222222222222")
        );
    }

    #[test]
    fn latest_hash_crosses_majors_for_sha_when_major_preservation_is_disabled() {
        let temp = tempfile::tempdir().unwrap();
        let mut settings = settings(temp.path());
        settings.latest_hash = true;
        settings.preserve_major = false;
        let provider = FakeProvider {
            tags: vec![
                tag_with_sha("v3", "3333333333333333333333333333333333333333"),
                tag_with_sha("v4", "4444444444444444444444444444444444444444"),
            ],
            calls: Cell::new(0),
        };

        let resolution = resolve_updates_with_provider(
            &settings,
            CacheState::prepare(&settings).unwrap(),
            &[reference("actions/checkout@3333333333333333333333333333333333333333")],
            &provider,
        )
        .unwrap();

        assert_eq!(
            resolution.updates[0].target.as_deref(),
            Some("4444444444444444444444444444444444444444")
        );
    }

    #[test]
    fn latest_hash_updates_unmatched_valid_sha_across_majors() {
        let temp = tempfile::tempdir().unwrap();
        let mut settings = settings(temp.path());
        settings.latest_hash = true;
        let provider = CommitCheckingProvider {
            tags: vec![tag_with_sha("v4.2.0", "2222222222222222222222222222222222222222")],
            commit_exists: true,
            tag_calls: Cell::new(0),
            commit_calls: Cell::new(0),
        };

        let resolution = resolve_updates_with_provider(
            &settings,
            CacheState::prepare(&settings).unwrap(),
            &[reference("actions/checkout@9999999999999999999999999999999999999999")],
            &provider,
        )
        .unwrap();

        assert_eq!(
            resolution.updates[0].target.as_deref(),
            Some("2222222222222222222222222222222222222222")
        );
        assert_eq!(provider.commit_calls.get(), 1);
    }

    #[test]
    fn reuses_cached_tags() {
        let temp = tempfile::tempdir().unwrap();
        let settings = settings(temp.path());
        let provider = FakeProvider {
            tags: vec![tag("v4.0.0"), tag("v4.2.0")],
            calls: Cell::new(0),
        };

        let cache = CacheState::prepare(&settings).unwrap();
        let first = resolve_updates_with_provider(&settings, cache, &[reference("actions/checkout@v4.0.0")], &provider)
            .unwrap();
        assert_eq!(first.cache.misses, 1);

        let second = resolve_updates_with_provider(
            &settings,
            CacheState::prepare(&settings).unwrap(),
            &[reference("actions/checkout@v4.0.0")],
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
            tags: vec![tag("v4.0.0"), tag("v4.2.0")],
            calls: Cell::new(0),
        };

        let resolution = resolve_updates_with_provider(
            &settings,
            CacheState::prepare(&settings).unwrap(),
            &[reference("actions/checkout@v4.0.0")],
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
                    "tags": [tag("v4.0.0"), tag("v4.2.0")]
                }
            })
            .to_string(),
        )
        .unwrap();
        let provider = FailingProvider { calls: Cell::new(0) };

        let resolution = resolve_updates_with_provider(
            &settings,
            CacheState::prepare(&settings).unwrap(),
            &[reference("actions/checkout@v4.0.0")],
            &provider,
        )
        .unwrap();

        assert_eq!(resolution.cache.stale_hits, 1);
        assert_eq!(resolution.updates[0].target.as_deref(), Some("v4.2.0"));
        assert!(resolution.diagnostics[0].message.contains("using stale metadata"));
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
    fn reports_missing_full_current_ref() {
        let temp = tempfile::tempdir().unwrap();
        let settings = settings(temp.path());
        let provider = FakeProvider {
            tags: vec![tag("v4.1.0")],
            calls: Cell::new(0),
        };

        let resolution: MetadataResolution = resolve_updates_with_provider(
            &settings,
            CacheState::prepare(&settings).unwrap(),
            &[reference("actions/checkout@v4.0.0")],
            &provider,
        )
        .unwrap();

        assert!(resolution.updates.is_empty());
        assert_eq!(resolution.diagnostics.len(), 1);
    }

    #[test]
    fn branch_backed_semver_ref_is_not_reported_missing() {
        let temp = tempfile::tempdir().unwrap();
        let settings = settings(temp.path());
        let provider = BranchCheckingProvider {
            tags: Vec::new(),
            branch_exists: true,
            tag_calls: Cell::new(0),
            branch_calls: Cell::new(0),
        };

        let resolution = resolve_updates_with_provider(
            &settings,
            CacheState::prepare(&settings).unwrap(),
            &[reference("ruby/setup-ruby@v1")],
            &provider,
        )
        .unwrap();

        assert!(resolution.updates.is_empty());
        assert!(resolution.diagnostics.is_empty());
        assert_eq!(provider.branch_calls.get(), 1);
    }

    #[test]
    fn validation_accepts_semver_looking_branches_used_by_corpus_actions() {
        for raw in [
            "ruby/setup-ruby@v1",
            "rust-lang/crates-io-auth-action@v1",
            "arduino/setup-task@v2",
            "arduino/setup-task@v3",
        ] {
            let temp = tempfile::tempdir().unwrap();
            let mut settings = settings(temp.path());
            settings.validate = true;
            settings.missing_ref = MissingRefPolicy::Error;
            let provider = BranchCheckingProvider {
                tags: Vec::new(),
                branch_exists: true,
                tag_calls: Cell::new(0),
                branch_calls: Cell::new(0),
            };

            let resolution = resolve_updates_with_provider(
                &settings,
                CacheState::prepare(&settings).unwrap(),
                &[reference(raw)],
                &provider,
            )
            .unwrap();

            assert!(resolution.diagnostics.is_empty(), "false missing diagnostic for {raw}");
            assert_eq!(provider.branch_calls.get(), 1);
        }
    }

    #[test]
    fn branch_backed_semver_ref_wins_over_matching_tag_updates() {
        let temp = tempfile::tempdir().unwrap();
        let settings = settings(temp.path());
        let provider = BranchCheckingProvider {
            tags: vec![tag("v1.2.0")],
            branch_exists: true,
            tag_calls: Cell::new(0),
            branch_calls: Cell::new(0),
        };

        let resolution = resolve_updates_with_provider(
            &settings,
            CacheState::prepare(&settings).unwrap(),
            &[reference("ruby/setup-ruby@v1")],
            &provider,
        )
        .unwrap();

        assert!(resolution.updates.is_empty());
        assert!(resolution.diagnostics.is_empty());
        assert_eq!(provider.branch_calls.get(), 1);
    }

    #[test]
    fn latest_hash_does_not_pin_missing_floating_tag_to_full_tag_sha() {
        let temp = tempfile::tempdir().unwrap();
        let mut settings = settings(temp.path());
        settings.latest_hash = true;
        let provider = BranchCheckingProvider {
            tags: vec![tag_with_sha("v1.2.0", "2222222222222222222222222222222222222222")],
            branch_exists: false,
            tag_calls: Cell::new(0),
            branch_calls: Cell::new(0),
        };

        let resolution = resolve_updates_with_provider(
            &settings,
            CacheState::prepare(&settings).unwrap(),
            &[reference("actions/example@v1")],
            &provider,
        )
        .unwrap();

        assert!(resolution.updates.is_empty());
        assert!(resolution.diagnostics.is_empty());
    }

    #[test]
    fn successful_missing_ref_fallback_is_not_a_validation_failure() {
        let temp = tempfile::tempdir().unwrap();
        let mut settings = settings(temp.path());
        settings.pin_style = PinStyle::Full;
        settings.missing_ref = MissingRefPolicy::Fallback;
        settings.validate = true;
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
        assert!(
            !resolution
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.category == DiagnosticCategory::Validation)
        );
        assert_eq!(exit_code_for_resolution(&settings, &resolution), None);
    }

    #[test]
    fn mixed_mode_fallback_rewrites_missing_float_without_failure() {
        let temp = tempfile::tempdir().unwrap();
        let mut settings = settings(temp.path());
        settings.update_mode = UpdateMode::Latest;
        settings.pin_floating_to_sha = true;
        settings.missing_ref = MissingRefPolicy::Fallback;
        settings.validate = true;
        let provider = FakeProvider {
            tags: vec![tag("v8.0.0")],
            calls: Cell::new(0),
        };

        let resolution = resolve_updates_with_provider(
            &settings,
            CacheState::prepare(&settings).unwrap(),
            &[reference("astral-sh/setup-uv@v8")],
            &provider,
        )
        .unwrap();

        assert_eq!(resolution.updates[0].target.as_deref(), Some("v8.0.0"));
        assert!(
            !resolution
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == DiagnosticCode::RemoteReferenceMissing)
        );
        assert_eq!(exit_code_for_resolution(&settings, &resolution), None);
    }

    #[test]
    fn missing_ref_fallback_counts_update() {
        let temp = tempfile::tempdir().unwrap();
        let mut settings = settings(temp.path());
        settings.pin_style = PinStyle::Full;
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
            &[reference("actions/checkout@v4.0.0")],
            &provider,
        )
        .unwrap();

        assert_eq!(exit_code_for_resolution(&settings, &resolution), Some(1));
    }

    #[test]
    fn missing_ref_error_returns_full_resolution_outside_check_mode() {
        let temp = tempfile::tempdir().unwrap();
        let mut settings = settings(temp.path());
        settings.missing_ref = MissingRefPolicy::Error;
        let provider = FakeProvider {
            tags: vec![tag("v4.1.0")],
            calls: Cell::new(0),
        };

        let resolution = resolve_updates_with_provider(
            &settings,
            CacheState::prepare(&settings).unwrap(),
            &[reference("actions/checkout@v4.0.0")],
            &provider,
        )
        .unwrap();

        assert_eq!(resolution.diagnostics[0].code, DiagnosticCode::RemoteReferenceMissing);
        assert_eq!(exit_code_for_resolution(&settings, &resolution), Some(1));
    }

    #[test]
    fn missing_ref_exit_uses_diagnostic_code_instead_of_message_text() {
        let temp = tempfile::tempdir().unwrap();
        let mut settings = settings(temp.path());
        settings.missing_ref = MissingRefPolicy::Error;
        let provider = FakeProvider {
            tags: Vec::new(),
            calls: Cell::new(0),
        };
        let mut resolution = resolve_updates_with_provider(
            &settings,
            CacheState::prepare(&settings).unwrap(),
            &[reference("actions/checkout@v4.0.0")],
            &provider,
        )
        .unwrap();
        resolution.diagnostics[0].message = "localized missing reference message".to_string();

        assert_eq!(exit_code_for_resolution(&settings, &resolution), Some(1));
    }

    #[test]
    fn duplicate_missing_reference_at_same_location_is_reported_once() {
        let temp = tempfile::tempdir().unwrap();
        let settings = settings(temp.path());
        let provider = FakeProvider {
            tags: Vec::new(),
            calls: Cell::new(0),
        };
        let missing = reference("spikard/action@v1.0.0");

        let resolution = resolve_updates_with_provider(
            &settings,
            CacheState::prepare(&settings).unwrap(),
            &[missing.clone(), missing],
            &provider,
        )
        .unwrap();

        assert_eq!(
            resolution
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code == DiagnosticCode::RemoteReferenceMissing)
                .count(),
            1
        );
    }

    #[test]
    fn local_reference_cannot_escape_repository_to_existing_target() {
        let temp = tempfile::tempdir().unwrap();
        let repository = temp.path().join("repository");
        let external = temp.path().join("external");
        std::fs::create_dir_all(repository.join(".git")).unwrap();
        std::fs::create_dir_all(repository.join(".github/workflows")).unwrap();
        std::fs::create_dir_all(&external).unwrap();
        std::fs::write(external.join("action.yml"), "name: external").unwrap();
        let mut settings = settings(&repository);
        settings.validate = true;
        let provider = FakeProvider {
            tags: Vec::new(),
            calls: Cell::new(0),
        };
        let mut escaped = reference("../external");
        escaped.file = repository.join(".github/workflows/ci.yml").display().to_string();

        let resolution = resolve_updates_with_provider(
            &settings,
            CacheState::prepare(&settings).unwrap(),
            &[escaped],
            &provider,
        )
        .unwrap();

        assert_eq!(
            resolution.diagnostics[0].code,
            DiagnosticCode::LocalReferenceEscapesRepository
        );
    }

    #[cfg(unix)]
    #[test]
    fn local_reference_cannot_escape_repository_through_symlink() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let repository = temp.path().join("repository");
        let external = temp.path().join("external");
        std::fs::create_dir_all(repository.join(".git")).unwrap();
        std::fs::create_dir_all(repository.join(".github/workflows")).unwrap();
        std::fs::create_dir_all(repository.join(".github/actions")).unwrap();
        std::fs::create_dir_all(&external).unwrap();
        std::fs::write(external.join("action.yml"), "name: external").unwrap();
        symlink(&external, repository.join(".github/actions/external")).unwrap();
        let mut settings = settings(&repository);
        settings.validate = true;
        let provider = FakeProvider {
            tags: Vec::new(),
            calls: Cell::new(0),
        };
        let mut escaped = reference("./.github/actions/external");
        escaped.file = repository.join(".github/workflows/ci.yml").display().to_string();

        let resolution = resolve_updates_with_provider(
            &settings,
            CacheState::prepare(&settings).unwrap(),
            &[escaped],
            &provider,
        )
        .unwrap();

        assert_eq!(
            resolution.diagnostics[0].code,
            DiagnosticCode::LocalReferenceEscapesRepository
        );
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
            vec![tag_with_sha("v1", "abc123"), tag_with_sha("v1.2.0", "def456")]
        );
    }

    #[test]
    fn parses_peeled_annotated_tag_commits() {
        let tags = parse_ls_remote_tags(
            "tag-object\trefs/tags/v1\n\
             commit-sha\trefs/tags/v1^{}\n",
        )
        .unwrap();

        assert_eq!(tags, vec![tag_with_sha("v1", "commit-sha")]);
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

    #[test]
    fn github_provider_uses_hardened_redirect_policy() {
        let temp = tempfile::tempdir().unwrap();
        let settings = settings(temp.path());
        let provider = super::GitHubRestProvider::new(&settings);

        assert!(provider.agent.config().https_only());
        assert_eq!(provider.agent.config().max_redirects(), super::MAX_GITHUB_REDIRECTS);
        assert_eq!(
            provider.agent.config().redirect_auth_headers(),
            ureq::config::RedirectAuthHeaders::SameHost
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

    #[test]
    fn latest_with_floating_pinning_updates_tags_shas_and_branches_additively() {
        let temp = tempfile::tempdir().unwrap();
        let mut settings = settings(temp.path());
        settings.update_mode = UpdateMode::Latest;
        settings.pin_floating_to_sha = true;
        let provider = BranchCheckingProvider {
            tags: vec![
                tag_with_sha("v1.0.0", "1111111111111111111111111111111111111111"),
                tag_with_sha("v1.1.0", "2222222222222222222222222222222222222222"),
            ],
            branch_exists: true,
            tag_calls: Cell::new(0),
            branch_calls: Cell::new(0),
        };

        let resolution = resolve_updates_with_provider(
            &settings,
            CacheState::prepare(&settings).unwrap(),
            &[
                reference("owner/action@v1.0.0"),
                reference("owner/action@1111111111111111111111111111111111111111"),
                reference("owner/action@main"),
            ],
            &provider,
        )
        .unwrap();

        assert_eq!(resolution.updates.len(), 3);
        assert_eq!(resolution.updates[0].target.as_deref(), Some("v1.1.0"));
        assert_eq!(
            resolution.updates[1].target.as_deref(),
            Some("2222222222222222222222222222222222222222")
        );
        assert_eq!(
            resolution.updates[2].target.as_deref(),
            Some("0123456789abcdef0123456789abcdef01234567")
        );
    }

    #[test]
    fn pin_floating_to_sha_rewrites_branch_to_sha() {
        use super::RefKind;
        let temp = tempfile::tempdir().unwrap();
        let mut settings = settings(temp.path());
        settings.pin_floating_to_sha = true;
        let provider = BranchCheckingProvider {
            tags: vec![tag("v1.0.0")],
            branch_exists: true,
            tag_calls: Cell::new(0),
            branch_calls: Cell::new(0),
        };

        let resolution = resolve_updates_with_provider(
            &settings,
            CacheState::prepare(&settings).unwrap(),
            &[reference("kreuzberg-dev/actions/lint-docs@main")],
            &provider,
        )
        .unwrap();

        assert_eq!(
            resolution.updates.len(),
            1,
            "expected branch ref to be rewritten to SHA"
        );
        assert_eq!(
            resolution.updates[0].target.as_deref(),
            Some("0123456789abcdef0123456789abcdef01234567")
        );
        assert_eq!(provider.branch_calls.get(), 1);
        assert_eq!(
            resolution.resolved_ref_kinds,
            vec![(0, RefKind::Branch)],
            "branch ref should be reclassified"
        );
    }

    #[test]
    fn pin_floating_to_sha_rewrites_non_semver_tag_to_sha() {
        use super::RefKind;
        let temp = tempfile::tempdir().unwrap();
        let mut settings = settings(temp.path());
        settings.pin_floating_to_sha = true;
        let provider = FakeProvider {
            tags: vec![tag("stable"), tag("nightly")],
            calls: Cell::new(0),
        };

        let resolution = resolve_updates_with_provider(
            &settings,
            CacheState::prepare(&settings).unwrap(),
            &[reference("dtolnay/rust-toolchain@stable")],
            &provider,
        )
        .unwrap();

        assert_eq!(resolution.updates.len(), 1);
        assert_eq!(resolution.updates[0].current, "stable");
        assert_eq!(resolution.updates[0].target.as_deref(), Some("sha-stable"));
        assert_eq!(resolution.resolved_ref_kinds, vec![(0, RefKind::NonSemverTag)],);
    }

    #[test]
    fn default_scan_does_not_fetch_metadata_for_floating_refs() {
        let temp = tempfile::tempdir().unwrap();
        let settings = settings(temp.path());
        let provider = BranchCheckingProvider {
            tags: vec![tag("v1.0.0")],
            branch_exists: true,
            tag_calls: Cell::new(0),
            branch_calls: Cell::new(0),
        };

        let resolution = resolve_updates_with_provider(
            &settings,
            CacheState::prepare(&settings).unwrap(),
            &[reference("kreuzberg-dev/actions/lint-docs@main")],
            &provider,
        )
        .unwrap();

        assert!(resolution.updates.is_empty());
        assert!(resolution.resolved_ref_kinds.is_empty());
        assert_eq!(
            provider.tag_calls.get(),
            0,
            "default scan must not fetch tags for branch refs"
        );
        assert_eq!(provider.branch_calls.get(), 0);
    }

    #[test]
    fn validate_reports_missing_tag_when_tag_absent_upstream() {
        let temp = tempfile::tempdir().unwrap();
        let mut settings = settings(temp.path());
        settings.validate = true;
        let provider = FakeProvider {
            tags: vec![tag("v4.0.0")],
            calls: Cell::new(0),
        };

        let resolution = resolve_updates_with_provider(
            &settings,
            CacheState::prepare(&settings).unwrap(),
            &[reference("actions/checkout@v3.5.0")],
            &provider,
        )
        .unwrap();

        assert!(
            resolution
                .diagnostics
                .iter()
                .any(|d| d.message.contains("remote ref no longer exists")),
            "validate should report missing upstream tag: {:?}",
            resolution.diagnostics
        );
    }

    #[test]
    fn validate_passes_when_tag_exists_upstream() {
        let temp = tempfile::tempdir().unwrap();
        let mut settings = settings(temp.path());
        settings.validate = true;
        let provider = FakeProvider {
            tags: vec![tag("v4.0.0"), tag("v4.1.0")],
            calls: Cell::new(0),
        };

        let resolution = resolve_updates_with_provider(
            &settings,
            CacheState::prepare(&settings).unwrap(),
            &[reference("actions/checkout@v4.0.0")],
            &provider,
        )
        .unwrap();

        assert!(
            !resolution
                .diagnostics
                .iter()
                .any(|d| d.message.contains("remote ref no longer exists")),
            "validate should not report present tag missing: {:?}",
            resolution.diagnostics
        );
    }

    #[test]
    fn sha_advisory_emitted_when_newer_sha_available_without_latest_hash() {
        let temp = tempfile::tempdir().unwrap();
        let settings = settings(temp.path());
        let pinned_sha = "1111111111111111111111111111111111111111";
        let newer_sha = "2222222222222222222222222222222222222222";
        let provider = FakeProvider {
            tags: vec![
                RemoteTag {
                    name: "v4.0.0".to_string(),
                    sha: pinned_sha.to_string(),
                },
                RemoteTag {
                    name: "v4.1.0".to_string(),
                    sha: newer_sha.to_string(),
                },
            ],
            calls: Cell::new(0),
        };

        let raw = format!("actions/checkout@{pinned_sha}");
        let resolution = resolve_updates_with_provider(
            &settings,
            CacheState::prepare(&settings).unwrap(),
            &[reference(&raw)],
            &provider,
        )
        .unwrap();

        assert!(
            resolution.updates.is_empty(),
            "SHA refs must not produce updates without --latest-hash"
        );
        assert!(
            resolution
                .diagnostics
                .iter()
                .any(|d| d.message.contains("--latest-hash")),
            "expected SHA advisory diagnostic, got {:?}",
            resolution.diagnostics
        );
    }
}
