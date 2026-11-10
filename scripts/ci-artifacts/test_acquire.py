"""Offline negative controls for the CI artifact acquisition gate."""
import hashlib
import json
import io
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock
import urllib.error
import zipfile

sys.path.insert(0, str(Path(__file__).parent))
import acquire


def fixture(name, content=b"verified-fixture"):
    return acquire.Artifact(
        name, "https://raw.githubusercontent.com/example/example/0123456789/file",
        hashlib.sha256(content).hexdigest(),
    )


def release_fixture(asset, *, digest=None, download_url=None, asset_id=171):
    return {
        "assets": [{
            "id": asset_id,
            "name": asset.filename,
            "browser_download_url": download_url or asset.url,
            "digest": digest if digest is not None else "sha256:" + asset.sha256,
        }]
    }


class ArtifactIntegrityTests(unittest.TestCase):
    def test_all_manifests_are_pinned_sha256(self):
        for item in (*acquire.LINUX, *acquire.WINDOWS):
            self.assertEqual(len(item.sha256), 64)
            bytes.fromhex(item.sha256)
            self.assertTrue(item.url.startswith("https://"))
            self.assertIn(item.url.split("/")[2], acquire.ALLOWED_HOSTS)

    def test_wrong_architecture_fails_closed(self):
        with self.assertRaises(acquire.IntegrityError):
            acquire.validate_target("linux", "Linux", "aarch64")
        with self.assertRaises(acquire.IntegrityError):
            acquire.validate_target("windows", "Windows", "ARM64")
        with self.assertRaises(acquire.IntegrityError):
            acquire.validate_target("windows", "Linux", "x86_64")

    def test_missing_artifact_and_unavailable_upstream_fail(self):
        item = fixture("missing.bin")
        with tempfile.TemporaryDirectory() as tmp:
            cache = Path(tmp)
            def unavailable(_):
                raise acquire.AcquisitionError("upstream unavailable")
            with self.assertRaises(acquire.AcquisitionError):
                acquire.ensure_cached(item, cache, unavailable)
            self.assertFalse((cache / item.filename).exists())

    def test_bad_download_sha_is_never_published(self):
        item = fixture("wrong.bin")
        with tempfile.TemporaryDirectory() as tmp:
            cache = Path(tmp)
            with self.assertRaises(acquire.IntegrityError):
                acquire.ensure_cached(item, cache, lambda _: b"tampered")
            self.assertFalse((cache / item.filename).exists())

    def test_corrupted_cache_is_not_reused_or_silently_replaced(self):
        item = fixture("cache.bin")
        with tempfile.TemporaryDirectory() as tmp:
            cache = Path(tmp)
            (cache / item.filename).write_bytes(b"poison")
            with self.assertRaises(acquire.IntegrityError):
                acquire.ensure_cached(item, cache, lambda _: self.fail("should not download after cache corruption"))

    def test_verified_cache_reuse(self):
        item = fixture("cached.bin")
        with tempfile.TemporaryDirectory() as tmp:
            cache = Path(tmp)
            first = acquire.ensure_cached(item, cache, lambda _: b"verified-fixture")
            self.assertEqual(first.read_bytes(), b"verified-fixture")
            again = acquire.ensure_cached(item, cache, lambda _: self.fail("should reuse verified cache"))
            self.assertEqual(again, first)

    def test_503_retries_are_bounded(self):
        item = fixture("down.bin")
        calls = []
        def offline(*args, **kwargs):
            calls.append(1)
            raise urllib.error.HTTPError(item.url, 503, "Service Unavailable", {}, None)
        with mock.patch.object(acquire.urllib.request, "urlopen", side_effect=offline):
            with mock.patch.object(acquire.time, "sleep") as pause:
                with self.assertRaises(acquire.AcquisitionError):
                    acquire.download_verified(item)
        self.assertEqual(len(calls), 4)
        self.assertEqual(pause.call_count, 3)

    def test_wrong_download_bytes_do_not_trigger_mirror_fallback(self):
        item = fixture("wrong-download.bin")
        with mock.patch.object(acquire, "request_bytes", return_value=b"malicious-bytes") as fetch:
            with self.assertRaises(acquire.IntegrityError):
                acquire.download_verified(item)
            self.assertEqual(fetch.call_count, 1)

    def test_release_api_metadata_rejects_wrong_asset_identity(self):
        item = acquire.WINDOWS[0]
        metadata = release_fixture(item, download_url="https://github.com/unapproved/repo/releases/download/changed/nsis-3.11.zip")
        with mock.patch.object(acquire, "request_bytes", return_value=json.dumps(metadata).encode()):
            with self.assertRaisesRegex(acquire.IntegrityError, "identity mismatch"):
                acquire.asset_api_url(item)

    def test_release_api_metadata_rejects_wrong_published_digest(self):
        item = acquire.WINDOWS[0]
        metadata = release_fixture(item, digest="sha256:" + "0" * 64)
        with mock.patch.object(acquire, "request_bytes", return_value=json.dumps(metadata).encode()):
            with self.assertRaisesRegex(acquire.IntegrityError, "different SHA-256"):
                acquire.asset_api_url(item)

    def test_release_api_metadata_rejects_duplicate_asset(self):
        item = acquire.WINDOWS[0]
        metadata = release_fixture(item)
        metadata["assets"].append(metadata["assets"][0])
        with mock.patch.object(acquire, "request_bytes", return_value=json.dumps(metadata).encode()):
            with self.assertRaises(acquire.IntegrityError):
                acquire.asset_api_url(item)

    def test_official_api_asset_fallback_requires_same_pinned_sha(self):
        # Canonical browser asset temporarily returns 504. The official GitHub
        # API endpoint for that exact asset ID serves identical verified bytes.
        payload = b"known-pinned-official-tool"
        item = acquire.Artifact(
            "nsis-3.11.zip",
            "https://github.com/tauri-apps/binary-releases/releases/download/nsis-3.11/nsis-3.11.zip",
            hashlib.sha256(payload).hexdigest(),
        )
        metadata = release_fixture(item, asset_id=171)
        seen = []

        def remote(url, *, binary=True):
            seen.append((url, binary))
            if not binary:
                return json.dumps(metadata).encode()
            if url == item.url:
                raise urllib.error.HTTPError(item.url, 504, "Gateway Timeout", {}, None)
            self.assertEqual(
                url, "https://api.github.com/repos/tauri-apps/binary-releases/releases/assets/171"
            )
            return payload

        with mock.patch.object(acquire, "request_bytes", side_effect=remote):
            with mock.patch.object(acquire.time, "sleep") as pause:
                self.assertEqual(acquire.download_verified(item), payload)
        self.assertEqual(len(seen), 3)
        pause.assert_not_called()

    def test_official_api_fallback_rejects_corrupt_bytes(self):
        payload = b"expected"
        item = acquire.Artifact(
            "nsis-3.11.zip",
            "https://github.com/tauri-apps/binary-releases/releases/download/nsis-3.11/nsis-3.11.zip",
            hashlib.sha256(payload).hexdigest(),
        )
        metadata = release_fixture(item)

        def remote(url, *, binary=True):
            if not binary:
                return json.dumps(metadata).encode()
            if url == item.url:
                raise urllib.error.HTTPError(item.url, 504, "Gateway Timeout", {}, None)
            return b"untrusted substitution"

        with mock.patch.object(acquire, "request_bytes", side_effect=remote):
            with mock.patch.object(acquire.time, "sleep") as pause:
                with self.assertRaises(acquire.IntegrityError):
                    acquire.download_verified(item)
        pause.assert_not_called()

    def test_zip_traversal_is_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / "archive.zip"
            with zipfile.ZipFile(source, "w") as zipfile_out:
                zipfile_out.writestr("nsis-3.11/legit.txt", b"ok")
                zipfile_out.writestr("nsis-3.11/../../outside.dll", b"pwn")
            with self.assertRaises(acquire.IntegrityError):
                acquire.safe_unzip(source, root / "extract")
            self.assertFalse((root / "outside.dll").exists())

    def test_wrong_nsis_zip_structure_is_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / "archive.zip"
            with zipfile.ZipFile(source, "w") as zipfile_out:
                zipfile_out.writestr("other-version/makensis.exe", b"not nsis")
            with self.assertRaises(acquire.IntegrityError):
                acquire.safe_unzip(source, root / "extract")


if __name__ == "__main__":
    unittest.main()
