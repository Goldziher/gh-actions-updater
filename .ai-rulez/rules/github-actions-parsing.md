---
priority: critical
---

# GitHub Actions Parsing

The scanner targets GitHub Actions workflow files and action metadata files. It
must not become a generic YAML updater.

## Supported Files

Default non-recursive discovery covers only the requested root's own Actions
surface:

- `.github/workflows/**/*.yml`
- `.github/workflows/**/*.yaml`
- `.github/actions/**/action.yml`
- `.github/actions/**/action.yaml`
- `action.yml`
- `action.yaml`

`-r, --recursive` and `[scan] recursive = true` opt into nested `.github`
folders for workspace-wide runs.

## Supported `uses:` Locations

- Workflow step actions: `jobs.<job_id>.steps[*].uses`.
- Reusable workflow calls: `jobs.<job_id>.uses`.
- Composite action step actions: `runs.steps[*].uses` when `runs.using = "composite"`.

Ignore top-level `uses` and arbitrary nested keys that are not part of the
GitHub Actions schema.

## Reference Classification

Classify references as:

- remote action: `owner/repo@ref` and `owner/repo/path@ref`
- reusable workflow: `owner/repo/.github/workflows/file.yml@ref`
- local action: `./...` or `../...`
- local reusable workflow
- Docker image: `docker://...`
- malformed or unsupported expression

Only semver-like tag refs are update candidates by default. Branch refs, Docker
refs, local refs, non-semver refs, and immutable SHAs are reportable but not
default update targets.

## Source Preservation

Use parser-derived byte spans for rewrites. Only replace the ref segment after
the final `@`. Preserve comments, quoting, whitespace, file ordering, and line
endings outside the ref span.

Schema validation uses vendored SchemaStore GitHub workflow and GitHub action
schemas. Keep schema validation optional and nonfatal unless `--strict-schema`
is set.
