"""Offline negative controls for the CI artifact acquisition gate."""
import hashlib
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
