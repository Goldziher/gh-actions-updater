# ADR 0002: GitHub Actions File Parsing And Update Semantics

Status: Accepted for v0.1.1 implementation

## Context

GitHub Actions workflow and action metadata files are YAML, but the hot path is
finding `uses:` references. The tool should scan quickly while still preserving
files when updates are eventually written.

## Decision

The default scan set covers workflow files and action metadata files:

- `.github/workflows/**/*.yml`
- `.github/workflows/**/*.yaml`
- `.github/actions/**/action.yml`
- `.github/actions/**/action.yaml`
- `action.yml`
- `action.yaml`

Users may add paths or glob patterns through CLI flags and config.

The scanner will use a SIMD-friendly byte prefilter for `uses:` lines before
performing structured parsing.

Workflow YAML will be validated with a vendored copy of the SchemaStore GitHub
workflow schema:

```text
https://json.schemastore.org/github-workflow.json
```

Action metadata YAML will be validated with a vendored copy of the SchemaStore
GitHub action metadata schema:

```text
https://json.schemastore.org/github-action.json
```

Schema diagnostics are non-fatal by default, fatal with `--strict-schema`, and
skipped with `--no-schema-validation`.

Supported `uses:` locations:

- workflow step actions: `jobs.<job_id>.steps[*].uses`
- reusable workflow calls: `jobs.<job_id>.uses`
- composite action step actions: `runs.steps[*].uses` when
  `runs.using = "composite"`

Remote GitHub action references have this shape:

```text
owner/repo[/path]@ref
```

Reusable workflow references use the same remote reference grammar, usually with
`.github/workflows/<file>.yml` or `.yaml` in the path.

Supported classifications:

- remote GitHub actions and reusable workflows: update candidates
- local actions: reported as non-updatable
- local reusable workflows: reported as non-updatable
- Docker image actions: reported as non-updatable
- malformed refs: reported as parse diagnostics

Update semantics:

- latest tag is the default target
- semver-like tags select the latest version across majors by default
- `preserve_major = true` restricts selection to the current major
- semver-like tag formatting is preserved by default: major floats stay major
  floats, minor floats stay minor floats, and full tags stay full tags
- `--pin-style major`, `minor`, or `full` explicitly converts semver-like refs
  to that formatting style in latest-tag mode
- prerelease tags are ignored by default
- branch refs are reported but not updated by default
- SHA refs are reported but not updated by default
- non-semver tag sets are reported but not updated by default
- deleted or missing remote tag refs are controlled by the missing-ref policy
- `--latest-hash` selects the same tag as latest-tag mode, then pins the commit
  SHA behind that tag
- `--latest-hash` is incompatible with non-preserve pin styles

Missing-ref policy values:

- `warn`: report the missing tag and continue
- `error`: fail check/update runs when the current ref no longer exists
- `ignore`: suppress missing-ref diagnostics
- `fallback`: select the normal update target and allow `--update` to rewrite
  to it

The default is `warn`. The latest-tag resolver enforces this policy for tag refs
only. The latest-hash resolver checks unmatched SHA refs through commit
metadata. SHA refs select the latest semver tag across majors unless
`preserve_major = true` restricts selection to their mapped major.
The updater must not silently rewrite a deleted tag unless the effective policy
is `fallback`.

File rewriting uses span-based replacement of only the `@ref` substring in
simple scalar `uses:` values. It must not round-trip or normalize the whole YAML
document. Multiline values, templated values, anchored values, and non-string
values are reported as unsupported instead of being rewritten.

## Consequences

The scanner can be fast without committing to lossy YAML round-tripping. Update
behavior is conservative and avoids surprising branch or SHA rewrites.
