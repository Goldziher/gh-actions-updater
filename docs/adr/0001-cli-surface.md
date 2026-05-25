# ADR 0001: CLI Surface And Modes

Status: Accepted for v0.1.1 implementation

## Context

The tool must work as an interactive CLI, an automation-friendly checker, and a
pre-commit hook. The default command needs to be fast and safe: users should be
able to run it without risking workflow rewrites.

## Decision

The v0.1.1 package name is `gh-actions-updater`; the installed binary is `gau`
at version `0.1.1`.

The default invocation scans workflow files and reports action refs. It does not
write files unless `--update` is provided. `--dry-run` is therefore the default
effective mode when `--update` is absent.

Supported flags:

- `--config <PATH>`
- `--init`, `--force`, and `--output <PATH>`
- `--include <GLOB>` and `--exclude <GLOB>`
- `--recursive`
- `--threads <N>`
- `--cache-dir <PATH>`
- `--cache-ttl <DURATION>`
- `--refresh-cache`
- `--no-cache`
- `--update`
- `--latest-hash`
- `--pin-style <preserve|major|minor|full>`
- `--missing-ref <warn|error|ignore|fallback>`
- `--check`
- `--dry-run`
- `--diff`
- `--format <human|json>`
- `--quiet`
- `--verbose`
- `--color <auto|always|never>`
- `--github-token <TOKEN>`
- `--github-api-url <URL>`
- `--strict-schema`
- `--no-schema-validation`
- `--help`
- `--version`

Exit codes are stable:

- `0`: success
- `1`: updates found in `--check`
- `2`: invalid arguments or config
- `3`: workflow parsing or strict schema validation failed
- `4`: metadata lookup failed without usable cache
- `5`: file rewrite failed

Successful `--update` exits `0`, including when files were changed. Users who
need drift detection must use `--check`.

Machine-readable JSON is written to stdout. Logs, warnings, and diagnostics not
included in the JSON payload are written to stderr. `--diff` prints diff text in
human output and includes a `diffs` array in JSON output. JSON `changed` means
files were written; `would_change` covers dry-run/check/diff automation.

## Consequences

Automation can rely on a non-mutating default. The CLI has enough surface for
pre-commit and CI without requiring separate subcommands in v0.1.1.
