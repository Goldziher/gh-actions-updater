# Changelog

All notable changes to this project are documented here.

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
