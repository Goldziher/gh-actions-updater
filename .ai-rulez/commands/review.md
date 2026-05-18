---
priority: high
aliases: [rev]
usage: "/review"
description: "Review current changes for correctness, performance, and release risk"
---

# Review

Review staged and unstaged changes with a bug-first stance.

## Steps

1. Inspect `git diff` and `git diff --staged`.
2. Prioritize findings in this order:
   - incorrect updates or unsafe rewrites
   - broken cache/TTL/no-cache behavior
   - excessive metadata fetches or missing deduplication
   - recursive discovery regressions
   - package wrapper or release asset mismatches
   - missing tests
3. Verify public contracts:
   - package name remains `gh-actions-updater`
   - installed command is `gau`
   - JSON `changed` and `would_change` semantics are stable
   - config precedence is CLI, env, config, defaults
   - `# gau: ignore` and `[update].exclude` do not hide references from reports
4. Check docs and `.ai-rulez` whenever CLI/config/release behavior changes.
5. Present findings first with file/line references, then residual risk.
