# Changelog

All notable changes to this project are documented here.

## [0.2.1] - 2026-08-23

### Fixed

- `preserve_major = false` now allows tag and hash update modes to select newer
  major versions, including floating refs such as `actions/checkout@v3`.

## [0.2.0] - 2026-08-22

### Added

- Added `--latest`, `--latest-tag`, and `--latest-hash` selection modes; `--latest`
  preserves whether each reference uses a tag or commit SHA.
- Added a checksum-verifying composite GitHub Action with explicit `check` and
  `update` operations.
- Added separate check/update hooks for pre-commit and Poly. The legacy
  `gh-actions-updater` pre-commit hook remains an alias for the update behavior.

### Changed

- Metadata failures and intentional skips are collected and reported instead of
  suppressing unrelated results. JSON output includes structured diagnostic and
  skip codes plus skipped/failure summary counts.
- Semver-looking branches, local references, SHA pins, and renamed repositories
  are resolved without false missing-reference or unsupported-update skips.
- Stable releases move the `v0` Action tag only after all archives and checksums
  are present and the GitHub release is published.

## [0.1.7] - 2026-06-26

### Fixed

- Rewrites no longer break when a multibyte character (for example an em-dash or
  box-drawing character in a step name or workflow `on` description) appears
  before a `uses:` reference. The parser reports char-based offsets, which were
  treated as byte offsets when slicing the source; references after multibyte
  content were misreported as "uses value is not a simple single-line rewrite
  target" and skipped. Offsets are now converted to bytes before slicing.

## [0.1.6] - 2026-06-04

### Added

- `--validate` verifies that every reference exists in its upstream repository
  (tags, branches, and commit SHAs) or on disk for local refs. Missing
  references are surfaced through the existing `missing_ref` policy.
- `--pin-floating-to-sha` rewrites references pinned to a branch or a
  non-semver tag (for example `main`, `release/v1`, `stable`) to the commit
  SHA they currently point at — a security-hardening fix that converts
  mutable refs into immutable ones.
- New `branch` and `non_semver_tag` `ref_kind` values, populated when
  `--validate` or `--pin-floating-to-sha` classifies an otherwise opaque ref
  against the upstream repository.
- SHA-pinned references now emit an advisory diagnostic in the default scan
  when a newer same-major SHA exists, so users see stale pins without having
  to opt into `--latest-hash`.

### Fixed

- `--pin-style preserve` no longer demotes full semver pins to equivalent
  lower-precision tags. For repositories such as `erlef/setup-beam` that
  publish both `v1.24` and `v1.24.0`, a pin at `v1.24.0` is now treated as a
  no-op instead of being "updated" to `v1.24`.

## [0.1.5] - 2026-05-25

### Fixed

- Recursive discovery now respects `.gitignore` while still allowing explicitly
  invoked ignored files or directories to be scanned.

## [0.1.4] - 2026-05-25

### Added

- Added `--pin-style <preserve|major|minor|full>` and `[update].pin_style`
  for explicit semver pin formatting.
- Added grouped, color-aware human output and fuller CLI help text.

### Fixed

- Floating semver pins such as `v1` and `v1.24` are now preserved by default,
  avoiding false updates such as `v1.24 -> v1.24.0`.
- Pin-style conversions now validate generated floating targets before
  rewriting, preventing broken refs.
- Recursive workspace scans now find nested repository Actions files even when
  the workspace root ignores those nested repos.

## [0.1.3] - 2026-05-20

### Changed

- Pre-commit hook now runs `gau --update` so supported GitHub Actions refs are
  autofixed during pre-commit.
- Major-only tags such as `v6` are accepted as current when the exact remote tag
  exists, avoiding rewrites to full patch tags such as `v6.0.2`.

## [0.1.2] - 2026-05-18

### Fixed

- Scanner now robustly handles cases where YAML parser spans don't match source
  exactly. A fallback mechanism searches for the `@` symbol directly within the
  parsed span when source matching fails, enabling updates for bare `uses:` lines,
  sub-path actions (e.g. `gradle/actions/setup-gradle@v6`), and other edge cases
  where the parser and source normalization diverge.

## [0.1.1] - 2026-05-18

### Added

- Installed CLI command is now `gau` while package names remain
  `gh-actions-updater`.
- Added `gau --init` for `.gh-actions-updater.toml` scaffolding.
- Added opt-in recursive discovery with `-r, --recursive` and
  `[scan] recursive = true`.
- Added Rayon-backed parallel discovery/scanning with `--threads` and
  `[performance] threads`.
- Added `[update].exclude` and inline `# gau: ignore` support for update
  suppression.
- Added `would_change` to JSON output.
- Added Taskfile release helpers, including `task set-version -- <version>`.
- Added `.ai-rulez` source rules for generated agent tooling.

### Changed

- Non-recursive directory scans now stay scoped to the requested root's own
  GitHub Actions files instead of traversing vendored nested `.github` folders.
- CLI excludes now add to configured excludes instead of replacing them.
- Diff output now renders focused unified hunks instead of full-file rewrites.
- npm, PyPI, and Homebrew packaging now install/test `gau`.

### Fixed

- Branch-backed semver refs such as `ruby/setup-ruby@v1` no longer report false
  missing-ref diagnostics.
- Repeated cache runs now reuse fresh metadata across sibling-repo scans.

## [0.1.0] - 2026-05-18

### Added

- Initial crates.io package for `gh-actions-updater`.
- GitHub Actions workflow and action metadata scanning.
- Latest-tag and latest-hash update resolution.
- Global cache with configurable TTL, refresh, and no-cache modes.
- SchemaStore validation for GitHub workflows and action metadata.
- Safe dry-run, check, update, diff, and JSON output modes.
- Missing-ref policy support for deleted or unavailable tags/SHAs.
- Initial GoReleaser, npm, PyPI, Homebrew, and pre-commit scaffolding.
