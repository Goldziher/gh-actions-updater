---
priority: high
aliases: [f]
usage: "/fix"
description: "Apply safe formatting and mechanical fixes"
---

# Fix

Apply safe mechanical fixes only.

## Steps

1. Run `cargo fmt --all`.
2. Run `cargo clippy --fix --allow-dirty --allow-staged --all-targets --all-features` only when the requested work allows mechanical lint fixes.
3. Do not run commands that mutate sibling repositories.
4. Re-run:
   - `cargo fmt --all --check`
   - `cargo clippy --locked --all-targets --all-features -- -D warnings`
5. Report changed files and any remaining manual fixes.
