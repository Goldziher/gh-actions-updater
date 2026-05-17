# ADR 0005: Release And Distribution

Status: Accepted for roadmap

## Context

The tool should be available through Rust and common package managers. The
near-term implementation should not overfit release wrappers before the CLI is
stable, but the repository layout should leave room for them.

## Decision

The Rust crate is the source artifact. Required crate metadata before publishing
includes description, license, repository, readme, categories, keywords, and
included files.

Release automation will use GoReleaser for GitHub archives and Homebrew formula
generation. Unlike the reference project, Homebrew bottles are an explicit
requirement and need a dedicated workflow or tap-side bottle process.

Distribution targets:

- v0.1.0 gate: crates.io and GitHub release archives
- later: Homebrew with bottles
- later: npm wrapper package
- later: PyPI wrapper package

Release flow requirements:

- publish wrapper packages only after GitHub release assets exist
- do not publish checked-in platform binaries in npm
- sync and verify versions across Cargo, npm, and PyPI metadata
- prefer clean `cargo publish` without `--allow-dirty`
- support prerelease tag mapping for npm and PyPI

## Consequences

Release design follows proven patterns from `../uncomment` while avoiding known
gaps: stale checked-in binaries, wrapper publication before assets, and missing
Homebrew bottle generation.
