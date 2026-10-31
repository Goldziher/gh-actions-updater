#!/usr/bin/env bash
#MISE description="Set the version across packages, hook channels and docs (usage: mise run set-version 1.2.3)"
#USAGE arg "<version>" help="Semver such as 1.2.3 or 1.2.3-rc.1, with or without a leading v"
# shellcheck disable=SC2154 # usage_* are set by mise from the #USAGE lines
set -euo pipefail

python3 - "${usage_version#v}" <<'PY'
import json
import re
import sys
from pathlib import Path

version = sys.argv[1]
if not re.fullmatch(r"\d+\.\d+\.\d+(?:-rc\.\d+)?", version):
    raise SystemExit(f"invalid version: {version}")

py_version = re.sub(r"-rc\.(\d+)$", r"rc\1", version)

def replace(path: str, pattern: str, repl: str) -> None:
    file = Path(path)
    text = file.read_text(encoding="utf-8")
    new = re.sub(pattern, repl, text, count=1, flags=re.MULTILINE)
    if new == text:
        raise SystemExit(f"pattern not found in {path}: {pattern}")
    file.write_text(new, encoding="utf-8")

replace("Cargo.toml", r'^version = ".*"', f'version = "{version}"')
replace(
    "pip-package/pyproject.toml",
    r'^version = ".*"',
    f'version = "{py_version}"',
)
replace(
    "pip-package/gh_actions_updater/__init__.py",
    r'^__version__ = ".*"',
    f'__version__ = "{py_version}"',
)

package = Path("npm-package/package.json")
data = json.loads(package.read_text(encoding="utf-8"))
data["version"] = version
package.write_text(json.dumps(data, indent=2) + "\n", encoding="utf-8")

replace("README.md", r'"version": ".*"', f'"version": "{version}"')
replace("README.md", r"rev: v[0-9][^\n]*", f"rev: v{version}")
replace("README.md", r'revision = "v[0-9][^"]*"', f'revision = "v{version}"')

hooks = Path("poly-hooks.toml")
text = hooks.read_text(encoding="utf-8")
new = re.sub(r"gh-actions-updater@[0-9][^\s\"]*", f"gh-actions-updater@{version}", text)
new = re.sub(r"gh-actions-updater --version [0-9][^\s\"]*", f"gh-actions-updater --version {version}", new)
if new.count(version) < 2 * text.count("[[hooks]]"):
    raise SystemExit("hook channel versions not found in poly-hooks.toml")
hooks.write_text(new, encoding="utf-8")

lock = Path("Cargo.lock")
text = lock.read_text(encoding="utf-8")
new = re.sub(
    r'(\[\[package\]\]\nname = "gh-actions-updater"\nversion = ")[^"]+(")',
    rf"\g<1>{version}\2",
    text,
    count=1,
)
if new == text:
    raise SystemExit("gh-actions-updater package entry not found in Cargo.lock")
lock.write_text(new, encoding="utf-8")

print(f"set version to {version} (PyPI: {py_version})")
PY
