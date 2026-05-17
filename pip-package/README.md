# gh-actions-updater

PyPI wrapper for the `gh-actions-updater` Rust CLI.

```bash
pip install gh-actions-updater
gh-actions-updater --help
```

The Python package downloads the matching binary from GitHub Releases on first
use and caches it under the user's cache directory. Set
`GH_ACTIONS_UPDATER_BINARY` to use a local binary instead.
