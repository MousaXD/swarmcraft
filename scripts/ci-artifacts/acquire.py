#!/usr/bin/env python3
"""Fail-closed acquisition of the exact Tauri 2.11.4 CI bundler assets.

Only GitHub-hosted official upstream sources are used. Every downloaded and
restored file is SHA-256 verified before it can enter Tauri's tools directory.
This helper is CI-only; it does not replace the application's runtime installer.
"""
from __future__ import annotations

import argparse
import hashlib
import io
import os
from pathlib import Path, PurePosixPath
import platform
import shutil
import stat
import sys
import tempfile
import time
from dataclasses import dataclass
import urllib.error
import urllib.parse
import urllib.request
import zipfile


class ArtifactError(RuntimeError):
    pass


class IntegrityError(ArtifactError):
    pass


class AcquisitionError(ArtifactError):
    pass


@dataclass(frozen=True)
class Artifact:
    filename: str
    url: str
    sha256: str


# SHA-256 values for Tauri 2.11.4's exact tool names. All URLs are official
# upstream GitHub release assets or GitHub raw files at immutable commit SHAs.
# Tauri intentionally modifies bytes 8..11 of linuxdeploy after the build
# begins; check the pristine input before Tauri's deliberate mutation.
LINUX = (
    Artifact("AppRun-x86_64", "https://github.com/tauri-apps/binary-releases/releases/download/apprun-old/AppRun-x86_64",
             "f30140a43a0a59e46db21bdefdf749b9e9f2c6946e92afabbacf98b8ae73fb4f"),
    Artifact("linuxdeploy-x86_64.AppImage", "https://github.com/tauri-apps/binary-releases/releases/download/linuxdeploy/linuxdeploy-x86_64.AppImage",
             "e762bea85c8eb0d4b3508d46e5c1f037f717d0f9303ae3b4aafc8b04991fa1ef"),
    Artifact("linuxdeploy-plugin-gtk.sh", "https://raw.githubusercontent.com/tauri-apps/linuxdeploy-plugin-gtk/b5eb8d05b4c0ed40107fe2158c5d8527f94568ef/linuxdeploy-plugin-gtk.sh",
             "cb379f9b0733e9ad9f8bd78f8c2fa038aef2478523bb7d4c8e64ff6a1ea3501a"),
    Artifact("linuxdeploy-plugin-gstreamer.sh", "https://raw.githubusercontent.com/tauri-apps/linuxdeploy-plugin-gstreamer/2a2e67491c32995a3f279ad0ecbe77abd512b42a/linuxdeploy-plugin-gstreamer.sh",
             "c107b49d84edbffc6ab226ed1007e0626a4f7aa2c3a36b7782bef62351d49e94"),
    Artifact("linuxdeploy-plugin-appimage.AppImage", "https://github.com/linuxdeploy/linuxdeploy-plugin-appimage/releases/download/continuous/linuxdeploy-plugin-appimage-x86_64.AppImage",
             "49d6a17160675a6bd1781699aae6bdf7692d98552e02a3671d2183d10547842e"),
)
WINDOWS = (
    Artifact("nsis-3.11.zip", "https://github.com/tauri-apps/binary-releases/releases/download/nsis-3.11/nsis-3.11.zip",
             "c7d27f780ddb6cffb4730138cd1591e841f4b7edb155856901cdf5f214394fa1"),
    Artifact("nsis_tauri_utils.dll", "https://github.com/tauri-apps/nsis-tauri-utils/releases/download/nsis_tauri_utils-v0.5.3/nsis_tauri_utils.dll",
             "5ba143b5db4a87d32d6e7802e033330aae56cbceabe0d1e3ba41948385ad4709"),
)
MAX_DOWNLOAD = 64 * 1024 * 1024
ALLOWED_HOSTS = {
    "github.com", "api.github.com", "raw.githubusercontent.com",
    "release-assets.githubusercontent.com", "objects.githubusercontent.com",
    "github-releases.githubusercontent.com",
}


def validate_target(target: str, system: str, machine: str) -> tuple[Artifact, ...]:
    arch = machine.lower()
    if target == "linux" and system == "Linux" and arch in {"x86_64", "amd64"}:
        return LINUX
    if target == "windows" and system == "Windows" and arch in {"amd64", "x86_64"}:
        return WINDOWS
    raise IntegrityError(f"unsupported platform/architecture: target={target} system={system} arch={machine}")


def verified_digest(data: bytes, expected: str) -> None:
    if len(expected) != 64 or any(char not in "0123456789abcdef" for char in expected):
        raise IntegrityError("invalid pinned SHA-256 metadata")
    if hashlib.sha256(data).hexdigest() != expected:
        raise IntegrityError("artifact SHA-256 mismatch; refusing unverified bytes")


