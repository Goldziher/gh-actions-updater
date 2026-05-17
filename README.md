# gh-actions-updater

Version: `0.1.0`

`gh-actions-updater` is a Rust 2024 CLI for finding and updating GitHub Actions
references in repository workflow and action metadata files. The v0.1.0 surface
is designed around fast local and pre-commit use: scan GitHub Actions YAML,
resolve available tag updates with a cache, and only perform heavier
commit-hash lookup when requested.

This README defines the intended v0.1.0 CLI contract. Implementation is being
built iteratively.

## Default Behavior

By default, the tool:

- scans `.github/workflows/**/*.yml` and `.github/workflows/**/*.yaml`
- scans repository `action.yml` and `action.yaml` files
- scans `.github/actions/**/action.yml` and `.github/actions/**/action.yaml`
- reports remote action and reusable workflow references from supported `uses:`
  entries
- checks latest tag metadata when cache entries are stale
- uses the global cache unless disabled
- does not rewrite workflow files unless `--update` is passed

Local actions such as `./.github/actions/build`, Docker image references, and
local reusable workflow calls are reported as non-updatable.

## Usage

```bash
gh-actions-updater [OPTIONS] [PATHS]...
```

`PATHS` may be files, directories, or glob patterns. When no paths are supplied,
the default scan set is:

- `.github/workflows/**/*.yml`
- `.github/workflows/**/*.yaml`
- `.github/actions/**/action.yml`
- `.github/actions/**/action.yaml`
- `action.yml`
- `action.yaml`

## CLI Flags

| Flag | Description |
| --- | --- |
| `-c, --config <PATH>` | Load a specific TOML config file instead of discovering one. |
| `--include <GLOB>` | Add an include glob. Can be repeated. |
| `--exclude <GLOB>` | Add an exclude glob. Can be repeated. |
| `--cache-dir <PATH>` | Override the global cache directory for this run. |
| `--cache-ttl <DURATION>` | Override cache TTL, for example `15m`, `6h`, `7d`, or `never`. |
| `--refresh-cache` | Ignore existing cache entries and write fresh metadata after fetching. |
| `--no-cache` | Disable cache reads and writes for this run. |
| `--update` | Rewrite supported workflow refs to the selected target version. |
| `--latest-hash` | Resolve update targets to commit SHAs instead of latest tags. |
| `--missing-ref <warn\|error\|ignore\|fallback>` | Behavior when the current remote tag or SHA no longer exists. |
| `--check` | Exit non-zero when updates are available. Does not rewrite files. |
| `--dry-run` | Show planned changes without writing files. Implied when `--update` is absent. |
| `--diff` | Show unified diffs for proposed workflow updates. |
| `--format <human\|json>` | Select human-readable or JSON output. Default: `human`. |
| `-q, --quiet` | Suppress non-error human output. |
| `-v, --verbose` | Print cache, network, and file discovery details. |
| `--color <auto\|always\|never>` | Control color in human output. Default: `auto`. |
| `--github-token <TOKEN>` | Use a GitHub token for metadata requests. Environment variables are preferred. |
| `--github-api-url <URL>` | Use a GitHub-compatible API endpoint. Default: `https://api.github.com`. |
| `--strict-schema` | Treat GitHub workflow schema validation diagnostics as failures. |
| `--no-schema-validation` | Skip workflow schema validation and only scan `uses:` refs. |
| `-h, --help` | Print help. |
| `-V, --version` | Print version. |

`--latest-hash` affects only update target resolution. Normal latest-tag checks
fetch tag metadata only.

## Configuration

The default config file is `.gh-actions-updater.toml`. Discovery starts in the
current directory, walks up to the repository root, and stops at the first match.
`--config` disables discovery.

```toml
[scan]
include = [
  ".github/workflows/**/*.yml",
  ".github/workflows/**/*.yaml",
  ".github/actions/**/action.yml",
  ".github/actions/**/action.yaml",
  "action.yml",
  "action.yaml",
]
exclude = []

[cache]
enabled = true
ttl = "6h"

[update]
mode = "latest-tag"
include_prereleases = false
preserve_major = true
missing_ref = "warn"

[output]
format = "human"
color = "auto"

[github]
api_url = "https://api.github.com"
```

Precedence is: CLI flags, environment variables, config file, defaults.
Output presentation flags such as `--quiet`, `--verbose`, and `--diff` are
CLI-only for v0.1.0.

Supported environment variables:

- `GHAU_GITHUB_TOKEN`
- `GITHUB_TOKEN`
- `GH_TOKEN`
- `GHAU_CACHE_DIR`
- `GHAU_CACHE_TTL`

## Update Semantics

The default update mode is `latest-tag`.

Supported `uses:` locations:

- workflow step actions: `jobs.<job_id>.steps[*].uses`
- reusable workflow calls: `jobs.<job_id>.uses`
- composite action step actions: `runs.steps[*].uses` when
  `runs.using = "composite"`
- `owner/repo@v4` updates within the same major tag line when semver-like tags
  are available.
- `owner/repo@v4.1.0` updates within the same major version.
- reusable workflows such as
  `owner/repo/.github/workflows/reusable.yml@v1` follow the same tag/hash
  update policy as actions.
- prerelease tags are ignored unless configured.
- branch refs such as `@main` are reported but not updated by default.
- immutable 40-character SHA refs are reported but not updated by default.
- non-semver tag sets are reported but not updated by default.

Deleted or missing refs are handled separately from normal update checks. The
default `missing_ref = "warn"` reports the missing ref but does not rewrite it.

