---
priority: high
---

# Cross-Platform Distribution

Package names stay `gh-actions-updater`; the installed command is `gau`, with a
`ghau` alias for shells where `gau` is already taken (for example the oh-my-zsh
`gau` alias for `git add --update`).

## Channels

- crates.io package: `gh-actions-updater`
- npm package: `gh-actions-updater`
- PyPI package: `gh-actions-updater`
- Homebrew formula: `gh-actions-updater`
- GitHub release archives built by GoReleaser

Do not add a compatibility `gh-actions-updater` binary unless explicitly
requested. Wrapper packages expose `gau` and the `ghau` alias, nothing else.

## Release Assets

GoReleaser builds the `gau` binary. Archive names may still use the project
name `gh-actions-updater` for discoverability and wrapper download URLs.

Expected targets:

- `x86_64-unknown-linux-gnu`
- `aarch64-unknown-linux-gnu`
- `x86_64-apple-darwin`
- `aarch64-apple-darwin`
- `x86_64-pc-windows-gnu`

Homebrew bottles are produced by the `Goldziher/homebrew-tap` bottle workflow.

## Integrations

- The composite GitHub Action exposes bounded `check` and `update` operations.
- The Action installer verifies release archives against `checksums.txt`.
- pre-commit and Poly publish `gh-actions-updater-check` and
  `gh-actions-updater-update`; the legacy pre-commit id aliases update.
- Stable releases move `v0` only after every required archive and checksum is
  present and the release is published.

## Wrapper Rules

- npm `bin` exposes `gau` and `ghau` (both point at the same wrapper).
- PyPI console scripts expose `gau` and `ghau`.
- Cargo installs `gau` and `ghau` from the same sources.
- Release archives contain `gau` and a `ghau` alias (symlink on Unix, copy on
  Windows); the Action installer materializes `ghau` next to `gau`.
- The npm wrapper is built and published with pnpm 12; `npm-package/package.json`
  pins `packageManager` and CI/publish use `pnpm install --frozen-lockfile` and
  `pnpm publish`.
- Wrappers download from GitHub Releases and verify checksums.
- `GH_ACTIONS_UPDATER_BINARY` may override the Python wrapper binary path.
- Test wrapper syntax and packaging whenever wrapper code changes.

## Trusted Publishing

crates.io, npm, and PyPI are intended to use trusted publishing/provenance in
CI. Manual bootstrap publishes may be needed to configure registry-side trusted
publisher settings.
