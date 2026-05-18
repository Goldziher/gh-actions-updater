---
priority: high
aliases: [t]
usage: "/test"
description: "Run Rust tests and installed-binary smoke checks"
---

# Test

Run the test suite and focused CLI smoke tests.

## Steps

1. Run `cargo test --locked --all-targets --all-features`.
2. Install locally when CLI behavior changed:
   - `cargo install --path . --force`
   - `gau --version`
   - `gau --help`
3. Test `gau --init` in a temporary directory, including overwrite refusal and `--force`.
4. Test dry-run/update/idempotency on temporary fixture copies only.
5. Test cache behavior with a temp cache directory:
   - first run
   - second run
   - `--cache-ttl 0`
   - `--cache-ttl never`
   - `--refresh-cache`
   - `--no-cache`
6. For corpus checks, use dry-run only against sibling repositories under `~/workspace`.
7. Summarize pass/fail counts and any residual risk.
