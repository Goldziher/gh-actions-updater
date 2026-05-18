---
priority: high
---

# Version Synchronization

Keep versions synchronized before release.

## Version Locations

1. `Cargo.toml` — `[package].version`
2. `npm-package/package.json` — `version`
3. `pip-package/pyproject.toml` — `[project].version`
4. `pip-package/gh_actions_updater/__init__.py` — `__version__`
5. README examples that pin release tags or show JSON version values

## Rules

- Use SemVer.
- Update all version files in one commit.
- Git tags use `vX.Y.Z`.
- Cargo/npm prereleases use `X.Y.Z-rc.N`.
- PyPI prereleases use `X.Y.ZrcN`.
- Do not create a release tag until `cargo package --locked`, npm pack dry-run,
  and Python build validation pass.
- `0.1.0` was used for registry bootstrap; subsequent proper release work should
  use a new version such as `0.1.1`.

## Validation

The publish workflow checks Cargo, npm, PyPI, and Python runtime versions. Keep
that workflow updated when adding new version-bearing files.
