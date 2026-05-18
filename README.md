# gh-actions-updater

Version: `0.1.1`

`gh-actions-updater` is the package. `gau` is the CLI command.

`gau` scans GitHub Actions workflow files and action metadata, reports available
updates, and can rewrite supported `uses:` refs to the latest compatible tag or
to the commit SHA behind that tag. It is built for fast local, CI, and
pre-commit use with a global metadata cache.

## Install

```bash
# Homebrew
brew tap goldziher/tap
brew install gh-actions-updater

# Cargo
cargo install gh-actions-updater

# npm
npm install -g gh-actions-updater
npx -y gh-actions-updater@latest --help

# PyPI
pip install gh-actions-updater
uvx --from gh-actions-updater gau --help
```

All package managers install the `gau` command.

## Usage

```bash
gau [OPTIONS] [PATHS]...
```

Common commands:

```bash
gau --init
gau .
gau --check .
gau --update .
gau --update --diff .
gau --latest-hash --update .
gau -r ~/workspace --check
```

When no path is supplied, `gau` scans the current directory. By default it scans
only the current repository's GitHub Actions surface:

- `.github/workflows/**/*.yml`
- `.github/workflows/**/*.yaml`
- `.github/actions/**/action.yml`
- `.github/actions/**/action.yaml`
- `action.yml`
- `action.yaml`

Use `-r, --recursive` to scan nested repositories or workspaces.

## CLI Flags

| Flag | Description |
| --- | --- |
| `--init` | Write an initial `.gh-actions-updater.toml`. |
| `--force` | Allow `--init` to overwrite an existing config. |
| `--output <PATH>` | Write `--init` config to a specific path. |
| `-c, --config <PATH>` | Load a specific TOML config file. |
| `--include <GLOB>` | Add an include glob. Repeatable. |
| `--exclude <GLOB>` | Add an exclude glob. Repeatable. |
| `-r, --recursive` | Scan nested `.github` folders under input directories. |
| `--threads <N>` | Override Rayon thread count. Default uses available CPU cores. |
| `--cache-dir <PATH>` | Override the global cache directory. |
| `--cache-ttl <DURATION>` | Cache TTL such as `15m`, `6h`, `7d`, `0`, or `never`. |
| `--refresh-cache` | Ignore existing cache entries and fetch fresh metadata. |
| `--no-cache` | Disable cache reads and writes. |
| `--update` | Rewrite supported refs. |
| `--latest-hash` | Pin update targets to commit SHAs instead of tags. |
| `--missing-ref <warn\|error\|ignore\|fallback>` | Policy for deleted or missing current refs. |
| `--check` | Exit nonzero when updates are available. Does not rewrite files. |
| `--dry-run` | Preview without writing. Implied unless `--update` is passed. |
| `--diff` | Include unified diffs for proposed updates. |
| `--format <human\|json>` | Select output format. |
| `-q, --quiet` | Suppress non-error human output. |
| `-v, --verbose` | Print cache details to stderr. |
| `--color <auto\|always\|never>` | Control human output color. |
| `--github-token <TOKEN>` | Use a GitHub token for metadata requests. |
| `--github-api-url <URL>` | Use a GitHub-compatible API endpoint. |
| `--strict-schema` | Treat schema diagnostics as failures. |
| `--no-schema-validation` | Skip SchemaStore validation. |
| `-h, --help` | Print help. |
| `-V, --version` | Print version. |

## Configuration

Generate a starter config:

```bash
gau --init
gau --init --recursive --force
gau --init --output config/gau.toml
```

Config discovery looks for `.gh-actions-updater.toml` from the current
directory upward until the repository root. `--config` disables discovery.

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
recursive = false

[cache]
enabled = true
ttl = "6h"

[update]
mode = "latest-tag"
exclude = []
include_prereleases = false
preserve_major = true
missing_ref = "warn"

[output]
format = "human"
color = "auto"

[github]
api_url = "https://api.github.com"

