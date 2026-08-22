---
priority: high
---

# Update Policy And Ignores

Updates must be predictable and conservative. Reporting a reference is not the
same as rewriting it.

## Default Update Policy

- Latest-tag mode is the default.
- Preserve the current major version for semver-like refs.
- Ignore prereleases unless configured.
- Reusable workflow refs follow the same tag/hash policy as action refs.
- `--latest-tag` is the default and selects compatible semantic-version tags.
- `--latest` preserves each reference's current tag or SHA format.
- `--latest-hash` first selects the same compatible tag as latest-tag mode,
  then pins to that tag's commit SHA.
- Never jump to the remote repository default branch as an update target.

## Missing Or Deleted Refs

Missing refs are controlled by `missing_ref`:

- `warn`: report and continue.
- `error`: fail.
- `ignore`: suppress missing-ref diagnostics.
- `fallback`: allow rewriting to the normal update target.

Do not silently rewrite a deleted tag unless the effective policy is
`fallback`.

## Excluding Updates

Config can exclude refs from update selection:

```toml
[update]
exclude = ["actions/checkout", "owner/repo/.github/workflows/deploy.yml@v*"]
```

Inline ignore comments suppress updates for that single `uses:` line:

```yaml
- uses: actions/checkout@v4 # gau: ignore
```

Ignore/exclude behavior should prevent metadata fetches when possible, but it
should not hide the reference from scan output.

## Rewrites And Diffs

- Rewrite only safe byte spans.
- `--dry-run` must never mutate files.
- JSON `changed` means files were written.
- JSON `would_change` means updates are available or a dry-run/diff would
  modify files.
- `--diff` should produce focused unified hunks, not full-file replacements.
