from __future__ import annotations

import os
import platform
import ssl
import subprocess
import sys
import tarfile
import tempfile
import zipfile
from hashlib import sha256
from importlib.metadata import PackageNotFoundError, version
from pathlib import Path
from urllib.error import URLError
from urllib.request import Request, urlopen

import certifi

ASSET_NAME = "gh-actions-updater"
BINARY_NAME = "gau"
REPO = "Goldziher/gh-actions-updater"


def _platform_triple() -> str:
    system = platform.system().lower()
    machine = platform.machine().lower()

    if system == "windows":
        if machine in {"amd64", "x86_64"}:
            return "x86_64-pc-windows-gnu"
        raise RuntimeError("32-bit Windows is not supported")

    if system == "linux":
        if machine in {"amd64", "x86_64"}:
            return "x86_64-unknown-linux-gnu"
        if machine in {"aarch64", "arm64"}:
            return "aarch64-unknown-linux-gnu"

    if system == "darwin":
        if machine in {"amd64", "x86_64"}:
            return "x86_64-apple-darwin"
        if machine in {"aarch64", "arm64"}:
            return "aarch64-apple-darwin"

    raise RuntimeError(f"Unsupported platform: {system} {machine}")


def _python_version_to_tag(version: str) -> str:
    if "rc" in version:
        core, suffix = version.split("rc", 1)
        return f"{core}-rc.{suffix}"
    return version


def _asset(version: str) -> tuple[str, str]:
    tag = _python_version_to_tag(version)
    triple = _platform_triple()
    extension = "zip" if "windows" in triple else "tar.gz"
    url = (
        f"https://github.com/{REPO}/releases/download/"
        f"v{tag}/{ASSET_NAME}-{triple}.{extension}"
    )
    return url, extension


def _checksums_url(version: str) -> str:
    tag = _python_version_to_tag(version)
    return f"https://github.com/{REPO}/releases/download/v{tag}/checksums.txt"


def _download(url: str, destination: Path) -> None:
    request = Request(url, headers={"User-Agent": "gh-actions-updater-python-wrapper"})
    context = ssl.create_default_context(cafile=certifi.where())
    try:
        with urlopen(request, timeout=30, context=context) as response:
            if response.status != 200:
                raise RuntimeError(f"HTTP {response.status}: {response.reason}")
            destination.write_bytes(response.read())
    except URLError as exc:
        raise RuntimeError(f"Failed to download binary: {exc}") from exc


def _binary_filename() -> str:
    return f"{BINARY_NAME}.exe" if platform.system().lower() == "windows" else BINARY_NAME


def _extract(archive: Path, extension: str, destination: Path) -> None:
    binary = _binary_filename()
    if extension == "zip":
        with zipfile.ZipFile(archive) as archive_file:
            for name in archive_file.namelist():
                if name.endswith(binary):
                    with archive_file.open(name) as src, destination.open("wb") as dst:
                        dst.write(src.read())
                    return
    else:
        with tarfile.open(archive, "r:gz") as archive_file:
            for member in archive_file.getmembers():
                if member.name.endswith(binary):
                    source = archive_file.extractfile(member)
                    if source is None:
                        continue
                    with source, destination.open("wb") as dst:
                        dst.write(source.read())
                    return
    raise RuntimeError("Binary not found in downloaded archive")


def _sha256(path: Path) -> str:
    digest = sha256()
    with path.open("rb") as file:
        for chunk in iter(lambda: file.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _expected_checksum(checksums: Path, archive: Path) -> str:
    archive_name = archive.name
    for line in checksums.read_text(encoding="utf-8").splitlines():
        parts = line.split()
        if len(parts) >= 2 and parts[1] == archive_name:
            return parts[0]
    raise RuntimeError(f"Checksum not found for {archive_name}")


def _verify_checksum(archive: Path, checksums: Path) -> None:
    expected = _expected_checksum(checksums, archive)
    actual = _sha256(archive)
    if actual != expected:
        raise RuntimeError(f"Checksum mismatch for {archive.name}")


def _cache_path(version: str) -> Path:
    base = Path(os.getenv("XDG_CACHE_HOME", Path.home() / ".cache"))
    cache_dir = base / BINARY_NAME / version
    cache_dir.mkdir(parents=True, exist_ok=True)
    return cache_dir / _binary_filename()


def _package_version() -> str:
    try:
        return version("gh-actions-updater")
    except PackageNotFoundError:
        from . import __version__

        return __version__


def ensure_binary() -> str:
    package_version = _package_version()

    override = os.getenv("GH_ACTIONS_UPDATER_BINARY")
    if override:
        return override

    binary_path = _cache_path(package_version)
    if binary_path.exists() and os.access(binary_path, os.X_OK):
        return str(binary_path)

    url, extension = _asset(package_version)
    print(f"Downloading {BINARY_NAME} v{package_version}...", file=sys.stderr)

    with tempfile.TemporaryDirectory() as temp_dir:
        archive_path = Path(temp_dir) / Path(url).name
        checksums_path = Path(temp_dir) / "checksums.txt"
        _download(url, archive_path)
        _download(_checksums_url(package_version), checksums_path)
        _verify_checksum(archive_path, checksums_path)
        _extract(archive_path, extension, binary_path)

    if platform.system().lower() != "windows":
        binary_path.chmod(0o755)

    return str(binary_path)


def run_binary(args: list[str]) -> None:
    binary_path = ensure_binary()
    result = subprocess.run([binary_path, *args], check=False)
    sys.exit(result.returncode)
