#!/usr/bin/env python3
"""Bound redacted native/player/transport evidence to exact reviewed file bytes.

This is an integrity/availability guard, NOT an image-content recognizer,
remote-machine attestation, or replacement for independent human review.
"""
from __future__ import annotations

import hashlib
import json
from pathlib import Path, PurePosixPath
import re

# Every required role must produce a different actual recording. A single
# reused screenshot/video cannot stand in for multiple players or phases.
REQUIRED = frozenset({
    ("native_install", role) for role in ("alice", "bob", "carol")
} | {
    ("client_join", role) for role in ("alice", "bob", "carol")
} | {
    ("marker_before", role) for role in ("alice", "bob", "carol")
} | {
    ("marker_after", role) for role in ("alice", "bob", "carol")
} | {
    ("signed_invite", role) for role in ("alice", "bob", "carol")
} | {
    ("hosting_budget", role) for role in ("bob", "carol")
} | {
    ("eula_consent", "alice"),
    ("transport_direct", "bob"),
    ("transport_relay", "bob"),
    ("disconnect_reconnect", "bob"),
    ("alice_crash", "alice"),
    ("alice_fenced", "alice"),
})

VALID_EXT = frozenset({".png", ".jpg", ".jpeg", ".webm", ".mp4"})
SHA256 = re.compile(r"^[0-9a-f]{64}$")
MAX_BYTES = 100 * 1024 * 1024
MIN_BYTES = 64


class EvidenceError(ValueError):
    pass


def require(condition, message):
    if not condition:
        raise EvidenceError(message)


def hash_file(path):
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1048576), b""):
            h.update(chunk)
    return h.hexdigest()


def media_magic_valid(file):
    with file.open("rb") as stream:
        header = stream.read(16)
    suffix = file.suffix.lower()
    if suffix == ".png":
        return header.startswith(b"\x89PNG\r\n\x1a\n")
    if suffix in (".jpg", ".jpeg"):
        return header.startswith(b"\xff\xd8\xff")
    if suffix == ".webm":
        return header.startswith(b"\x1a\x45\xdf\xa3")
    if suffix == ".mp4":
        return len(header) >= 12 and header[4:8] == b"ftyp"
    return False


def verify_review_artifacts(review, output):
    root = Path(output).resolve(strict=True)
    evidence_root = root / "evidence"
    require(evidence_root.is_dir() and not evidence_root.is_symlink(),
            "redacted_evidence_directory_missing")
    records = review.get("artifacts")
    require(isinstance(records, list) and len(records) == len(REQUIRED),
            "required_real_player_artifact_count_missing")
    seen = set()
    fingerprints = set()
    manifest = []

    for record in records:
        require(isinstance(record, dict), "invalid_evidence_record")
        kind, role = record.get("kind"), record.get("role")
        pair = (kind, role)
        require(pair in REQUIRED and pair not in seen,
                "missing_duplicate_or_wrong_role_artifact")
        seen.add(pair)
        name = record.get("file")
        require(isinstance(name, str) and len(name) < 256 and "\\" not in name,
                "unsafe_evidence_path")
        rel = PurePosixPath(name)
        require(not rel.is_absolute() and len(rel.parts) >= 3 and rel.parts[0] == "evidence"
                and all(part not in ("", ".", "..") for part in rel.parts),
                "unsafe_evidence_path")
        path = root.joinpath(*rel.parts)
        require(path.suffix.lower() in VALID_EXT and path.is_file() and not path.is_symlink(),
                "missing_or_invalid_real_media")
        require(path.resolve(strict=True).is_relative_to(evidence_root.resolve(strict=True)),
                "evidence_outside_private_directory")
        size = path.stat().st_size
        require(MIN_BYTES <= size <= MAX_BYTES, "evidence_media_size_out_of_bounds")
        require(media_magic_valid(path), "not_real_media_magic")
        supplied = record.get("sha256")
        require(isinstance(supplied, str) and SHA256.fullmatch(supplied),
                "invalid_evidence_sha")
        observed = hash_file(path)
        require(observed == supplied, "tampered_reviewed_media")
        require(observed not in fingerprints, "reused_media_across_roles_or_stages")
        fingerprints.add(observed)
        # Only hash data and role labels are emitted; no private file paths.
        manifest.append({"kind": kind, "role": role, "sha256": observed, "size": size})

    require(seen == REQUIRED, "real_player_artifacts_incomplete")
    raw = json.dumps(sorted(manifest, key=lambda x: (x["kind"], x["role"])),
                     sort_keys=True, separators=(",", ":")).encode()
    return hashlib.sha256(raw).hexdigest()
