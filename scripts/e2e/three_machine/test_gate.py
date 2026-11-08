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
        "authorized_peers": {"alice_created": 1, "bob_joined": 2}.get(stage, 3),
        "snapshot_number": 3,
        "snapshot_epoch": 1 if authority == ALICE else 2,
        "snapshot_hash": "1" * 64 if authority == ALICE else "2" * 64,
        "state_root": "3" * 64,
        "snapshot_verify_passed": True,
        "canonical": stage != "alice_created",
        "safety_level": "SoloUnreplicated" if stage == "alice_created" else "Canonical",
        "epoch": 1 if authority == ALICE else 2,
        "fencing_token": 1 if authority == ALICE else 2,
        "phase": "ready", "runtime_ready": role == ("alice" if authority == ALICE else "bob"),
        "host_readiness_state": "safe" if role == "alice" else "unknown",
        "safe_to_shutdown": role == "alice",
        "desktop_process_observed": True,
        "minecraft_client_process_observed": True,
        "minecraft_server_process_observed": role == ("alice" if authority == ALICE else "bob"),
        "hosting_consent": {
            "available": True, "enabled": role != "carol",
            "heap_mib": {"alice": 2048, "bob": 4096, "carol": 0}[role],
            "revision": 123, "world_bound": True, "identity_bound": True,
        },
        "minecraft_server_heap_mib": (
            {"alice": 2048, "bob": 4096}.get(role)
            if role == ("alice" if authority == ALICE else "bob") else None
        ),
        "connectivity_diagnostics": {
            "available": True,
            "state": "relay_connected" if stage == "transport_relay" else "direct_reachable",
            "direct_connectivity": stage != "transport_relay",
            "relay_connectivity": stage == "transport_relay",
            "hole_punch": "idle",
        },
    }


