---
priority: critical
---

# Metadata, Cache, And Performance

`gau` is intended for the commit step and workspace-wide dry runs. It must be fast by
default and careful with network access.

## Metadata Fetching

- Fetch tag metadata for update candidates only.
- Do not clone repositories for normal tag checks.
- Use GitHub REST with conditional requests where possible.
- Use `git ls-remote --tags --refs` only as a cheap fallback or optimization.
- Fetch commit or branch metadata only when needed by latest-hash or
  missing-ref handling.
- Deduplicate metadata work by unique repository/ref/mode before fetching.

## Cache

- Store cache under the platform cache directory with `gh-actions-updater`
  appended unless overridden.
- Cache keys must use BLAKE3 and include host, owner, repo, lookup mode, and
  relevant auth identity.
- `--no-cache` disables both reads and writes.
- `--refresh-cache` skips reads and writes fresh entries after successful fetch.
- `--cache-ttl 0` treats entries as stale.
- `--cache-ttl never` treats existing entries as fresh until refresh is forced.
- Corrupt cache entries should be ignored and replaced when network fetches
  succeed.

## Parallelism

- Use Rayon for CPU/file work.
- Default to Rayon available-core behavior.
- `--threads <N>` and `[performance] threads = N` are overrides; reject `0`.
- Keep output deterministic by sorting discovered files and stable report data.
- Do not parallelize writes to the same file. Group replacements per file and
  apply them deterministically.

## Hot-Path Patterns

- Use `ahash` for hot maps/sets.
- Use `memchr`/byte prechecks before YAML parsing where possible.
- Avoid ad hoc string parsing where structured parser data is available.
- Keep expensive schema validation optional via `--no-schema-validation`.
