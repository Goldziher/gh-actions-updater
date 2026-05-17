use crate::config::{CacheTtl, Settings};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize)]
pub struct CacheReport {
    pub enabled: bool,
    pub fresh_hits: usize,
    pub stale_hits: usize,
    pub misses: usize,
    pub refreshes: usize,
}

#[derive(Debug, Clone)]
pub struct CacheState {
    pub report: CacheReport,
    pub dir: PathBuf,
    pub key_prefix: String,
}

impl CacheState {
    pub fn prepare(settings: &Settings) -> Result<Self> {
        if settings.cache_enabled {
            fs::create_dir_all(&settings.cache_dir).with_context(|| {
                format!(
                    "failed to create cache directory {}",
                    settings.cache_dir.display()
                )
            })?;
        }

        let _ = match settings.cache_ttl {
            CacheTtl::Seconds(seconds) => seconds,
            CacheTtl::Never => u64::MAX,
        };

        Ok(Self {
            report: CacheReport {
                enabled: settings.cache_enabled,
                fresh_hits: 0,
                stale_hits: 0,
                misses: 0,
                refreshes: 0,
            },
            dir: settings.cache_dir.clone(),
            key_prefix: cache_key(CacheKeyParts {
                api_host: &settings.github_api_url,
                owner: "",
                repo: "",
                lookup_mode: if settings.latest_hash { "hash" } else { "tags" },
                auth_fingerprint: if settings.github_token_present {
                    "present"
                } else {
                    "anonymous"
                },
            }),
        })
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub struct CacheKeyParts<'a> {
    pub api_host: &'a str,
    pub owner: &'a str,
    pub repo: &'a str,
    pub lookup_mode: &'a str,
    pub auth_fingerprint: &'a str,
}

pub fn cache_key(parts: CacheKeyParts<'_>) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(parts.api_host.as_bytes());
    hasher.update(b"\0");
    hasher.update(parts.owner.as_bytes());
    hasher.update(b"\0");
    hasher.update(parts.repo.as_bytes());
    hasher.update(b"\0");
    hasher.update(parts.lookup_mode.as_bytes());
    hasher.update(b"\0");
    hasher.update(parts.auth_fingerprint.as_bytes());
    hasher.finalize().to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::{CacheKeyParts, cache_key};

    #[test]
    fn cache_key_includes_repo_mode_and_auth_identity() {
        let base = CacheKeyParts {
            api_host: "https://api.github.com",
            owner: "actions",
            repo: "checkout",
            lookup_mode: "tags",
            auth_fingerprint: "anonymous",
        };
        let other_repo = CacheKeyParts {
            repo: "setup-node",
            ..base
        };
        let other_mode = CacheKeyParts {
            lookup_mode: "hash",
            ..base
        };
        let other_auth = CacheKeyParts {
            auth_fingerprint: "present",
            ..base
        };

        assert_ne!(cache_key(base), cache_key(other_repo));
        assert_ne!(cache_key(base), cache_key(other_mode));
        assert_ne!(cache_key(base), cache_key(other_auth));
    }
}
