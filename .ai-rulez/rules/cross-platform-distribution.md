---
priority: high
---

# Cross-Platform Distribution

Package names stay `gh-actions-updater`; the installed command is `gau`.

## Channels

- crates.io package: `gh-actions-updater`
- npm package: `gh-actions-updater`
- PyPI package: `gh-actions-updater`
- Homebrew formula: `gh-actions-updater`
- GitHub release archives built by GoReleaser

Do not add a compatibility `gh-actions-updater` binary unless explicitly
requested. Wrapper packages should expose only `gau`.

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

## Wrapper Rules

- npm `bin` exposes `gau`.
- PyPI console scripts expose `gau`.
- Wrappers download from GitHub Releases and verify checksums.
- `GH_ACTIONS_UPDATER_BINARY` may override the Python wrapper binary path.
- Test wrapper syntax and packaging whenever wrapper code changes.

## Trusted Publishing

crates.io, npm, and PyPI are intended to use trusted publishing/provenance in
CI. Manual bootstrap publishes may be needed to configure registry-side trusted
publisher settings.
