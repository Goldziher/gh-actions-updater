# gh-actions-updater

PyPI package for the `gau` Rust CLI.

```bash
pip install gh-actions-updater
gau --help
```

Run without installing into the current environment:

```bash
uvx --from gh-actions-updater gau --check .
uvx --from gh-actions-updater gau --init
```

Common usage:

```bash
gau --init
gau --check .
gau --update --diff .
gau --latest-hash --update .
gau --pin-style major --update .
gau -r ~/workspace --check
```

The package name is `gh-actions-updater`; the installed command is `gau`.
The Python wrapper downloads the matching `gau` binary from GitHub Releases on
first use, verifies the release checksum, and caches it under the user's cache
directory. Set `GH_ACTIONS_UPDATER_BINARY` to use a local binary instead.

Configuration lives in `.gh-actions-updater.toml`. Generate one with:

```bash
gau --init
gau --init --recursive --force
```

Useful flags:

- `--cache-ttl <DURATION>`: `15m`, `6h`, `7d`, `0`, or `never`
- `--no-cache`: disable cache reads and writes
- `--refresh-cache`: force fresh metadata
- `-r, --recursive`: scan nested repositories/workspaces
- `--pin-style <preserve|major|minor|full>`: control semver tag pin formatting
- `--threads <N>`: override Rayon available-core default
- `--format json`: machine-readable output

Skip specific updates with config:

```toml
[update]
exclude = ["actions/checkout", "owner/repo/.github/workflows/deploy.yml@v*"]
```

Or inline:

```yaml
- uses: actions/checkout@v4 # gau: ignore
```

See the repository README for the complete CLI and configuration reference.
