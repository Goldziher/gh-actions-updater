#!/usr/bin/env bash
set -euo pipefail

version="${1#v}"
install_dir="${GHAU_INSTALL_DIR:?GHAU_INSTALL_DIR must be set}"
base_url="https://github.com/Goldziher/gh-actions-updater/releases/download/v${version}"

case "$(uname -s):$(uname -m)" in
  Linux:x86_64) target="x86_64-unknown-linux-gnu" ;;
  Linux:aarch64|Linux:arm64) target="aarch64-unknown-linux-gnu" ;;
  Darwin:x86_64) target="x86_64-apple-darwin" ;;
  Darwin:arm64) target="aarch64-apple-darwin" ;;
  MINGW*:x86_64|MSYS*:x86_64) target="x86_64-pc-windows-gnu" ;;
  *) echo "unsupported runner platform: $(uname -s) $(uname -m)" >&2; exit 2 ;;
esac

if [ "$target" = "x86_64-pc-windows-gnu" ]; then
  archive="gh-actions-updater-${target}.zip"
else
  archive="gh-actions-updater-${target}.tar.gz"
fi

temporary_dir="$(mktemp -d)"
trap 'rm -rf "$temporary_dir"' EXIT
auth_header=()
[ -n "${GH_TOKEN:-}" ] && auth_header=(-H "Authorization: Bearer ${GH_TOKEN}")
curl --fail --silent --show-error --location "${auth_header[@]}" "${base_url}/${archive}" -o "${temporary_dir}/${archive}"
curl --fail --silent --show-error --location "${auth_header[@]}" "${base_url}/checksums.txt" -o "${temporary_dir}/checksums.txt"

expected="$(awk -v archive="$archive" '$2 == archive || $2 == "*" archive { print $1; exit }' "${temporary_dir}/checksums.txt")"
[ -n "$expected" ] || { echo "checksum entry missing for ${archive}" >&2; exit 1; }
if command -v sha256sum >/dev/null 2>&1; then
  actual="$(sha256sum "${temporary_dir}/${archive}" | awk '{print $1}')"
else
  actual="$(shasum -a 256 "${temporary_dir}/${archive}" | awk '{print $1}')"
fi
[ "$actual" = "$expected" ] || { echo "checksum verification failed for ${archive}" >&2; exit 1; }

mkdir -p "$install_dir" "$temporary_dir/unpacked"
if [[ "$archive" == *.zip ]]; then
  unzip -q "${temporary_dir}/${archive}" -d "$temporary_dir/unpacked"
  binary="$(find "$temporary_dir/unpacked" -type f -name 'gau.exe' -print -quit)"
  destination="${install_dir}/gau.exe"
else
  tar -xzf "${temporary_dir}/${archive}" -C "$temporary_dir/unpacked"
  binary="$(find "$temporary_dir/unpacked" -type f -name gau -print -quit)"
  destination="${install_dir}/gau"
fi
[ -n "$binary" ] || { echo "gau binary missing from ${archive}" >&2; exit 1; }
temporary_binary="${destination}.tmp.$$"
cp "$binary" "$temporary_binary"
chmod +x "$temporary_binary"
mv "$temporary_binary" "$destination"
