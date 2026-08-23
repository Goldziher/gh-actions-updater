# gh-actions-updater

This tool comes out of necessity - its a pain to maintain Github action versions, and depending on "dependabot" for this creates a lot of unnecessary noise. I tried a similar tool written in typescript and was underwhelmed by the results - it was slow, and did not implement some of the behaviors I needed. Thus `gh-actions-updater` or as the CLI command goes `gau`.

`gau` scans GitHub Actions workflow files and action metadata, reports available updates, and can rewrite supported `uses:` refs to the latest compatible tag or
to the commit SHA behind that tag. It is built for fast local, CI, and pre-commit use with a global metadata cache. Its extremely fast, idempotent and published across multiple packaging ecosystems (using go-releaser) which makes it easy to use regardless of your stack.

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
gau --latest --check .
gau --latest-tag --update .
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

Use `-r, --recursive` to scan nested repositories or workspaces. Recursive
discovery respects `.gitignore`; pass an ignored file or directory explicitly
when you want to scan it anyway.

## CLI Flags

| Flag | Description |
| --- | --- |
| `--init` | Write an initial `.gh-actions-updater.toml`. |
| `--force` | Allow `--init` to overwrite an existing config. |
| `--output <PATH>` | Write `--init` config to a specific path. |
| `-c, --config <PATH>` | Load a specific TOML config file. |
| `--include <GLOB>` | Add an include glob. Repeatable. |
| `--exclude <GLOB>` | Add an exclude glob. Repeatable. |
| `-r, --recursive` | Scan nested `.github` folders under input directories, respecting `.gitignore`. |
| `--threads <N>` | Override Rayon thread count. Default uses available CPU cores. |
| `--cache-dir <PATH>` | Override the global cache directory. |
| `--cache-ttl <DURATION>` | Cache TTL such as `15m`, `6h`, `7d`, `0`, or `never`. |
| `--refresh-cache` | Ignore existing cache entries and fetch fresh metadata. |
| `--no-cache` | Disable cache reads and writes. |
| `--update` | Rewrite supported refs. |
| `--latest` | Update tags as tags and SHA pins as SHAs. |
| `--latest-tag` | Update eligible references to compatible tags; this is the default. |
| `--latest-hash` | Pin update targets to commit SHAs instead of tags. |
| `--pin-style <preserve\|major\|minor\|full>` | Control semver tag pin formatting. |
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
pin_style = "preserve"
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

For GitHub metadata authentication, `--github-token` takes precedence over
`GHAU_GITHUB_TOKEN`, `GITHUB_TOKEN`, and `GH_TOKEN`, in that order. A token is
recommended in CI and for workspace-wide scans to avoid anonymous API rate
limits.

## Update Semantics

Supported `uses:` locations:

- workflow step actions: `jobs.<job_id>.steps[*].uses`
- reusable workflow calls: `jobs.<job_id>.uses`
- composite action step actions: `runs.steps[*].uses` when
  `runs.using = "composite"`

Default mode is `latest-tag`. `--latest` preserves the current representation:
tag pins update to compatible tags and SHA pins update to SHAs. `--latest-hash`
selects the compatible release and pins its commit SHA. Semver-like refs update within the current major
version by default. `pin_style = "preserve"` keeps the current precision:
`@v4` stays a major floating pin, `@v4.1` stays a minor floating pin, and
`@v4.1.0` updates to full compatible tags. Use `--pin-style major`, `minor`, or
`full` to intentionally convert between styles. Reusable workflows such as
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
jump to the action repository default branch. `--latest-hash` cannot be combined
with non-preserve pin styles.

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
  "version": "0.2.1",
  "changed": false,
  "would_change": false,
  "summary": {
    "files_scanned": 0,
    "references_found": 0,
    "updates_available": 0,
    "skipped": 0,
    "failures": 0
  },
  "files": [],
  "references": [],
  "updates": [],
  "diagnostics": [],
  "skips": [],
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
or a dry-run/diff would modify files. Each diagnostic includes a stable `code`
and `category`. Each intentional skip includes its file, line, and reason code.

## GitHub Action

Use the stable `v0` tag as a validation gate:

```yaml
- name: Check GitHub Actions references
  uses: Goldziher/gh-actions-updater@v0
  with:
    operation: check
    mode: latest
```

Set `operation: update` to rewrite the checkout. The Action never commits or
pushes changes.

| Input | Default | Values or purpose |
| --- | --- | --- |
| `operation` | `check` | `check` validates without writing; `update` rewrites the checkout. |
| `mode` | `latest` | `latest`, `latest-tag`, or `latest-hash`. |
| `path` | `.` | Repository or workspace path to scan. |
| `recursive` | `false` | Scan nested repositories and `.github` directories. |
| `validate` | `true` | Verify remote and local references exist. |
| `missing-ref` | `error` | `warn`, `error`, `ignore`, or `fallback`. |
| `version` | `latest` | `gau` release version, with or without a `v` prefix. |
| `cache` | `true` | Cache the installed binary on the runner. |
| `github-token` | `${{ github.token }}` | Token used for release downloads and metadata requests. |

The Action exposes `version` and `install-dir` outputs. Pin the Action to an
exact release instead of `v0` when immutable workflow dependencies are
required.

## Pre-commit

```yaml
repos:
  - repo: https://github.com/Goldziher/gh-actions-updater
    rev: v0.2.1
    hooks:
      - id: gh-actions-updater-check
```

The check hook runs `gau --latest --check --validate --missing-ref error`, uses
the global cache, and never rewrites files. Use `gh-actions-updater-update` for
the equivalent autofixing hook. The legacy `gh-actions-updater` hook id remains
an alias for update. Users can override or extend hook arguments for settings
such as `--cache-ttl 24h`, `--no-cache`, or `--latest-hash`.

## Poly hooks

This repository publishes `gh-actions-updater-check` and
`gh-actions-updater-update` through `poly-hooks.toml`. A consumer selects them:

```toml
[[hooks.sources]]
id = "gh-actions-updater"
git = "https://github.com/Goldziher/gh-actions-updater.git"
revision = "v0.2.1"
hooks = ["gh-actions-updater-check"]
```

Choose an execution channel in the uncommitted `poly.local.toml`:

```toml
[hook_preferences]
channels = ["system", "cargo"]
```

The `system` channel uses an installed `gau`; `cargo` installs the catalog's
pinned package version. Run `poly hooks update` when changing the source revision
and commit the resulting `poly-hooks.lock`.

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
