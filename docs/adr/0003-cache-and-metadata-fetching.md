# ADR 0003: Cache And Metadata Fetching

Status: Accepted for v0.1.1 implementation

## Context

The tool is expected to run in pre-commit hooks, where repeated network and git
operations would be too costly. It must fetch only the metadata needed for the
requested update mode and cache results predictably.

## Decision

The default cache directory is the platform cache directory with
`gh-actions-updater` appended. Users may override it with `--cache-dir` or
`GHAU_CACHE_DIR`.

TTL is configurable with `[cache].ttl`, `GHAU_CACHE_TTL`, or `--cache-ttl`.
`--cache-ttl` has highest precedence.

Cache flag behavior:

- `--no-cache` disables reads and writes
- `--refresh-cache` ignores cache reads and writes fresh entries after fetch
- `--cache-ttl 0` treats every entry as stale
- `--cache-ttl never` treats existing entries as fresh until explicit refresh

Freshness behavior:

| State | Behavior |
| --- | --- |
| Fresh cache hit | Use cached metadata. |
| Stale cache hit | Refresh conditionally. If refresh fails, report-only mode may use stale data with a warning. |
| Stale cache hit with `--check` or `--update` | Refresh conditionally. If refresh fails, exit `4`. |
| `--refresh-cache` | Skip cache reads, fetch metadata, write fresh entries, or exit `4`. |
| `--no-cache` | Fetch metadata without reading or writing cache, or exit `4`. |

Cache keys use `blake3` over:

- GitHub API host
- action owner/repo
- lookup mode: tags or hash
- relevant auth identity fingerprint

Latest-tag cached values include the metadata provider, API host, auth
fingerprint, tag lists, ETags, and timestamps. Hash-mode cache values add
per-SHA commit-existence metadata when the current SHA cannot be matched to a
fetched tag.

Metadata fetching strategy:

- latest-tag mode fetches tag metadata only
- hash mode fetches tag metadata, selects the target tag, and uses the selected
  tag's commit SHA as the update target
- hash mode fetches per-SHA commit metadata only when the current ref is a SHA
  that does not match any fetched semver tag
- full history is never fetched for tag-only checks
- repositories are never cloned
- GitHub REST API with conditional ETag requests is the default metadata source
- `git ls-remote --tags --refs` may be used for public or git-accessible
  repositories when it avoids expensive API pagination

Authentication is read from `--github-token`, `GHAU_GITHUB_TOKEN`,
`GITHUB_TOKEN`, then `GH_TOKEN`. `--github-api-url` supports GitHub Enterprise
or compatible hosts.

## Consequences

Pre-commit runs stay cheap. Hash mode is explicitly more expensive, and cache
identity includes enough context to avoid cross-host or cross-token confusion.
