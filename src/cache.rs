use crate::config::{CacheTtl, Settings};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

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
    pub enabled: bool,
    pub refresh: bool,
    pub ttl: CacheTtl,
}

impl CacheState {
    pub fn prepare(settings: &Settings) -> Result<Self> {
        if settings.cache_enabled {
            fs::create_dir_all(&settings.cache_dir)
                .with_context(|| format!("failed to create cache directory {}", settings.cache_dir.display()))?;
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
            enabled: settings.cache_enabled,
            refresh: settings.refresh_cache,
            ttl: settings.cache_ttl.clone(),
        })
    }

    pub fn read_json<T: for<'de> Deserialize<'de>>(&mut self, key: &str) -> Result<CacheLookup<T>> {
        if !self.enabled || self.refresh {
            return Ok(CacheLookup::Miss);
        }

        let path = self.entry_path(key);
        let content = match fs::read_to_string(&path) {
            Ok(content) => content,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                self.report.misses += 1;
                return Ok(CacheLookup::Miss);
            }
            Err(error) => {
                return Err(error).with_context(|| format!("failed to read {}", path.display()));
            }
        };

        let entry: CacheEntry<T> = match serde_json::from_str(&content) {
            Ok(entry) => entry,
            Err(_) => return Ok(CacheLookup::Corrupt),
        };
        if self.is_fresh(entry.fetched_at) {
            self.report.fresh_hits += 1;
            Ok(CacheLookup::Fresh(entry.value))
        } else {
            self.report.stale_hits += 1;
            Ok(CacheLookup::Stale(entry.value))
        }
    }

    pub fn write_json<T: Serialize>(&mut self, key: &str, value: &T) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }

        fs::create_dir_all(&self.dir)
            .with_context(|| format!("failed to create cache directory {}", self.dir.display()))?;
        let path = self.entry_path(key);
        let entry = CacheEntry {
            fetched_at: unix_now(),
            value,
        };
        let temp_path = self
            .dir
            .join(format!("{key}.{}.{}.json.tmp", std::process::id(), unix_now()));
        fs::write(&temp_path, serde_json::to_vec(&entry)?)
            .with_context(|| format!("failed to write cache entry {}", temp_path.display()))?;
        fs::rename(&temp_path, &path).with_context(|| format!("failed to write cache entry {}", path.display()))?;
        self.report.refreshes += 1;
        Ok(())
    }

    fn entry_path(&self, key: &str) -> PathBuf {
        self.dir.join(format!("{key}.json"))
    }

    fn is_fresh(&self, fetched_at: u64) -> bool {
        match self.ttl {
            CacheTtl::Never => true,
            CacheTtl::Seconds(0) => false,
            CacheTtl::Seconds(ttl) => unix_now().saturating_sub(fetched_at) <= ttl,
        }
    }
}

#[derive(Debug)]
pub enum CacheLookup<T> {
    Fresh(T),
    Stale(T),
    Corrupt,
    Miss,
}

#[derive(Debug, Deserialize, Serialize)]
struct CacheEntry<T> {
    fetched_at: u64,
    value: T,
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

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
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