def verify_file(path: Path, expected: str) -> None:
    if not path.is_file() or path.is_symlink():
        raise IntegrityError(f"missing, non-file or symlinked artifact: {path.name}")
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    if digest.hexdigest() != expected:
        raise IntegrityError(f"cached artifact SHA-256 mismatch: {path.name}")


def request_bytes(url: str, *, binary: bool = True) -> bytes:
    if urllib.parse.urlparse(url).hostname not in ALLOWED_HOSTS:
        raise IntegrityError("unapproved artifact source hostname")
    headers = {"User-Agent": "SwarmCraft-CI-Artifacts/1", "Accept": "application/octet-stream" if binary else "application/vnd.github+json"}
    request = urllib.request.Request(url, headers=headers)
    with urllib.request.urlopen(request, timeout=30) as response:
        if urllib.parse.urlparse(response.geturl()).hostname not in ALLOWED_HOSTS:
            raise IntegrityError("unapproved redirect destination")
        data = response.read(MAX_DOWNLOAD + 1 if binary else 1024 * 1024 + 1)
    if len(data) > (MAX_DOWNLOAD if binary else 1024 * 1024):
        raise IntegrityError("artifact or metadata exceeds size limit")
    return data


def asset_api_url(asset: Artifact) -> str | None:
    parsed = urllib.parse.urlparse(asset.url)
    if parsed.hostname != "github.com":
        return None
    parts = parsed.path.strip("/").split("/")
    if len(parts) != 6 or parts[2:4] != ["releases", "download"]:
        raise IntegrityError("unrecognized release asset URL")
    owner, repo, _, _, tag, name = parts
    if not all((owner, repo, tag, name)) or name != asset.url.rsplit("/", 1)[-1]:
        raise IntegrityError("invalid release artifact identity")
    url = f"https://api.github.com/repos/{owner}/{repo}/releases/tags/{tag}"
    import json
    release = json.loads(request_bytes(url, binary=False))
    matches = [a for a in release.get("assets", []) if a.get("name") == name]
    if len(matches) != 1 or matches[0].get("browser_download_url") != asset.url:
        raise IntegrityError("upstream release asset identity mismatch")
    digest = matches[0].get("digest")
    if digest is not None and digest != "sha256:" + asset.sha256:
        raise IntegrityError("upstream release published a different SHA-256")
    asset_id = matches[0].get("id")
    if not isinstance(asset_id, int) or asset_id <= 0:
        raise IntegrityError("release asset ID missing")
    return f"https://api.github.com/repos/{owner}/{repo}/releases/assets/{asset_id}"


def download_verified(asset: Artifact) -> bytes:
    # Try the canonical browser URL and GitHub's official asset API route;
    # bounded backoff, no alternate versions or unauthenticated mirrors.
    for attempt in range(4):
        urls = [asset.url]
        try:
            api = asset_api_url(asset)
            if api:
                urls.append(api)
        except IntegrityError:
            raise
        except (urllib.error.URLError, TimeoutError, OSError, ValueError):
            pass  # metadata endpoint unavailable; try the canonical source
        for source in urls:
            try:
                data = request_bytes(source)
                verified_digest(data, asset.sha256)
                return data
            except IntegrityError:
                raise
            except (urllib.error.HTTPError, urllib.error.URLError, TimeoutError, OSError) as exc:
                status = getattr(exc, "code", None)
                print(f"Download attempt {attempt + 1}/4 for {asset.filename}: unavailable (HTTP {status or 'network'}); no unverified fallback", flush=True)
        if attempt < 3:
            time.sleep((3, 8, 18)[attempt])
    raise AcquisitionError(f"all pinned upstream acquisition attempts failed for {asset.filename}")


def ensure_cached(asset: Artifact, cache_dir: Path, downloader=download_verified) -> Path:
    cache_dir.mkdir(parents=True, exist_ok=True)
    target = cache_dir / asset.filename
    if target.exists() or target.is_symlink():
        verify_file(target, asset.sha256)
        return target
    data = downloader(asset)
    verified_digest(data, asset.sha256)
    fd, temp = tempfile.mkstemp(prefix=".artifact-", dir=cache_dir)
    try:
        with os.fdopen(fd, "wb") as out:
            out.write(data)
            out.flush()
            os.fsync(out.fileno())
        os.replace(temp, target)
    finally:
        if os.path.exists(temp):
            os.unlink(temp)
    verify_file(target, asset.sha256)
    return target


