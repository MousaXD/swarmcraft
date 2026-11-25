import json
from pathlib import Path
import tempfile
import unittest
import zipfile

from verify_artifacts import expected_groups, sha256_file
from verify_published_preview import packages, verify, strict_json

SHA = "a" * 40
VERSION = "0.5.1"


class PublishedPreviewTests(unittest.TestCase):
    def setUp(self):
        self.temp = self.enterContext(tempfile.TemporaryDirectory())
        self.directory = Path(self.temp)

    def make(self, legacy=False):
        records = []
        for platform, name in packages(VERSION).items():
            path = self.directory / name
            if platform == "fabric":
                with zipfile.ZipFile(path, "w") as jar:
                    jar.writestr("fabric.mod.json", json.dumps(
                        {"id": "swarmcraft", "version": VERSION, "environment": "server"}))
            else:
                path.write_bytes(name.encode())
            digest = sha256_file(path)
            path.with_name(name + ".sha256").write_text(f"{digest}  dist/{name}\n")
            records.append({"platform": platform, "source_artifact": expected_groups("preview", SHA)[platform],
                            "file": name, "sha256": digest, "bytes": path.stat().st_size})
        if not legacy:
            self.manifest = self.directory / "release-provenance.json"
            self.manifest.write_text(json.dumps({"commit": SHA, "version": VERSION, "channel": "preview",
                "scope": "current-workflow artifact checksums, not a signing attestation", "assets": records}))
        self.release = {"tag_name": "main-latest" if legacy else "main-snapshot-" + SHA,
                        "tag_target": SHA, "draft": False, "prerelease": True}
        self.refresh()

    def refresh(self):
        self.release["assets"] = [{"name": p.name, "state": "uploaded", "size": p.stat().st_size,
                                   "digest": "sha256:" + sha256_file(p)} for p in self.directory.iterdir()]

    def test_exact_pinned_download(self):
        self.make()
        result = verify(self.directory, self.release, SHA, VERSION)
        self.assertEqual(result["asset_count"], 11)
        self.assertTrue(result["source_manifest_verified"])

    def test_legacy_is_explicit_and_never_source_attested(self):
        self.make(legacy=True)
        with self.assertRaises(ValueError):
            verify(self.directory, self.release, SHA, VERSION)
        result = verify(self.directory, self.release, SHA, VERSION, legacy=True)
        self.assertEqual(result["asset_count"], 10)
        self.assertFalse(result["source_manifest_verified"])

    def test_remote_digest_checks_all_downloaded_bytes(self):
        self.make()
        (self.directory / packages(VERSION)["windows"]).write_bytes(b"tampered")
        with self.assertRaisesRegex(ValueError, "remote integrity metadata"):
            verify(self.directory, self.release, SHA, VERSION)

    def test_new_remote_digest_cannot_bypass_package_checksum(self):
        self.make()
        (self.directory / packages(VERSION)["windows"]).write_bytes(b"tampered")
        self.refresh()
        with self.assertRaisesRegex(ValueError, "SHA-256 mismatch"):
            verify(self.directory, self.release, SHA, VERSION)

    def test_manifest_wrong_commit_group_and_digest_fail(self):
        self.make()
        original = self.manifest.read_text()
        for field, value in (("source_artifact", "wrong-source"), ("sha256", "0" * 64), ("bytes", 1)):
            manifest = json.loads(original)
            manifest["assets"][0][field] = value
            self.manifest.write_text(json.dumps(manifest))
            self.refresh()
            with self.assertRaisesRegex(ValueError, "manifest disagrees"):
                verify(self.directory, self.release, SHA, VERSION)

    def test_retargeted_tag_or_draft_rejected(self):
        self.make()
        for change in ({"tag_target": "b" * 40}, {"draft": True}, {"prerelease": False}):
            with self.assertRaises(ValueError):
                verify(self.directory, {**self.release, **change}, SHA, VERSION)

    def test_extra_missing_duplicate_or_symlink_asset_rejected(self):
        self.make()
        with self.assertRaises(ValueError):
            verify(self.directory, {**self.release, "assets": self.release["assets"] * 2}, SHA, VERSION)
        with self.assertRaises(ValueError):
            verify(self.directory, {**self.release, "assets": self.release["assets"][:-1]}, SHA, VERSION)
        extra = self.directory / "extra"
        extra.touch()
        with self.assertRaises(ValueError):
            verify(self.directory, self.release, SHA, VERSION)
        extra.unlink()
        path = self.directory / packages(VERSION)["windows"]
        path.unlink()
        path.symlink_to(self.manifest)
        with self.assertRaises(ValueError):
            verify(self.directory, self.release, SHA, VERSION)

    def test_fabric_internal_version_required_and_duplicate_json_rejected(self):
        self.make()
        path = self.directory / packages(VERSION)["fabric"]
        with zipfile.ZipFile(path, "w") as jar:
            jar.writestr("fabric.mod.json", '{"id":"swarmcraft","version":"0.0.0","environment":"server"}')
        path.with_name(path.name + ".sha256").write_text(f"{sha256_file(path)}  {path.name}\n")
        self.refresh()
        with self.assertRaisesRegex(ValueError, "Fabric version"):
            verify(self.directory, self.release, SHA, VERSION)
        with self.assertRaisesRegex(ValueError, "duplicate JSON"):
            strict_json('{"commit":"a","commit":"b"}')
