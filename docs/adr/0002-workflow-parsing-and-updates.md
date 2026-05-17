# ADR 0002: Workflow Parsing And Update Semantics

Status: Accepted for v0.1.0 implementation

## Context

GitHub Actions workflow files are YAML, but the hot path is finding `uses:`
references. The tool should scan quickly while still preserving files when
updates are eventually written.

## Decision

The default scan set is `.github/**/*.yml` and `.github/**/*.yaml`. Users may
add paths or glob patterns through CLI flags and config.

The scanner will use a SIMD-friendly byte prefilter for `uses:` lines before
performing structured parsing. Workflow YAML will be validated with a vendored
copy of the SchemaStore GitHub workflow schema:

```text
https://json.schemastore.org/github-workflow.json
```

Schema diagnostics are non-fatal by default, fatal with `--strict-schema`, and
skipped with `--no-schema-validation`.

The SchemaStore `github-action.json` schema applies to action metadata files
such as `action.yml` and `action.yaml`; it is not used as the workflow schema in
v0.1.0.

Remote GitHub action references have this shape:

```text
owner/repo[/path]@ref
```

Supported classifications:

- remote GitHub actions: update candidates
- local actions: reported as non-updatable
- Docker image actions: reported as non-updatable
- malformed refs: reported as parse diagnostics

Update semantics:

- latest tag is the default target
- semver-like tags preserve the current major version by default
- prerelease tags are ignored by default
- branch refs are reported but not updated by default
- SHA refs are reported but not updated by default
- non-semver tag sets are reported but not updated by default
- `--latest-hash` selects the same tag as latest-tag mode, then pins the commit
  SHA behind that tag

When file rewriting is implemented, the updater will use span-based replacement
of only the `@ref` substring in simple scalar `uses:` values. It must not
round-trip or normalize the whole YAML document. Multiline values, templated
values, anchored values, and non-string values are reported as unsupported
instead of being rewritten.

## Consequences

The scanner can be fast without committing to lossy YAML round-tripping. Update
behavior is conservative and avoids surprising branch or SHA rewrites.
