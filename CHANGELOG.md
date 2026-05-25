# Changelog

All notable changes to this project are documented here.

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
