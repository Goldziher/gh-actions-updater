---
priority: high
aliases: [l]
usage: "/lint"
description: "Run formatting, clippy, wrapper syntax, and release config checks"
---

# Lint

Run static quality checks.

## Steps

1. Run `cargo fmt --all --check`.
2. Run `cargo clippy --locked --all-targets --all-features -- -D warnings`.
3. Run wrapper syntax checks:
   - `node --check npm-package/index.js`
   - `node --check npm-package/install.js`
   - `node --check npm-package/bin/gau`
   - `python3 -m compileall pip-package/gh_actions_updater`
4. Run `goreleaser check` if `.goreleaser.yaml` changed.
5. Run `poly lint .` if GitHub workflows changed.
6. Report failures by severity and include exact commands to rerun.
