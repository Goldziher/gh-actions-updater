#!/usr/bin/env bash
set -euo pipefail

# package-release.sh: Bundle the gau binary with README, LICENSE, and .pre-commit-hooks.yaml
# NOTE: the archive contents are at the ROOT (no leading staging-dir component) so

if [ $# -ne 1 ]; then
  echo "Usage: $0 <target-triple>" >&2
  exit 1
fi

TRIPLE="$1"

case "$TRIPLE" in
x86_64-unknown-linux-gnu | aarch64-unknown-linux-gnu)
  SYSTEM="linux"
  BINEXT=""
  ;;
x86_64-apple-darwin | aarch64-apple-darwin)
  SYSTEM="macos"
  BINEXT=""
  ;;
x86_64-pc-windows-gnu)
  SYSTEM="windows"
  BINEXT=".exe"
  ;;
*)
  echo "Unknown target triple: $TRIPLE" >&2
  exit 1
  ;;
esac

RELEASE_DIR="target/${TRIPLE}/release"
BINARY_PATH="${RELEASE_DIR}/gau${BINEXT}"

if [ ! -f "$BINARY_PATH" ]; then
  echo "Binary not found at $BINARY_PATH" >&2
  exit 1
fi

for required_file in README.md LICENSE .pre-commit-hooks.yaml; do
  if [ ! -f "$required_file" ]; then
    echo "Required file not found: $required_file" >&2
    exit 1
  fi
done

STAGING_DIR="gh-actions-updater-staging-${TRIPLE}"
rm -rf "$STAGING_DIR"
mkdir -p "$STAGING_DIR"

cp "$BINARY_PATH" "$STAGING_DIR/gau${BINEXT}"
cp README.md "$STAGING_DIR/"
cp LICENSE "$STAGING_DIR/"
cp .pre-commit-hooks.yaml "$STAGING_DIR/"

chmod +x "$STAGING_DIR/gau${BINEXT}"

case "$SYSTEM" in
linux | macos)
  tar czf "gh-actions-updater-${TRIPLE}.tar.gz" -C "$STAGING_DIR" .
  echo "✓ Created gh-actions-updater-${TRIPLE}.tar.gz"
  ;;
windows)
  (cd "$STAGING_DIR" && {
    zip -q -r "../gh-actions-updater-${TRIPLE}.zip" . ||
      (command -v 7z >/dev/null 2>&1 && 7z a -tzip "../gh-actions-updater-${TRIPLE}.zip" . >/dev/null 2>&1) ||
      (command -v powershell >/dev/null 2>&1 && powershell -Command "Compress-Archive -Path '*' -DestinationPath '../gh-actions-updater-${TRIPLE}.zip' -Force") ||
      (echo "No suitable zip tool found (zip, 7z, or powershell)" >&2 && exit 1)
  })
  echo "✓ Created gh-actions-updater-${TRIPLE}.zip"
  ;;
esac

rm -rf "$STAGING_DIR"
echo "✓ Release package ready: gh-actions-updater-${TRIPLE}.$([ "$SYSTEM" = "windows" ] && echo "zip" || echo "tar.gz")"
