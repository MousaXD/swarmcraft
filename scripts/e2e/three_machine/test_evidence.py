#!/usr/bin/env python3
"""Only tests evidence-file integrity. Synthetic PNGs cannot certify gameplay."""
from __future__ import annotations

import hashlib
from pathlib import Path
import tempfile
import unittest

import evidence


def tiny_png_payload(label):
    """Valid tiny 1x1 PNG with per-record ancillary metadata."""
    import struct
    import zlib
    def chunk(kind, raw):
        return (struct.pack("!I", len(raw)) + kind + raw +
                struct.pack("!I", zlib.crc32(kind + raw) & 0xffffffff))
    rgba = b"\0" + b"\x11\x22\x33\xff"
    return (b"\x89PNG\r\n\x1a\n" +
            chunk(b"IHDR", struct.pack("!IIBBBBB", 1, 1, 8, 6, 0, 0, 0)) +
            chunk(b"tEXt", b"synthetic-not-real-evidence\x00" + label.encode()) +
            chunk(b"IDAT", zlib.compress(rgba)) + chunk(b"IEND", b""))


def fixtures(root):
    records = []
    for kind, role in sorted(evidence.REQUIRED):
        file = root / "evidence" / kind / (role + ".png")
        file.parent.mkdir(parents=True, exist_ok=True)
        file.write_bytes(tiny_png_payload(kind + role))
        records.append({"kind": kind, "role": role,
                        "file": file.relative_to(root).as_posix(),
                        "sha256": hashlib.sha256(file.read_bytes()).hexdigest()})
    return {"artifacts": records}


class TestMediaIntegrity(unittest.TestCase):
    def test_all_distinct_checksum_matched_media_can_make_manifest(self):
        with tempfile.TemporaryDirectory() as temp:
            folder = Path(temp)
            review = fixtures(folder)
            self.assertEqual(len(evidence.verify_review_artifacts(review, folder)), 64)

    def test_missing_real_player_media_rejected(self):
        with tempfile.TemporaryDirectory() as temp:
            folder = Path(temp)
            review = fixtures(folder)
            review["artifacts"].pop()
            with self.assertRaisesRegex(evidence.EvidenceError, "count_missing"):
                evidence.verify_review_artifacts(review, folder)

    def test_reused_artifact_rejected(self):
        with tempfile.TemporaryDirectory() as temp:
            folder = Path(temp)
            review = fixtures(folder)
            first = review["artifacts"][0]
            second = review["artifacts"][1]
            source = folder / first["file"]
            target = folder / second["file"]
            target.write_bytes(source.read_bytes())
            second["sha256"] = first["sha256"]
            with self.assertRaisesRegex(evidence.EvidenceError, "reused_media"):
                evidence.verify_review_artifacts(review, folder)

    def test_corrupt_or_swapped_screenshot_rejected(self):
        with tempfile.TemporaryDirectory() as temp:
            folder = Path(temp)
            review = fixtures(folder)
            path = folder / review["artifacts"][0]["file"]
            path.write_bytes(tiny_png_payload("different"))
            with self.assertRaisesRegex(evidence.EvidenceError, "tampered_reviewed_media"):
                evidence.verify_review_artifacts(review, folder)

    def test_escape_outside_private_evidence_rejected(self):
        with tempfile.TemporaryDirectory() as temp:
            folder = Path(temp)
            review = fixtures(folder)
            review["artifacts"][0]["file"] = "evidence/../../outside.png"
            with self.assertRaisesRegex(evidence.EvidenceError, "unsafe_evidence_path"):
                evidence.verify_review_artifacts(review, folder)

    def test_fake_bytes_with_png_extension_rejected(self):
        with tempfile.TemporaryDirectory() as temp:
            folder = Path(temp)
            review = fixtures(folder)
            record = review["artifacts"][0]
            file = folder / record["file"]
            file.write_bytes(b"NOT A PNG" + b"\0" * 128)
            record["sha256"] = hashlib.sha256(file.read_bytes()).hexdigest()
            with self.assertRaisesRegex(evidence.EvidenceError, "not_real_media_magic"):
                evidence.verify_review_artifacts(review, folder)

    def test_symlink_to_another_directory_rejected(self):
        with tempfile.TemporaryDirectory() as temp:
            folder = Path(temp)
            review = fixtures(folder)
            file = folder / review["artifacts"][0]["file"]
            another = folder / "outside.png"
            another.write_bytes(file.read_bytes())
            file.unlink()
            file.symlink_to(another)
            with self.assertRaisesRegex(evidence.EvidenceError, "missing_or_invalid_real_media"):
                evidence.verify_review_artifacts(review, folder)


if __name__ == "__main__":
    unittest.main()
