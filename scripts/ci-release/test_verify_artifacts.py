#!/usr/bin/env python3
"""Offline, fail-closed release publication boundary tests (no network/certificates)."""
from __future__ import annotations

import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import tomllib
import unittest

MODULE = Path(__file__).with_name("verify_artifacts.py")
spec = importlib.util.spec_from_file_location("verify_artifacts", MODULE)
verify_artifacts = importlib.util.module_from_spec(spec)
assert spec.loader is not None
spec.loader.exec_module(verify_artifacts)

SHA = "a" * 40
with (MODULE.parents[2] / "Cargo.toml").open("rb") as _root_cargo:
    VERSION = tomllib.load(_root_cargo)["workspace"]["package"]["version"]
PACKAGES = {
    "linux": (f"SwarmCraft_{VERSION}_amd64.deb",),
    "windows": (f"SwarmCraft_{VERSION}_x64-setup.exe",),
    "macos-arm64": (f"SwarmCraft_{VERSION}_arm64.dmg",),
    "macos-x86_64": (f"SwarmCraft_{VERSION}_x86_64.dmg",),
    "fabric": (f"swarmcraft-fabric-{VERSION}.jar",),
}


class PublicationBoundaryTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name)
        self.source = self.base / "downloaded"
        self.out = self.base / "release-assets"

    def make_artifacts(self, channel="preview"):
        for platform, folder in verify_artifacts.expected_groups(channel, SHA).items():
            dest = self.source / folder
            dest.mkdir(parents=True, exist_ok=True)
            packages = list(PACKAGES[platform])
            if channel == "production" and platform == "linux":
                packages.append(f"SwarmCraft_{VERSION}_amd64.AppImage")
            for name in packages:
                data = f"fixture:{platform}:{name}".encode()
                (dest / name).write_bytes(data)
                digest = hashlib.sha256(data).hexdigest()
                # Both original artifact-manifest conventions are supported.
                target = f"dist/{name}" if platform != "windows" else name
                (dest / f"{name}.sha256").write_text(f"{digest}  {target}\n")
            if channel == "production":
                status = verify_artifacts.expected_status_file(platform)
                if status:
                    if platform in verify_artifacts.NATIVE_CHECKS:
                        package = dest / packages[0]
                        receipt = {
                            "schema": "swarmcraft-native-signing-v1", "commit": SHA,
                            "platform": platform, "file": package.name,
                            "bytes": package.stat().st_size, "sha256": verify_artifacts.sha256_file(package),
                            "checks": verify_artifacts.NATIVE_CHECKS[platform],
                            "identity": "A" * (40 if platform == "windows" else 10),
                        }
                        (dest / status).write_text(json.dumps(receipt))
                    else:
                        (dest / status).write_text("Checksum-protected Linux release package.\n")

    def verify(self, channel="preview"):
        return verify_artifacts.verify(self.source, self.out, channel, SHA, VERSION)

    def test_preview_exact_sha_checksum_all_five_platforms(self):
        self.make_artifacts()
        entries = self.verify()
        self.assertEqual(len(entries), 5)
        self.assertTrue((self.out / "release-provenance.json").is_file())
        summary = json.loads((self.out / "release-provenance.json").read_text())
        self.assertEqual(summary["commit"], SHA)
        self.assertEqual(summary["version"], VERSION)

    def test_production_requires_appimage_and_status_files(self):
        self.make_artifacts("production")
        self.assertEqual(len(self.verify("production")), 6)


    def test_missing_production_appimage_fails_closed(self):
        self.make_artifacts("production")
        linux = self.source / verify_artifacts.expected_groups("production", SHA)["linux"]
        appimage = linux / f"SwarmCraft_{VERSION}_amd64.AppImage"
        appimage.unlink()
        (linux / f"{appimage.name}.sha256").unlink()
        with self.assertRaisesRegex(ValueError, "requires exactly one .AppImage"):
            self.verify("production")

    def test_malformed_sha256_sidecar_fails_closed(self):
        self.make_artifacts()
        folder = self.source / verify_artifacts.expected_groups("preview", SHA)["linux"]
        (folder / f"{PACKAGES['linux'][0]}.sha256").write_text("not a checksum")
        with self.assertRaisesRegex(ValueError, "malformed SHA-256"):
            self.verify()

    def test_corrupted_package_is_fatal_and_not_published(self):
        self.make_artifacts()
        folder = verify_artifacts.expected_groups("preview", SHA)["windows"]
        (self.source / folder / PACKAGES["windows"][0]).write_bytes(b"tampered")
        with self.assertRaisesRegex(ValueError, "SHA-256 mismatch"):
            self.verify()
        self.assertFalse(self.out.exists())

    def test_missing_platform_cannot_publish_even_when_other_assets_exist(self):
        self.make_artifacts()
        folder = self.source / verify_artifacts.expected_groups("preview", SHA)["macos-x86_64"]
        for child in folder.iterdir():
            child.unlink()
        folder.rmdir()
        with self.assertRaisesRegex(ValueError, "missing/unexpected exact-run"):
            self.verify()

    def test_wrong_commit_group_is_rejected(self):
        self.make_artifacts()
        with self.assertRaisesRegex(ValueError, "missing/unexpected exact-run"):
            verify_artifacts.verify(self.source, self.out, "preview", "b" * 40, VERSION)

    def test_unknown_asset_and_bad_checksum_target_are_rejected(self):
        self.make_artifacts()
        linux = self.source / verify_artifacts.expected_groups("preview", SHA)["linux"]
        (linux / "injected.txt").write_text("bad")
        with self.assertRaisesRegex(ValueError, "missing/unknown"):
            self.verify()
        (linux / "injected.txt").unlink()
        (linux / f"{PACKAGES['linux'][0]}.sha256").write_text("f" * 64 + "  ../untrusted.deb\n")
        with self.assertRaisesRegex(ValueError, "wrong asset"):
            self.verify()

    def test_mismatched_version_is_rejected(self):
        self.make_artifacts()
        with self.assertRaisesRegex(ValueError, "package version differs"):
            verify_artifacts.verify(self.source, self.out, "preview", SHA, "999.999.999")

    def test_macos_architecture_must_match_its_artifact_group(self):
        self.make_artifacts()
        arm = self.source / verify_artifacts.expected_groups("preview", SHA)["macos-arm64"]
        for file in arm.iterdir():
            file.unlink()
        name = f"SwarmCraft_{VERSION}_x64.dmg"
        data = b"mislabelled-arm"
        (arm / name).write_bytes(data)
        (arm / f"{name}.sha256").write_text(hashlib.sha256(data).hexdigest() + f"  {name}\n")
        with self.assertRaisesRegex(ValueError, "architecture"):
            self.verify()

    def test_missing_production_signing_status_is_fatal(self):
        self.make_artifacts("production")
        folder = self.source / verify_artifacts.expected_groups("production", SHA)["windows"]
        (folder / verify_artifacts.expected_status_file("windows")).unlink()
        with self.assertRaisesRegex(ValueError, "missing/unknown"):
            self.verify("production")

    def test_production_receipt_must_match_exact_bytes_commit_platform_and_checks(self):
        for field, value in (
            ("sha256", "b" * 64), ("commit", "b" * 40), ("file", "other.exe"),
            ("bytes", 123), ("platform", "macos-arm64"), ("checks", []), ("identity", ""),
        ):
            with self.subTest(field=field):
                self.make_artifacts("production")
                folder = self.source / verify_artifacts.expected_groups("production", SHA)["windows"]
                path = folder / verify_artifacts.expected_status_file("windows")
                record = json.loads(path.read_text())
                record[field] = value
                path.write_text(json.dumps(record))
                with self.assertRaises(ValueError):
                    self.verify("production")
                self.assertFalse(self.out.exists())

    def test_plaintext_signing_status_cannot_substitute_for_native_receipt(self):
        self.make_artifacts("production")
        folder = self.source / verify_artifacts.expected_groups("production", SHA)["macos-arm64"]
        (folder / verify_artifacts.expected_status_file("macos-arm64")).write_text("Signed and notarized")
        with self.assertRaises(ValueError):
            self.verify("production")

    def test_duplicate_receipt_fields_fail_closed(self):
        self.make_artifacts("production")
        folder = self.source / verify_artifacts.expected_groups("production", SHA)["macos-arm64"]
        path = folder / verify_artifacts.expected_status_file("macos-arm64")
        path.write_text(path.read_text()[:-1] + ', "commit": "' + SHA + '"}')
        with self.assertRaisesRegex(ValueError, "duplicate"):
            self.verify("production")

    def test_symlink_package_is_rejected(self):
        self.make_artifacts()
        folder = self.source / verify_artifacts.expected_groups("preview", SHA)["linux"]
        target = folder / PACKAGES["linux"][0]
        target.unlink()
        target.symlink_to(folder / f"{PACKAGES['linux'][0]}.sha256")
        with self.assertRaisesRegex(ValueError, "symlinks"):
            self.verify()

    def test_nonempty_publication_staging_is_rejected(self):
        self.make_artifacts()
        self.out.mkdir()
        (self.out / "stale.exe").write_bytes(b"stale")
        with self.assertRaisesRegex(ValueError, "must be empty"):
            self.verify()


if __name__ == "__main__":
    unittest.main()
