# Contributing

## Setup

Install [mise](https://mise.jdx.dev), then run `mise run setup` to install the locked toolchain, fetch
dependencies and install the git hooks. `mise tasks` lists the build, test, lint and package tasks.

## AI governance

Run `mise run governance:generate` after editing `.ai-rulez/`; it runs the latest ai-rulez through `npx`.
Run `mise run governance:check` to validate native OKF sources, the lock, and generated output.

Run `mise run governance:hooks:install` to provision the reviewed governance hooks. Existing machine preferences
are preserved.
