---
priority: high
aliases: [b]
usage: "/build"
description: "Build the gau CLI and package wrappers"
---

# Build

Build and package-check the project.

## Steps

1. Run `cargo build --locked --release`.
2. Confirm the release binary is `target/release/gau`.
3. Run `cargo package --locked` when the worktree is clean enough for package validation.
4. Run wrapper syntax checks:
   - `node --check npm-package/index.js`
   - `node --check npm-package/install.js`
   - `node --check npm-package/bin/gau`
   - `python3 -m compileall pip-package/gh_actions_updater`
5. Report any build/package failures with the exact failing command and the first actionable error.
