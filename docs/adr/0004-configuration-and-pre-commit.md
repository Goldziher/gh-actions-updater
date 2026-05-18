# ADR 0004: Configuration And Pre-commit Hook

Status: Accepted for v0.1.1 implementation

## Context

Users need repo-local defaults, while automation needs stable override behavior.
The project also publishes a user-facing pre-commit hook, which is separate
from this repository's own development pre-commit configuration.

## Decision

The project config file is `.gh-actions-updater.toml`.

Discovery starts at the current directory, walks upward to the repository root,
and uses the first config found. `--config` disables discovery and loads only
the provided file.

Precedence order:

1. CLI flags
2. environment variables
3. config file
4. defaults

Initial config sections:

- `[scan]`: `include`, `exclude`, `recursive`
- `[cache]`: `enabled`, `ttl`, `dir`
- `[update]`: `mode`, `exclude`, `include_prereleases`, `preserve_major`, `missing_ref`
- `[github]`: `api_url`
- `[output]`: `format`, `color`
- `[performance]`: `threads`

Presentation flags `quiet`, `verbose`, and `diff` are CLI-only for v0.1.1.

The user-facing pre-commit hook will be published in `.pre-commit-hooks.yaml`
with hook id `gh-actions-updater`. The default hook args are check-only and do
not rewrite files. Users may opt into mutation by passing `--update`.

The hook contract is:

- `id: gh-actions-updater`
- `name: gh-actions-updater`
- `entry: gau --check`
- `language: rust`
- `pass_filenames: false`
- `stages: [pre-commit]`

When cache TTL expires, the hook may refresh cheap tag metadata. Commit-hash
lookups are enabled only when users opt into `--latest-hash`.

## Consequences

Config behavior is deterministic, and pre-commit users get a safe default that
does not rewrite workflow files during commit unless they ask for it.