def stage_linux(cache: dict[str, Path], tools: Path) -> None:
    tools.mkdir(parents=True, exist_ok=True)
    for asset in LINUX:
        dest = tools / asset.filename
        if dest.exists() or dest.is_symlink():
            verify_file(dest, asset.sha256)
            continue
        fd, temp = tempfile.mkstemp(prefix=".tool-", dir=tools)
        try:
            with os.fdopen(fd, "wb") as out, cache[asset.filename].open("rb") as inp:
                shutil.copyfileobj(inp, out)
            verify_file(Path(temp), asset.sha256)
            os.chmod(temp, 0o755)
            os.replace(temp, dest)
        finally:
            if os.path.exists(temp):
                os.unlink(temp)
        verify_file(dest, asset.sha256)
    print("Verified Linux AppRun, linuxdeploy and all three plugin inputs", flush=True)


def safe_unzip(source: Path, root: Path) -> None:
    total = 0
    with zipfile.ZipFile(source) as archive:
        for info in archive.infolist():
            name = info.filename
            path = PurePosixPath(name)
            if not name or name.startswith("/") or "\\" in name or any(part in {"", ".", ".."} for part in path.parts):
                raise IntegrityError("unsafe NSIS archive entry")
            if (info.external_attr >> 16) & 0o170000 == stat.S_IFLNK:
                raise IntegrityError("symlink in NSIS archive")
            if path.parts[0] != "nsis-3.11":
                raise IntegrityError("NSIS archive contains unexpected top-level directory")
            total += info.file_size
            if total > 128 * 1024 * 1024:
                raise IntegrityError("NSIS archive unpacked size limit exceeded")
            dest = root.joinpath(*path.parts)
            if info.is_dir():
                dest.mkdir(parents=True, exist_ok=True)
            else:
                dest.parent.mkdir(parents=True, exist_ok=True)
                if dest.exists():
                    raise IntegrityError("duplicate NSIS archive entry")
                with archive.open(info) as inp, dest.open("xb") as out:
                    shutil.copyfileobj(inp, out)


def stage_windows(cache: dict[str, Path], tools: Path) -> None:
    tools.mkdir(parents=True, exist_ok=True)
    dest = tools / "NSIS"
    if dest.exists() or dest.is_symlink():
        raise IntegrityError("refusing unverified pre-existing NSIS tools directory")
    temp = Path(tempfile.mkdtemp(prefix=".nsis-stage-", dir=tools))
    try:
        safe_unzip(cache["nsis-3.11.zip"], temp)
        nsis = temp / "nsis-3.11"
        for required in ("makensis.exe", "Bin/makensis.exe", "Stubs/lzma-x86-unicode", "Stubs/lzma_solid-x86-unicode"):
            if not (nsis / required).is_file():
                raise IntegrityError(f"NSIS archive missing required {required}")
        plugin = nsis / "Plugins" / "x86-unicode" / "additional" / "nsis_tauri_utils.dll"
        plugin.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(cache["nsis_tauri_utils.dll"], plugin)
        verify_file(plugin, WINDOWS[1].sha256)
        os.replace(nsis, dest)
        print("Verified and staged NSIS 3.11 and nsis_tauri_utils v0.5.3", flush=True)
    finally:
        shutil.rmtree(temp, ignore_errors=True)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", required=True, choices=("linux", "windows"))
    parser.add_argument("--source-cache", type=Path, required=True)
    args = parser.parse_args()
    assets = validate_target(args.target, platform.system(), platform.machine())
    cache_dir = args.source_cache / args.target
    if args.source_cache.is_symlink() or cache_dir.is_symlink():
        raise IntegrityError("symlinked artifact cache directory")
    if args.target == "linux":
        tools = Path(os.environ.get("XDG_CACHE_HOME", str(Path.home() / ".cache"))) / "tauri"
    else:
        local = os.environ.get("LOCALAPPDATA")
        if not local:
            raise IntegrityError("LOCALAPPDATA is missing on Windows runner")
        tools = Path(local) / "tauri"
    if tools.is_symlink():
        raise IntegrityError("symlinked Tauri tools directory")
    files = {item.filename: ensure_cached(item, cache_dir) for item in assets}
    if args.target == "linux":
        stage_linux(files, tools)
    else:
        stage_windows(files, tools)
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except ArtifactError as err:
        print(f"CI artifact integrity gate FAILED: {err}", file=sys.stderr)
        sys.exit(1)
    except (OSError, ValueError, zipfile.BadZipFile) as err:
        print(f"CI artifact staging FAILED: {type(err).__name__}", file=sys.stderr)
        sys.exit(1)