def fixtures(directory):
    c = config()
    gate.save(directory / "preflight.json", {
        "commit": COMMIT, "package_sha256": SHA,
        "observed_utc": "2026-11-08T00:00:00+00:00",
        "nodes": {
            role: {"machine_fingerprint": "host-" + role,
                   "egress_fingerprint": "wan-" + role} for role in gate.ROLES
        },
    })
    for index, (stage, roles) in enumerate(gate.STAGES.items(), 1):
        gate.save(directory / (stage + ".json"), {
            "stage": stage, "commit": COMMIT, "package_sha256": SHA,
            "observed_utc": f"2026-11-08T00:{index:02}:00+00:00",
            "nodes": {role: node(role, stage) for role in roles},
        })
    gate.save(directory / "crash.json", {
        "commit": COMMIT,
        "observed_utc": "2026-11-08T00:07:30+00:00",
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

    def test_disallow_ssh_option_as_destination(self):
        with tempfile.TemporaryDirectory() as temp:
            c = config()
            c["nodes"]["alice"]["ssh"] = "-oStrictHostKeyChecking=no"
            path = Path(temp) / "config.json"
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
                "all_required_sidecars_installed": True,
                "os": "linux",
            }):
                with self.assertRaisesRegex(gate.Blocked, "external_networks_not_independent"):
                    gate.preflight(c, Path(temp))

    def test_preflight_rejects_missing_sidecar(self):
        with tempfile.TemporaryDirectory() as temp:
            c = config()
            def probe(cfg, role, operation):
                return {
                    "machine_fingerprint": role,
                    "egress_fingerprint": role,
                    "package_sha256": SHA,
                    "package_version": "0.5.0",
                    "app_installed": True,
                    "cli_installed": True,
                    "all_required_sidecars_installed": role != "carol",
                    "os": "linux",
                }
            with patch.object(gate, "remote", side_effect=probe):
                with self.assertRaisesRegex(gate.Blocked, "installed_linux_package_required"):
                    gate.preflight(c, Path(temp))

    def test_signed_solo_bootstrap_is_not_canonical_quorum(self):
        with tempfile.TemporaryDirectory() as temp:
            out = Path(temp)
            c = fixtures(out)
            self.assertFalse(gate.load_json(out / "alice_created.json")["nodes"]["alice"]["canonical"])
            with self.assertRaisesRegex(gate.Blocked, "independent_player_evidence_review_missing"):
                gate.verify(c, out)

    def test_rejects_out_of_order_observations(self):
        with tempfile.TemporaryDirectory() as temp:
            out = Path(temp)
            c = fixtures(out)
            record = gate.load_json(out / "bob_joined.json")
            record["observed_utc"] = "2026-11-07T12:00:00+00:00"
            gate.save(out / "bob_joined.json", record)
            with self.assertRaisesRegex(gate.Blocked, "stale_or_out_of_order_stage_bob_joined"):
                gate.verify(c, out)

    def test_rejects_conflicted_bootstrap(self):
        with tempfile.TemporaryDirectory() as temp:
            out = Path(temp)
            c = fixtures(out)
            record = gate.load_json(out / "alice_created.json")
            record["nodes"]["alice"]["safety_level"] = "Conflict"
            gate.save(out / "alice_created.json", record)
            with self.assertRaisesRegex(gate.Blocked, "bootstrap_state_conflicted"):
                gate.verify(c, out)

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

    def test_rejects_configured_but_inactive_relay(self):
        with tempfile.TemporaryDirectory() as temp:
            out = Path(temp)
            c = fixtures(out)
            check = gate.load_json(out / "transport_relay.json")
            for node in check["nodes"].values():
                node["connectivity_diagnostics"]["relay_connectivity"] = False
            gate.save(out / "transport_relay.json", check)
            with self.assertRaisesRegex(gate.Blocked, "active_relay_fallback_not_observed"):
                gate.verify(c, out)

    def test_rejects_missing_direct_path(self):
        with tempfile.TemporaryDirectory() as temp:
            out = Path(temp)
            c = fixtures(out)
            check = gate.load_json(out / "transport_direct.json")
            for node in check["nodes"].values():
                node["connectivity_diagnostics"]["direct_connectivity"] = False
            gate.save(out / "transport_direct.json", check)
            with self.assertRaisesRegex(gate.Blocked, "real_direct_or_hole_punched_path_not_observed"):
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

    def test_rejects_bob_without_explicit_opt_in(self):
        with tempfile.TemporaryDirectory() as temp:
            out = Path(temp)
            c = fixtures(out)
            check = gate.load_json(out / "checkpoint.json")
            check["nodes"]["bob"]["hosting_consent"]["enabled"] = False
            gate.save(out / "checkpoint.json", check)
            with self.assertRaisesRegex(gate.Blocked, "alice_or_bob_host_opt_in_missing"):
                gate.verify(c, out)

    def test_rejects_carol_host_allocation(self):
        with tempfile.TemporaryDirectory() as temp:
            out = Path(temp)
            c = fixtures(out)
            check = gate.load_json(out / "checkpoint.json")
            check["nodes"]["carol"]["hosting_consent"]["enabled"] = True
            check["nodes"]["carol"]["hosting_consent"]["heap_mib"] = 512
            gate.save(out / "checkpoint.json", check)
            with self.assertRaisesRegex(gate.Blocked, "carol_player_only_zero_budget_missing"):
                gate.verify(c, out)

    def test_rejects_bob_java_without_configured_heap(self):
        with tempfile.TemporaryDirectory() as temp:
            out = Path(temp)
            c = fixtures(out)
            check = gate.load_json(out / "bob_recovered.json")
            check["nodes"]["bob"]["minecraft_server_heap_mib"] = None
            gate.save(out / "bob_recovered.json", check)
            with self.assertRaisesRegex(gate.Blocked, "bob_real_java_heap_exceeds_or_ignores_consent"):
                gate.verify(c, out)

    def test_rejects_carol_host_on_return(self):
        with tempfile.TemporaryDirectory() as temp:
            out = Path(temp)
            c = fixtures(out)
            check = gate.load_json(out / "alice_returned.json")
            check["nodes"]["carol"]["hosting_consent"]["enabled"] = True
            check["nodes"]["carol"]["hosting_consent"]["heap_mib"] = 1024
            gate.save(out / "alice_returned.json", check)
            with self.assertRaisesRegex(gate.Blocked, "carol_zero_budget_or_player_role_violated"):
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
