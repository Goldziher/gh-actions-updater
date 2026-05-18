---
priority: high
---

# Development Setup

## Prerequisites

- Rust 1.85+ with edition 2024 support.
- Cargo and rustfmt/clippy.
- Node.js 20+ for the npm wrapper.
- Python 3.11+ for PyPI wrapper validation.
- GoReleaser for release-config checks.
- `jq`, `gh`, and npm auth for release/publish smoke work.

## Core Commands

```bash
cargo fmt --all --check
cargo check --locked --all-targets --all-features
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets --all-features
cargo package --locked
```

Install and exercise the local binary:

```bash
cargo install --path . --force
gau --version
gau --help
gau --no-cache --no-schema-validation --format json .github
```

The package installs `gau`; do not assume a `gh-actions-updater` binary exists.

## Wrapper Checks

```bash
node --check npm-package/index.js
node --check npm-package/install.js
node --check npm-package/bin/gau
npm pack --dry-run --cache /tmp/ghau-npm-cache

python3 -m compileall pip-package/gh_actions_updater
python3 -m build pip-package --outdir /tmp/ghau-pip-dist
```

## Release Checks

```bash
goreleaser check
prek run actionlint --all-files
```

When adding dependencies, run:

```bash
cargo upgrade --incompatible
```

## Corpus Testing

Use dry-run only against sibling repositories:

```bash
gau ~/workspace/some-repo --format json --cache-ttl 24h --no-schema-validation
gau -r ~/workspace --format json --cache-ttl 24h --no-schema-validation
```

Run repeated passes with the same cache directory when validating idempotency and
cache reuse. Do not run `--update` against sibling repositories directly; use
temporary copies for mutation tests.
