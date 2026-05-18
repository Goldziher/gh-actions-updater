---
priority: high
---

# Project Architecture

`gh-actions-updater` is a Rust 2024 CLI package. The installed user command is
`gau`.

## Core Modules

- `src/main.rs` — process entry point.
- `src/lib.rs` — top-level run orchestration and exit-code handling.
- `src/cli.rs` — clap flag surface for scan/update/init modes.
- `src/config.rs` — config discovery, env/CLI precedence, TTL parsing.
- `src/init.rs` — `.gh-actions-updater.toml` scaffolding for `gau --init`.
- `src/discover.rs` — GitHub Actions file discovery, including opt-in recursive scanning.
- `src/scanner.rs` — YAML parsing, SchemaStore validation, `uses:` extraction, inline ignore detection.
- `src/action_ref.rs` — parsing/classification of remote actions, reusable workflows, local refs, Docker refs, branches, tags, and SHAs.
- `src/metadata.rs` — GitHub tag/commit/branch metadata resolution, missing-ref policy, cache use, update selection.
- `src/cache.rs` — global cache preparation, BLAKE3 cache keys, cache load/save.
- `src/rewrite.rs` — safe byte-span rewrites and unified diff rendering.
- `src/report.rs` — human/JSON report model and output.

## Distribution

- Cargo package name: `gh-actions-updater`.
- Installed binary: `gau`.
- `npm-package/` — npm wrapper package named `gh-actions-updater`; exposes `gau`.
- `pip-package/` — PyPI wrapper package named `gh-actions-updater`; exposes `gau`.
- `.goreleaser.yaml` — release archives and Homebrew formula generation.
- `.github/workflows/ci.yaml` — quality gates.
- `.github/workflows/publish.yaml` — crates.io, GitHub release assets, Homebrew bottles, npm, and PyPI.

## Runtime Model

The default command is non-mutating. It discovers GitHub Actions YAML files,
extracts supported `uses:` references, validates schemas unless disabled,
resolves metadata for update candidates, then reports updates. Files are written
only when `--update` is passed and the reference has a safe byte span.

## Generated Agent Tooling

`.ai-rulez/` is the source for generated agent instructions. Update these files
first when project conventions change, then regenerate the downstream agent
artifacts with the ai-rulez tooling.
