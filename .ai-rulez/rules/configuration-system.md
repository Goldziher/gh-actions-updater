---
priority: high
---

# Configuration System

Configuration must support zero-config local use, pre-commit hooks, and
workspace-wide recursive audits.

## Files And Precedence

Project config file: `.gh-actions-updater.toml`.

Discovery starts at the current directory and walks upward until the repository
root. `--config` disables discovery and loads only the specified file.

Precedence:

1. CLI flags
2. environment variables
3. config file
4. defaults

CLI `--include` and `--exclude` are additive with config lists. Do not replace
configured excludes when CLI excludes are present.

## Config Sections

- `[scan]`: `include`, `exclude`, `recursive`
- `[cache]`: `enabled`, `ttl`, `dir`
- `[update]`: `mode`, `exclude`, `include_prereleases`, `preserve_major`, `missing_ref`
- `[output]`: `format`, `color`
- `[github]`: `api_url`
- `[performance]`: `threads`

Environment variables:

- `GHAU_GITHUB_TOKEN`
- `GITHUB_TOKEN`
- `GH_TOKEN`
- `GHAU_CACHE_DIR`
- `GHAU_CACHE_TTL`

## Init

`gau --init` writes `.gh-actions-updater.toml`.

- Refuse overwrite unless `--force`.
- `--output <PATH>` selects a custom destination.
- `--init --recursive` writes `recursive = true`.
- Include all supported config sections in the generated file.

## Compatibility

The package name is `gh-actions-updater`, but examples and generated docs should
use the `gau` command.
