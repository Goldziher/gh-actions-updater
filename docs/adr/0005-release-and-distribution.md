# ADR 0005: Release And Distribution

Status: Accepted for v0.1.0 implementation

## Context

The tool should be available through Rust and common package managers. The
release layout should match the proven `../uncomment` pattern while avoiding
checked-in binaries and keeping package versions synchronized.

## Decision

The Rust crate is the source artifact. Required crate metadata before publishing
includes description, license, repository, readme, categories, keywords, and
included files.

Release automation uses GoReleaser for GitHub archives and Homebrew formula
generation. Homebrew bottles are delegated to a tap-side workflow dispatched
after the formula update.

Distribution targets:

- crates.io
- GitHub release archives
- Homebrew formula plus bottles in `Goldziher/homebrew-tap`
- npm wrapper package `gh-actions-updater`
- PyPI wrapper package `gh-actions-updater`

Release flow requirements:

- publish wrapper packages only after GitHub release assets exist
- do not publish checked-in platform binaries in npm
- verify downloaded GitHub release archive checksums in npm and PyPI wrappers
- sync and verify versions across Cargo, npm, and PyPI metadata
- use clean `cargo publish --locked` without `--allow-dirty`
- support prerelease tag mapping for npm and PyPI
- publish crates.io with OIDC trusted publishing
- publish npm and PyPI with provenance/trusted publishing
- pin release workflow actions to commit SHAs

## Consequences

Release design follows proven patterns from `../uncomment` while avoiding known
gaps: stale checked-in binaries, wrapper publication before assets, dirty crate
publishing, and missing Homebrew bottle generation.