[performance]
# Omit threads to use Rayon available-core default.
# threads = 8
```

Precedence is CLI flags, environment variables, config file, then defaults.

Supported environment variables:

- `GHAU_GITHUB_TOKEN`
- `GITHUB_TOKEN`
- `GH_TOKEN`
- `GHAU_CACHE_DIR`
- `GHAU_CACHE_TTL`

## Update Semantics

Supported `uses:` locations:

- workflow step actions: `jobs.<job_id>.steps[*].uses`
- reusable workflow calls: `jobs.<job_id>.uses`
- composite action step actions: `runs.steps[*].uses` when
  `runs.using = "composite"`

Default mode is `latest-tag`. Semver-like refs such as `@v4` and `@v4.1.0`
update within the current major version by default. Reusable workflows such as
`owner/repo/.github/workflows/reusable.yml@v1` follow the same tag/hash policy.

Branch refs such as `@main`, Docker image refs, local actions, local reusable
workflows, immutable SHAs, and non-semver tag sets are reported but not updated
by default.

Exclude specific remote actions or reusable workflows from updates in config:

```toml
[update]
exclude = [
  "actions/checkout",
  "docker/*",
  "owner/repo/.github/workflows/deploy.yml@v*",
]
```

Inline ignores are also supported on the same line as a `uses:` value:

```yaml
- uses: actions/checkout@v4 # gau: ignore
```

`--latest-hash` first selects the same compatible tag that latest-tag mode would
select, then rewrites to the commit SHA behind that selected tag. It does not
jump to the action repository default branch.

Deleted or missing refs are controlled by `missing_ref`:

| Policy | Behavior |
| --- | --- |
| `warn` | Report the missing ref and continue. |
| `error` | Treat the missing ref as a failure. |
| `ignore` | Suppress missing-ref diagnostics. |
| `fallback` | Allow normal update target selection and rewriting. |

## Cache And Performance

The default cache directory is the platform cache directory with
`gh-actions-updater` appended:

- Linux: `$XDG_CACHE_HOME/gh-actions-updater` or `~/.cache/gh-actions-updater`
- macOS: `~/Library/Caches/gh-actions-updater`
- Windows: `%LOCALAPPDATA%\gh-actions-updater`

Cache keys use `blake3`. Hot lookup sets/maps use `ahash`. YAML scanning uses a
cheap `memchr` precheck before parsing. File scanning is parallelized with Rayon;
by default Rayon uses available CPU cores. Use `--threads 1` for single-threaded
debugging or deterministic profiling.

Cache behavior:

| Flag/state | Behavior |
| --- | --- |
| Fresh hit | Use cached metadata. |
| Stale hit | Refresh metadata; report-only mode may use stale data with a warning if refresh fails. |
| `--refresh-cache` | Skip cache reads and write fresh entries after fetch. |
| `--no-cache` | Fetch without reading or writing cache. |
| `--cache-ttl 0` | Treat entries as stale. |
| `--cache-ttl never` | Treat existing entries as fresh until refresh is requested. |

Normal latest-tag checks fetch tag metadata only. The tool does not clone
repositories for tag checks.

## Schemas

Workflow files are validated against the vendored SchemaStore GitHub workflow
schema. Action metadata files are validated against the vendored SchemaStore
GitHub action schema.

Schema diagnostics are nonfatal by default. `--strict-schema` exits with code
`3` when schema diagnostics are present. `--no-schema-validation` skips schema
validation and only scans supported `uses:` refs.

## Output

Human output goes to stdout. Diagnostics and verbose cache details go to
stderr. JSON output is always written to stdout, even with `--quiet`.

JSON output includes:

```json
{
  "version": "0.1.1",
  "changed": false,
  "would_change": false,
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
  },
  "diffs": []
}
```

`changed` means files were written. `would_change` means updates are available
or a dry-run/diff would modify files.

## Pre-commit

```yaml
repos:
  - repo: https://github.com/Goldziher/gh-actions-updater
    rev: v0.1.1
    hooks:
      - id: gh-actions-updater
```

The hook entry is `gau --check`, uses the global cache, and does not mutate
files. Users can pass args such as `--cache-ttl 24h`, `--no-cache`, or
`--latest-hash`.

## Exit Codes

| Code | Meaning |
| --- | --- |
| `0` | Completed successfully. |
| `1` | `--check` found updates or missing-ref policy requested a check failure. |
| `2` | Invalid CLI arguments, config, init, cache setup, or thread setup. |
| `3` | File discovery, parsing, or strict schema validation failed. |
| `4` | Metadata lookup failed and no usable cache entry exists. |
| `5` | File rewrite failed. |

## Release

Release automation uses GoReleaser for GitHub archives and Homebrew formula
generation, with wrapper packages for npm and PyPI. Distribution targets are
crates.io, GitHub Releases, Homebrew bottles, npm, and PyPI.

Package names stay `gh-actions-updater`. The installed command is `gau`.
