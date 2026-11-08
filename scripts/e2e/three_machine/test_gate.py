#!/usr/bin/env python3
"""Negative-only unit checks. Synthetic records are NEVER real E2E evidence."""
from __future__ import annotations
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import run as gate

SHA = "a" * 64
COMMIT = "b" * 40
BOB = "bob-fp"
ALICE = "alice-fp"
CAROL = "carol-fp"


def config():
    return {
        "schema_version": 1, "commit": COMMIT,
        "package_sha256": SHA, "fingerprint_salt": SHA,
        "nodes": {
            role: {
                "ssh": "test-" + role, "package": "/tmp/swarmcraft.deb",
                "binary": "/usr/bin/swarmcraft-desktop",
                "cli": "/usr/bin/swarmcraft", "data_dir": "/tmp/e2e/" + role,
                "network_class": "home_nat",
            } for role in gate.ROLES
        },
    }


def node(role, stage):
    authority = ALICE if stage not in ("bob_recovered", "alice_returned") else BOB
    return {
        "world_fingerprint": "world-fp",
        "identity_fingerprint": {"alice": ALICE, "bob": BOB, "carol": CAROL}[role],
        "authority_fingerprint": authority,
        "successor_fingerprint": BOB if role == "alice" else None,
        "authorized_peers": 3,
        "snapshot_number": 3,
        "snapshot_epoch": 1 if authority == ALICE else 2,
        "snapshot_hash": "1" * 64 if authority == ALICE else "2" * 64,
        "state_root": "3" * 64,
        "snapshot_verify_passed": True,
        "canonical": True,
        "epoch": 1 if authority == ALICE else 2,
        "fencing_token": 1 if authority == ALICE else 2,
        "phase": "ready", "runtime_ready": role == ("alice" if authority == ALICE else "bob"),
        "host_readiness_state": "safe" if role == "alice" else "unknown",
        "safe_to_shutdown": role == "alice",
        "desktop_process_observed": True,
        "minecraft_client_process_observed": True,
        "minecraft_server_process_observed": role == ("alice" if authority == ALICE else "bob"),
    }


def fixtures(directory):
    c = config()
    gate.save(directory / "preflight.json", {
        "commit": COMMIT, "package_sha256": SHA,
        "nodes": {
            role: {"machine_fingerprint": "host-" + role,
                   "egress_fingerprint": "wan-" + role} for role in gate.ROLES
        },
    })
    for stage, roles in gate.STAGES.items():
        gate.save(directory / (stage + ".json"), {
            "stage": stage, "commit": COMMIT, "package_sha256": SHA,
            "nodes": {role: node(role, stage) for role in roles},
        })
    gate.save(directory / "crash.json", {
        "commit": COMMIT,
        "receipt": {"signal": "SIGKILL", "process_gone": True, "role": "alice",
                    "crashed_components": ["desktop", "daemon", "server"]},
    })
    return c


class TestFailClosed(unittest.TestCase):
    def test_disallow_shell_target_injection(self):
        with tempfile.TemporaryDirectory() as temp:
            c = config()
            c["nodes"]["alice"]["ssh"] = "test;touch /tmp/injected"
            path = Path(temp) / "c.json"
            gate.save(path, c)
            with self.assertRaisesRegex(gate.Blocked, "invalid_ssh_target_alice"):
                gate.config(path)

    def test_disallow_missing_expected_commit(self):
        with tempfile.TemporaryDirectory() as temp:
            c = config()
            c["commit"] = ""
            path = Path(temp) / "c.json"
            gate.save(path, c)
            with self.assertRaisesRegex(gate.Blocked, "expected_commit_missing"):
                gate.config(path)

    def test_preflight_rejects_colocated_networks(self):
        with tempfile.TemporaryDirectory() as temp:
            c = config()
            with patch.object(gate, "remote", side_effect=lambda cfg, role, op: {
                "machine_fingerprint": role,
                "egress_fingerprint": "shared-nat",
                "package_sha256": SHA,
                "package_version": "0.5.0",
                "app_installed": True,
                "cli_installed": True,
                "os": "linux",
            }):
                with self.assertRaisesRegex(gate.Blocked, "external_networks_not_independent"):
                    gate.preflight(c, Path(temp))

    def test_synthetic_state_cannot_pass_without_real_player_review(self):
        with tempfile.TemporaryDirectory() as temp:
            out = Path(temp)
            c = fixtures(out)
            with self.assertRaisesRegex(gate.Blocked, "independent_player_evidence_review_missing"):
                gate.verify(c, out)

    def test_rejects_stale_authority_after_recovery(self):
        with tempfile.TemporaryDirectory() as temp:
            out = Path(temp)
            c = fixtures(out)
            recovered = gate.load_json(out / "alice_returned.json")
            recovered["nodes"]["alice"]["authority_fingerprint"] = ALICE
            gate.save(out / "alice_returned.json", recovered)
            with self.assertRaisesRegex(gate.Blocked, "stale_alice_or_split_brain"):
                gate.verify(c, out)

    def test_rejects_unverified_snapshot(self):
        with tempfile.TemporaryDirectory() as temp:
            out = Path(temp)
            c = fixtures(out)
            check = gate.load_json(out / "checkpoint.json")
            check["nodes"]["carol"]["snapshot_verify_passed"] = False
            gate.save(out / "checkpoint.json", check)
            with self.assertRaisesRegex(gate.Blocked, "unsigned_or_conflicted"):
                gate.verify(c, out)

    def test_rejects_only_two_world_members(self):
        with tempfile.TemporaryDirectory() as temp:
            out = Path(temp)
            c = fixtures(out)
            check = gate.load_json(out / "checkpoint.json")
            check["nodes"]["carol"]["authorized_peers"] = 2
            gate.save(out / "checkpoint.json", check)
            with self.assertRaisesRegex(gate.Blocked, "signed_three_member_quorum_missing"):
                gate.verify(c, out)

    def test_rejects_only_java_crash(self):
        with tempfile.TemporaryDirectory() as temp:
            out = Path(temp)
            c = fixtures(out)
            record = gate.load_json(out / "crash.json")
            record["receipt"]["crashed_components"] = ["server"]
            gate.save(out / "crash.json", record)
            with self.assertRaisesRegex(gate.Blocked, "alice_full_authority_process_crash_missing"):
                gate.verify(c, out)

    def test_rejects_real_client_not_observed(self):
        with tempfile.TemporaryDirectory() as temp:
            out = Path(temp)
            c = fixtures(out)
            check = gate.load_json(out / "checkpoint.json")
            check["nodes"]["bob"]["minecraft_client_process_observed"] = False
            gate.save(out / "checkpoint.json", check)
            with self.assertRaisesRegex(gate.Blocked, "installed_gui_or_real_client_not_observed"):
                gate.verify(c, out)


if __name__ == "__main__":
    unittest.main()
