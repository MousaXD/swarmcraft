#!/usr/bin/env python3
"""Publish one verified preview without changing older releases."""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import re
import subprocess

from verify_artifacts import COMMIT, PLATFORM_EXTENSIONS, sha256_file, verify_checksum


def gh(*args: str) -> str:
    return subprocess.run(["gh", *args], check=True, text=True, stdout=subprocess.PIPE).stdout


def staged_assets(directory: Path, commit: str) -> dict[str, tuple[int, str]]:
    manifest_name = "release-provenance.json"
    files = {path.name: path for path in directory.iterdir()}
    if any(not path.is_file() or path.is_symlink() for path in files.values()):
        raise ValueError("preview staging contains an unsafe entry")
    manifest = json.loads(files[manifest_name].read_text(encoding="utf-8"))
    if manifest["commit"] != commit or manifest["channel"] != "preview":
        raise ValueError("preview staging belongs to a different commit or channel")
    packages = manifest["assets"]
    names = {package["file"] for package in packages}
    expected_names = {manifest_name, *names, *(f"{name}.sha256" for name in names)}
    if (
        len(packages) != len(PLATFORM_EXTENSIONS)
        or {package["platform"] for package in packages} != PLATFORM_EXTENSIONS.keys()
        or len(names) != len(packages)
        or files.keys() != expected_names
    ):
        raise ValueError("preview staging does not contain every verified package and checksum")
    if any(not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.-]*", name) for name in names):
        raise ValueError("preview staging contains a name unsuitable for a GitHub release asset")
    assets = {
        name: (files[name].stat().st_size, f"sha256:{sha256_file(files[name])}")
        for name in expected_names - names
    }
    for package in packages:
        name = package["file"]
        digest = verify_checksum(files[name], files[f"{name}.sha256"])
        size = files[name].stat().st_size
        if (size, digest) != (package["bytes"], package["sha256"]):
            raise ValueError(f"staged package changed after verification: {name}")
        assets[name] = (size, f"sha256:{digest}")
    return assets


def verified_remote_assets(release: dict, expected: dict[str, tuple[int, str]]) -> set[str]:
    names = set()
    for asset in release["assets"]:
        name = asset["name"]
        if name in names or asset["state"] != "uploaded" or expected.get(name) != (asset["size"], asset["digest"]):
            raise ValueError(f"remote preview asset conflicts with verified staging: {name}")
        names.add(name)
    return names


def check_release(release: dict, tag: str, commit: str) -> None:
    if release["tag_name"] != tag or release["prerelease"] is not True:
        raise ValueError("GitHub returned an unexpected preview release")
    if release["draft"] and release["target_commitish"] != commit:
        raise ValueError("preview draft targets another commit")


def bind_tag(repo: str, tag: str, commit: str, *, create: bool) -> None:
    full_ref = f"refs/tags/{tag}"
    candidates = json.loads(gh("api", f"repos/{repo}/git/matching-refs/tags/{tag}"))
    refs = [ref for ref in candidates if ref["ref"] == full_ref]
    if not refs and create:
        refs = [json.loads(gh("api", f"repos/{repo}/git/refs", "-f", f"ref={full_ref}", "-f", f"sha={commit}"))]
    if len(refs) != 1 or refs[0]["ref"] != full_ref:
        raise ValueError("preview tag is missing or points to another commit")
    target = refs[0]["object"]
    if target["type"] != "commit" or target["sha"] != commit:
        raise ValueError("preview tag is missing or points to another commit")


def publish(repo: str, commit: str, directory: Path, notes: Path) -> None:
    if not COMMIT.fullmatch(commit) or not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repo):
        raise ValueError("invalid preview repository or commit")
    expected = staged_assets(directory, commit)
    tag = f"main-snapshot-{commit}"
    endpoint = f"repos/{repo}/releases"
    # Listing releases with the publisher token also finds draft releases on retry.
    records = gh(
        "api", f"{endpoint}?per_page=100", "--paginate", "--jq", f'.[] | select(.tag_name == "{tag}") | @json',
    )
    matches = [json.loads(record) for record in records.splitlines() if record]
    if len(matches) > 1:
        raise ValueError("multiple GitHub releases match the preview tag")
    if matches:
        release = matches[0]
    else:
        release = json.loads(gh(
            "api", endpoint, "-f", f"tag_name={tag}", "-f", f"target_commitish={commit}",
            "-f", f"name=SwarmCraft Main Snapshot ({commit[:12]})", "-f", f"body={notes.read_text(encoding='utf-8')}",
            "-F", "draft=true", "-F", "prerelease=true", "-f", "make_latest=false",
        ))
    check_release(release, tag, commit)
    present = verified_remote_assets(release, expected)
    if not release["draft"]:
        if present != expected.keys():
            raise ValueError("existing published preview is incomplete")
        bind_tag(repo, tag, commit, create=False)
        print(f"Verified existing preview {tag}")
        return

    missing = sorted(expected.keys() - present)
    if missing:
        gh("release", "upload", tag, "--repo", repo, *(str(directory.resolve() / name) for name in missing))
    release_endpoint = f"{endpoint}/{release['id']}"
    draft = json.loads(gh("api", release_endpoint))
    check_release(draft, tag, commit)
    if draft["draft"] is not True or verified_remote_assets(draft, expected) != expected.keys():
        raise ValueError("preview draft is not ready for publication")
    bind_tag(repo, tag, commit, create=True)
    published = json.loads(gh(
        "api", release_endpoint, "--method", "PATCH", "-F", "draft=false", "-f", "make_latest=false",
    ))
    check_release(published, tag, commit)
    if published["draft"] is not False or verified_remote_assets(published, expected) != expected.keys():
        raise ValueError("GitHub did not confirm publication of every verified asset")
    print(f"Published {tag} after verifying {len(expected)} uploaded assets")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo", required=True)
    parser.add_argument("--commit", required=True)
    parser.add_argument("--assets", required=True, type=Path)
    parser.add_argument("--notes", required=True, type=Path)
    args = parser.parse_args()
    publish(args.repo, args.commit, args.assets, args.notes)


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError, KeyError, subprocess.CalledProcessError) as error:
        raise SystemExit(f"preview publication failed: {error}") from None
