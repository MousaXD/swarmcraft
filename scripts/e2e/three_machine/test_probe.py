#!/usr/bin/env python3
"""Diagnostic redaction regression tests; no live network or simulated certification."""
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import node


class TestCanonicalCliIdentifiers(unittest.TestCase):
    WORLD = "ab" * 32
    PEER = "cd" * 32

    def test_cli_display_world_prefix_is_normalized(self):
        self.assertEqual(node.canonical_world_id("scworld:" + self.WORLD), self.WORLD)
        self.assertEqual(node.canonical_world_id(self.WORLD.upper()), self.WORLD)
        for bad in ("scpeer:" + self.WORLD, "scworld:../world", "scworld:" + "a" * 63, None, ""):
            self.assertIsNone(node.canonical_world_id(bad))

    def test_backend_peer_id_is_validated_and_canonical(self):
        self.assertEqual(node.canonical_peer_id("scpeer:" + self.PEER), "scpeer:" + self.PEER)
        self.assertEqual(node.canonical_peer_id(self.PEER.upper()), "scpeer:" + self.PEER)
        for bad in ("scworld:" + self.PEER, "secret", "", None):
            self.assertIsNone(node.canonical_peer_id(bad))


class TestSanitizedDiagnostics(unittest.TestCase):
    def test_does_not_publish_addresses_tokens_or_error_details(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "connectivity-diagnostics-v1.json"
            secret = "SCINVITE-SENSITIVE-SECRET"
            path.write_text(json.dumps({
                "state": "relay_connected",
                "nat_status": "private",
                "direct_connectivity": False,
                "relay_connectivity": True,
                "hole_punch": {"failed": secret},
                "selected_relay": "/ip4/203.0.113.10/tcp/4001/p2p/SENSITIVE",
                "observed_public_address": "203.0.113.20",
                "last_failure": secret,
                "recent_failures": [{"detail": secret, "address": "203.0.113.20"}],
                "ipv4_available": True,
                "ipv6_available": False,
            }))
            summary = node.connectivity_summary(temp)
            self.assertEqual(summary["state"], "relay_connected")
            self.assertEqual(summary["hole_punch"], "failed")
            redacted = json.dumps(summary)
            for forbidden in (secret, "203.0.113", "/p2p/", "last_failure"):
                self.assertNotIn(forbidden, redacted)

    def test_missing_diagnostics_is_unavailable_not_fake_direct(self):
        with tempfile.TemporaryDirectory() as temp:
            self.assertEqual(node.connectivity_summary(temp), {"available": False})


class TestCanonicalSnapshotHeadEvidence(unittest.TestCase):
    WORLD = "ab" * 32

    def test_reads_exact_swarm_storage_canonical_head_bytes(self):
        import hashlib
        with tempfile.TemporaryDirectory() as temp:
            head = (Path(temp) / "worlds" / self.WORLD / "metadata" /
                    "canonical-snapshot-head.postcard")
            head.parent.mkdir(parents=True)
            actual = b"\\x01" * 80
            head.write_bytes(actual)
            self.assertEqual(node.canonical_head_digest(temp, self.WORLD),
                             hashlib.sha256(actual).hexdigest())
            head.write_bytes(b"")
            self.assertIsNone(node.canonical_head_digest(temp, self.WORLD))
            head.write_bytes(b"a" * 20000)
            self.assertIsNone(node.canonical_head_digest(temp, self.WORLD))

    def test_canonical_head_symlink_is_never_accepted(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            head = root / "worlds" / self.WORLD / "metadata" / "canonical-snapshot-head.postcard"
            head.parent.mkdir(parents=True)
            secret = root / "outside.postcard"
            secret.write_bytes(b"b" * 80)
            head.symlink_to(secret)
            self.assertIsNone(node.canonical_head_digest(temp, self.WORLD))


class TestRealHostBudgetEvidence(unittest.TestCase):
    WORLD = "ab" * 32
    PEER = "scpeer:" + "cd" * 32

    def test_local_opt_in_record_binds_world_and_device(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "control" / self.WORLD / "hosting-consent.json"
            path.parent.mkdir(parents=True)
            record = {
                "schema_version": 1, "world_id": list(bytes.fromhex(self.WORLD)),
                "peer_id": list(bytes.fromhex(self.PEER.removeprefix("scpeer:"))),
                "enabled": True, "heap_mib": 4096, "revision": 123,
            }
            path.write_text(json.dumps(record))
            data = node.hosting_consent(temp, self.WORLD, self.PEER)
            self.assertTrue(data["available"])
            self.assertTrue(data["identity_bound"])
            self.assertEqual(data["heap_mib"], 4096)
            self.assertFalse(node.hosting_consent(temp, self.WORLD, "scpeer:" + "ef" * 32)["available"])

    def test_zero_budget_is_player_only(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "control" / self.WORLD / "hosting-consent.json"
            path.parent.mkdir(parents=True)
            record = {
                "schema_version": 1, "world_id": list(bytes.fromhex(self.WORLD)),
                "peer_id": list(bytes.fromhex(self.PEER.removeprefix("scpeer:"))),
                "enabled": False, "heap_mib": 0, "revision": 123,
            }
            path.write_text(json.dumps(record))
            data = node.hosting_consent(temp, self.WORLD, self.PEER)
            self.assertEqual(data["heap_mib"], 0)
            self.assertFalse(data["enabled"])
            record["enabled"] = True
            path.write_text(json.dumps(record))
            self.assertFalse(node.hosting_consent(temp, self.WORLD, self.PEER)["available"])

    def test_version_values_are_bounded_and_cannot_contain_private_paths(self):
        self.assertEqual(node.safe_version("25.0.5+8"), "25.0.5+8")
        self.assertEqual(node.safe_version("0.19.3"), "0.19.3")
        for text in ("25\\\\secret", "../../secrets", "25;cat /etc/passwd", "", None):
            self.assertIsNone(node.safe_version(text))

    def test_java_version_comes_from_actual_process_executable(self):
        from types import SimpleNamespace
        with patch.object(node, "isolated_process", return_value=True):
            with patch.object(Path, "resolve", return_value=Path("/usr/lib/jvm/java")):
                with patch.object(node, "run", return_value=SimpleNamespace(
                    returncode=0, stderr='openjdk version "25.0.1" 2026', stdout=""
                )) as runner:
                    version = node.java_runtime_version(123, Path("/tmp/e2e"), self.WORLD,
                                                        Path("/usr/bin/gui"), Path("/usr/bin/swarmcraft"))
                    self.assertEqual(version, "25.0.1")
                    runner.assert_called_once_with(["/usr/lib/jvm/java", "-version"], timeout=8)

    def test_java_xmx_reads_real_argument_boundary_not_substrings(self):
        with patch.object(node, "isolated_process", return_value=True):
            with patch.object(Path, "read_bytes", return_value=b"java\0-Xmx4096m\0-jar\0fabric-server.jar\0"):
                value = node.runtime_heap_mib(123, Path("/private/test"), self.WORLD,
                                              Path("/usr/bin/gui"), Path("/usr/bin/swarmcraft"))
                self.assertEqual(value, 4096)
            # A localized/non-ASCII world path must not hide a genuine
            # ASCII heap argument elsewhere in the Java command line.
            with patch.object(Path, "read_bytes", return_value=b"java\0\xff\xfe\0-Xmx4096m\0-jar\0fabric.jar"):
                value = node.runtime_heap_mib(123, Path("/private/test"), self.WORLD,
                                              Path("/usr/bin/gui"), Path("/usr/bin/swarmcraft"))
                self.assertEqual(value, 4096)
            with patch.object(Path, "read_bytes", return_value=b"java\0-jar\0-Xmx4096m.fake"):
                value = node.runtime_heap_mib(123, Path("/private/test"), self.WORLD,
                                              Path("/usr/bin/gui"), Path("/usr/bin/swarmcraft"))
                self.assertIsNone(value)


if __name__ == "__main__":
    unittest.main()