Missing-ref policies:

| Policy | Behavior |
| --- | --- |
| `warn` | Report the missing tag or SHA and continue. |
| `error` | Treat the missing ref as a failure; exits `1` in `--check`, otherwise exits `4`. |
| `ignore` | Suppress missing-ref diagnostics for intentionally unavailable refs. |
| `fallback` | Select the normal update target and allow `--update` to rewrite to it. |

The updater must not silently rewrite a deleted tag unless the effective policy
is `fallback`.

`--latest-hash` first selects the same tag that latest-tag mode would select,
then pins the workflow to the commit SHA behind that selected tag. It does not
jump to the action repository default branch. It is intended for users who
prefer immutable CI dependencies while keeping the same version-selection
policy.

## Schemas

Workflow files are validated against the SchemaStore GitHub workflow schema:

```text
https://json.schemastore.org/github-workflow.json
```

Action metadata files are validated against the SchemaStore GitHub action
metadata schema:

```text
https://json.schemastore.org/github-action.json
```

The schema is vendored into release builds so normal scans do not depend on
network access. Schema diagnostics are reported with file diagnostics by
default. `--strict-schema` turns schema diagnostics into exit code `3`, while
`--no-schema-validation` skips schema validation and only scans supported
`uses:` refs.

## Cache And Metadata

The default cache directory is the platform cache directory with
`gh-actions-updater` appended, for example:

- Linux: `$XDG_CACHE_HOME/gh-actions-updater` or
  `~/.cache/gh-actions-updater`
- macOS: `~/Library/Caches/gh-actions-updater`
- Windows: `%LOCALAPPDATA%\gh-actions-updater`

Cache entries are keyed with `blake3` over the GitHub host, action repository,
lookup mode, and relevant request identity. Cached data includes repository
metadata, tag lists, default branch, selected commit hashes, ETags, and fetch
timestamps.

Cache flags are exact:

- `--cache-ttl` changes freshness for this run.
- `--refresh-cache` skips cache reads and writes fresh entries after successful
  fetches.
- `--no-cache` disables both cache reads and cache writes.

Cache freshness behavior:

| State | Behavior |
| --- | --- |
| Fresh cache hit | Use cached metadata. |
| Stale cache hit | Try conditional metadata refresh. If refresh fails, use stale data with a warning in report-only mode. |
| Stale cache hit with `--check` or `--update` | Try refresh. If refresh fails, exit `4` instead of using stale data. |
| `--refresh-cache` | Skip cache reads. Fetch metadata and write fresh entries. If fetch fails, exit `4`. |
| `--no-cache` | Fetch metadata without reading or writing cache. If fetch fails, exit `4`. |
| `--cache-ttl 0` | Treat all entries as stale. |
| `--cache-ttl never` | Treat existing entries as fresh until `--refresh-cache` is used. |

Corrupt cache entries are ignored and replaced when network access succeeds.

Metadata lookup uses conditional GitHub REST requests with cached ETags when
available. The implementation may use `git ls-remote --tags --refs` for public
or git-accessible repositories when it is cheaper than API pagination. It must
not clone repositories for tag-only checks.

## Output

Human output is written to stdout. Diagnostics, warnings, and network/cache logs
are written to stderr. `--quiet` suppresses human stdout but not errors.

JSON output is written to stdout and is not suppressed by `--quiet`. Logs and
diagnostics that are not part of the JSON payload go to stderr. `--diff` is
only supported with `--format human` in v0.1.0.

The v0.1.0 JSON shape is:

```json
{
  "version": "0.1.0",
  "changed": false,
  "summary": {
    "files_scanned": 0,
    "references_found": 0,
    "updates_available": 0
  },
  "files": [],
  "references": [],
  "updates": [],
  "diagnostics": [],
  "cache": {
    "enabled": true,
    "fresh_hits": 0,
    "stale_hits": 0,
    "misses": 0,
    "refreshes": 0
  }
}
```

## Pre-commit Hook

The project will publish a `pre-commit.com` hook through `.pre-commit-hooks.yaml`.
The hook default is check-only:

```yaml
repos:
  - repo: https://github.com/Goldziher/gh-actions-updater
    rev: v0.1.0
    hooks:
      - id: gh-actions-updater
```

The hook uses `--check` and does not mutate files unless users pass
`--update`. When cache TTL expires, it may refresh cheap tag metadata. It only
performs commit-hash resolution when `--latest-hash` is configured.

The hook contract is:

- `id: gh-actions-updater`
- `name: gh-actions-updater`
- `entry: gh-actions-updater --check`
- `language: rust`
- `pass_filenames: false`
- `stages: [pre-commit]`

## Exit Codes

| Code | Meaning |
| --- | --- |
| `0` | Completed successfully and no blocking updates were found. |
| `1` | `--check` found updates. |
| `2` | Invalid CLI arguments or configuration. |
| `3` | Workflow parsing failed. |
| `4` | Metadata lookup failed and no usable cache entry exists. |
| `5` | File rewrite failed. |

Successful `--update` runs exit `0`, even when files were changed. Use
`--check` for drift-detection behavior.

## Release Roadmap

The Rust crate is the source package. Release automation will use GoReleaser for
GitHub archives and Homebrew, with wrapper packages for npm and PyPI. Planned
distribution targets are:

- crates.io
- GitHub release archives
- Homebrew with bottles, after the archive flow is stable
- npm, after the CLI and JSON output contract are stable
- PyPI, after the CLI and JSON output contract are stable
