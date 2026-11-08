#!/usr/bin/env python3
"""Three independently installed SwarmCraft machines: orchestration + fail-closed evidence.

This runner never creates worlds, joins players, accepts the EULA, supplies fake
runtime data, or claims a real Minecraft join from a CLI-only observation.
"""
from __future__ import annotations
import argparse
import hashlib
import hashlib
import json
from pathlib import Path
import re
import shlex
import subprocess
import sys
from datetime import datetime, timezone
from urllib.parse import urlparse

from evidence import EvidenceError, verify_review_artifacts

ROLES = ("alice", "bob", "carol")
STAGES = {
    "alice_created": ("alice",),
    "bob_joined": ("alice", "bob"),
    "carol_joined": ROLES,
    "transport_direct": ROLES,
    "transport_relay": ROLES,
    "transport_disrupted": ROLES,
    "checkpoint": ROLES,
    "alice_crashed": ("bob", "carol"),
    "bob_recovered": ("bob", "carol"),
    "alice_returned": ROLES,
}
SHA = re.compile(r"^[0-9a-fA-F]{64}$")
COMMIT = re.compile(r"^[0-9a-fA-F]{40}$")
SSH_TARGET = re.compile(r"^[a-zA-Z0-9][a-zA-Z0-9_.@:-]*$")
HERE = Path(__file__).resolve().parent


class Blocked(Exception):
    pass


def require(condition, reason):
    if not condition:
        raise Blocked(reason)


def load_json(path):
    return json.loads(Path(path).read_text(encoding="utf-8"))


def save(path, data):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(data, sort_keys=True, indent=2) + "\n", encoding="utf-8")


def config(path):
    c = load_json(path)
    require(c.get("schema_version") == 1, "unsupported_config_schema")
    require(COMMIT.fullmatch(c.get("commit", "")), "expected_commit_missing")
    require(SHA.fullmatch(c.get("package_sha256", "")), "expected_package_hash_missing")
    require(SHA.fullmatch(c.get("fingerprint_salt", "")), "private_fingerprint_salt_missing")
    nodes = c.get("nodes", {})
    require(set(nodes) == set(ROLES), "exactly_three_roles_required")
    for role in ROLES:
        n = nodes[role]
        require(SSH_TARGET.fullmatch(n.get("ssh", "")), "invalid_ssh_target_" + role)
        for key in ("package", "binary", "cli", "data_dir"):
            require(isinstance(n.get(key), str) and n[key].startswith("/"), "absolute_path_required_" + role + "_" + key)
        require(n.get("network_class") in ("home_nat", "cgnat", "mobile", "public_wan", "other_wan"),
                "network_class_missing_" + role)
    return c


def remote(c, role, operation, **kwargs):
    node = c["nodes"][role]
    arguments = [
        operation, "--salt", c["fingerprint_salt"],
        "--package", node["package"], "--binary", node["binary"],
        "--cli", node["cli"], "--data-dir", node["data_dir"],
    ]
    for name, value in kwargs.items():
        if value is not None:
            arguments.extend(("--" + name.replace("_", "-"), str(value)))
    command = "python3 - " + " ".join(shlex.quote(arg) for arg in arguments)
    ssh = ["ssh", "-o", "BatchMode=yes", "-o", "StrictHostKeyChecking=yes",
           "-o", "ConnectTimeout=12", node["ssh"], command]
    try:
        response = subprocess.run(
            ssh, input=(HERE / "node.py").read_bytes(), capture_output=True,
            timeout=100, check=False,
        )
    except (OSError, subprocess.TimeoutExpired) as exc:
        raise Blocked("ssh_unavailable_" + role) from exc
    # Never include SSH stderr: it may contain private network identifiers.
    require(response.returncode == 0, "remote_probe_failed_" + role)
    try:
        data = json.loads(response.stdout)
    except (ValueError, UnicodeError) as exc:
        raise Blocked("remote_response_invalid_" + role) from exc
    require(data.get("ok") is True and data.get("operation") == operation, "remote_probe_rejected_" + role)
    del data["ok"]
    del data["operation"]
    return data


def preflight(c, output):
    probes = {role: remote(c, role, "preflight") for role in ROLES}
    require(len({p["machine_fingerprint"] for p in probes.values()}) == 3, "machines_not_independent")
    require(len({p["egress_fingerprint"] for p in probes.values()}) == 3, "external_networks_not_independent")
    require(all(p["package_sha256"].lower() == c["package_sha256"].lower() for p in probes.values()),
            "package_sha_mismatch")
    require(len({p["package_version"] for p in probes.values()}) == 1, "package_version_mismatch")
    require(all(p["app_installed"] and p["cli_installed"] and p["all_required_sidecars_installed"]
                and p["os"] == "linux" for p in probes.values()),
            "installed_linux_package_required")
    save(output / "preflight.json", {
        "commit": c["commit"], "package_sha256": c["package_sha256"],
        "observed_utc": datetime.now(timezone.utc).isoformat(),
        "nodes": {role: {**probes[role], "network_class": c["nodes"][role]["network_class"]} for role in ROLES},
    })


def capture(c, output, stage, world, pids):
    require(stage in STAGES, "unknown_capture_stage")
    pid_data = load_json(pids) if pids else {}
    probes = {}
    for role in STAGES[stage]:
        local = pid_data.get(role, {})
        probes[role] = remote(
            c, role, "capture", world=world,
            desktop_pid=local.get("desktop"), client_pid=local.get("client"),
            server_pid=local.get("server"),
        )
    save(output / (stage + ".json"), {
        "stage": stage, "commit": c["commit"], "package_sha256": c["package_sha256"],
        "observed_utc": datetime.now(timezone.utc).isoformat(),
        "nodes": probes,
    })


def crash(c, output, world, desktop_pid, daemon_pid, server_pid):
    pids = (desktop_pid, daemon_pid, server_pid)
    require(all(pid and pid > 1 for pid in pids) and len(set(pids)) == 3, "three_crash_process_ids_required")
    receipt = remote(c, "alice", "crash", world=world, desktop_pid=desktop_pid,
                     daemon_pid=daemon_pid, server_pid=server_pid, confirm="CRASH-ALICE-AUTHORITY")
    require(receipt.get("process_gone") is True and receipt.get("signal") == "SIGKILL" and
            receipt.get("crashed_components") == ["desktop", "daemon", "server"], "three_component_crash_not_observed")
    save(output / "crash.json", {"commit": c["commit"], "receipt": receipt,
                                  "observed_utc": datetime.now(timezone.utc).isoformat()})


def timestamp(value, field):
    require(isinstance(value, str), "missing_timestamp_" + field)
    try:
        instant = datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError as exc:
        raise Blocked("invalid_timestamp_" + field) from exc
    require(instant.tzinfo is not None and instant.utcoffset() is not None,
            "timestamp_not_timezone_aware_" + field)
    return instant.astimezone(timezone.utc)


def observations_sha256(output):
    """Canonical binding to every recorded stage and crash receipt.

    Contains ONLY a digest; files stay private until a reviewer confirms
    redaction and source provenance. This binds review to the *recorded*
    observations, not to real-world authenticity.
    """
    names = ("preflight", *STAGES.keys(), "crash")
    records = []
    for name in names:
        value = load_json(Path(output) / (name + ".json"))
        # Canonical serialization gives a stable digest across JSON
        # formatting/whitespace changes; changing any stage data changes it.
        records.append({"stage": name, "record": value})
    wire = json.dumps(records, sort_keys=True, separators=(",", ":"),
                      ensure_ascii=True, allow_nan=False).encode("utf-8")
    return hashlib.sha256(wire).hexdigest()


def verify(c, output, review_file=None):
    pre = load_json(output / "preflight.json")
    require(pre["commit"] == c["commit"] and pre["package_sha256"] == c["package_sha256"],
            "preflight_wrong_candidate")
    require(len({x["machine_fingerprint"] for x in pre["nodes"].values()}) == 3, "duplicate_machine")
    require(len({x["egress_fingerprint"] for x in pre["nodes"].values()}) == 3, "duplicate_egress")
    previous_timestamp = timestamp(pre.get("observed_utc"), "preflight")
    stages = {}
    stage_times = {}
    for stage, roles in STAGES.items():
        s = load_json(output / (stage + ".json"))
        require(s["stage"] == stage and s["commit"] == c["commit"] and
                s["package_sha256"] == c["package_sha256"], "stage_candidate_mismatch_" + stage)
        require(set(s["nodes"]) == set(roles), "stage_missing_machine_" + stage)
        observed = timestamp(s.get("observed_utc"), stage)
        require(observed > previous_timestamp, "stale_or_out_of_order_stage_" + stage)
        previous_timestamp = observed
        stage_times[stage] = observed
        stages[stage] = s["nodes"]
        for role, node in s["nodes"].items():
            require(node["snapshot_verify_passed"], "unsigned_or_conflicted_" + stage + "_" + role)
            if stage in ("alice_created", "bob_joined", "carol_joined"):
                # Genuine first host can be solo until a signed multi-member
                # quorum exists. Conflicts/unknown never qualify here.
                require(node.get("safety_level") in ("Canonical", "SoloUnreplicated"),
                        "bootstrap_state_conflicted_" + stage + "_" + role)
            else:
                require(node["canonical"] and node.get("safety_level") == "Canonical",
                        "unsigned_or_conflicted_" + stage + "_" + role)
            require(node.get("snapshot_hash") and SHA.fullmatch(node["snapshot_hash"]) and node.get("state_root"),
                    "accepted_snapshot_digest_missing_" + stage + "_" + role)
            require(node["snapshot_number"] is not None and node["snapshot_number"] >= 1,
                    "snapshot_missing_" + stage + "_" + role)
            require(node["world_fingerprint"] == stages["alice_created"]["alice"]["world_fingerprint"],
                    "world_divergence_" + stage + "_" + role)
    require(stages["alice_created"]["alice"]["authorized_peers"] == 1,
            "fresh_signed_alice_world_not_solo")
    require(all(n["authorized_peers"] >= 2 for n in stages["bob_joined"].values()),
            "bob_membership_not_accepted")
    require(all(n["authorized_peers"] == 3 for n in stages["carol_joined"].values()),
            "carol_membership_not_accepted")
    require(stages["bob_joined"]["bob"]["minecraft_client_process_observed"],
            "bob_client_missing_after_join")
    require(stages["carol_joined"]["carol"]["minecraft_client_process_observed"],
            "carol_client_missing_after_join")
    # An unconsented third player can stay in quorum/storage but must never
    # turn into a host, even briefly during the intermediate network stages.
    for name in tuple(STAGES)[2:]:
        if "carol" not in stages[name]:
            continue
        carol = stages[name]["carol"]
        consent = carol.get("hosting_consent")
        require(isinstance(consent, dict) and consent.get("available") is True and
                consent.get("enabled") is False and consent.get("heap_mib") == 0 and
                not carol["minecraft_server_process_observed"],
                "carol_player_only_role_violated_" + name)
    # Observed runtime transport state must agree with the independent
    # reviewer. A configured relay, unverified NAT label or SSH reachability
    # cannot count as a selected libp2p transport.
    direct = [n.get("connectivity_diagnostics") for n in stages["transport_direct"].values()]
    relay = [n.get("connectivity_diagnostics") for n in stages["transport_relay"].values()]
    require(any(isinstance(d, dict) and d.get("available") is True and
                (d.get("direct_connectivity") is True or
                 (d.get("hole_punch") == "succeeded" and d.get("state") == "hole_punched"))
                for d in direct), "real_direct_or_hole_punched_path_not_observed")
    require(any(isinstance(d, dict) and d.get("available") is True and
                d.get("relay_connectivity") is True and
                d.get("state") in ("relay_connected", "relay_required")
                for d in relay), "active_relay_fallback_not_observed")
    p = stages["checkpoint"]
    a, b, d = p["alice"], p["bob"], p["carol"]
    require(len({v["identity_fingerprint"] for v in p.values()}) == 3, "duplicate_player_identity")
    require(all(x["authorized_peers"] == 3 for x in p.values()), "signed_three_member_quorum_missing")
    for key in ("minecraft_version", "fabric_loader_version"):
        values = {x.get(key) for x in p.values()}
        require(len(values) == 1 and None not in values, "canonical_runtime_version_mismatch_" + key)
    require(len({(x["epoch"], x["fencing_token"]) for x in p.values()}) == 1 and
            a["epoch"] is not None and a["fencing_token"] is not None,
            "checkpoint_generation_not_agreed")
    require(all(x["snapshot_hash"] and x["state_root"] for x in p.values()), "checkpoint_hash_missing")
    require(len({x["snapshot_hash"] for x in p.values()}) == 1 and
            len({x["state_root"] for x in p.values()}) == 1, "checkpoint_not_replicated")
    require(all(x["authority_fingerprint"] == a["identity_fingerprint"] for x in p.values()),
            "alice_not_canonical_authority")
    consent_a, consent_b, consent_c = (v.get("hosting_consent") for v in (a, b, d))
    require(all(isinstance(x, dict) and x.get("available") is True and
                x.get("identity_bound") is True and x.get("world_bound") is True
                for x in (consent_a, consent_b, consent_c)),
            "local_identity_bound_host_consents_missing")
    require(all(x["enabled"] is True and type(x.get("heap_mib")) is int and
                512 <= x["heap_mib"] <= 65536 and
                type(x.get("revision")) is int and x["revision"] > 0
                for x in (consent_a, consent_b)), "alice_or_bob_host_opt_in_missing")
    require(consent_c["enabled"] is False and consent_c.get("heap_mib") == 0,
            "carol_player_only_zero_budget_missing")
    require(a["runtime_ready"] and a["minecraft_server_process_observed"], "alice_real_server_missing")
    require(isinstance(a.get("java_runtime_version"), str) and a["java_runtime_version"],
            "alice_java_runtime_version_missing")
    require(a.get("minecraft_server_heap_mib") == consent_a["heap_mib"],
            "alice_real_java_heap_not_bounded_by_consent")
    require(not b["minecraft_server_process_observed"] and not d["minecraft_server_process_observed"],
            "parallel_authority_runtimes_before_crash")
    require(a["host_readiness_state"] == "safe" and a["safe_to_shutdown"] and
            a["successor_fingerprint"] == b["identity_fingerprint"],
            "bob_not_verified_successor_or_no_surviving_quorum")
    require(all(x["desktop_process_observed"] and x["minecraft_client_process_observed"] for x in p.values()),
            "installed_gui_or_real_client_not_observed")
    receipt = load_json(output / "crash.json")
    crash_time = timestamp(receipt.get("observed_utc"), "crash")
    require(stage_times["checkpoint"] < crash_time < stage_times["alice_crashed"],
            "crash_timestamp_outside_recorded_authority_loss")
    require(receipt["commit"] == c["commit"] and receipt["receipt"]["process_gone"] and
            receipt["receipt"]["signal"] == "SIGKILL" and
            receipt["receipt"].get("crashed_components") == ["desktop", "daemon", "server"],
            "alice_full_authority_process_crash_missing")
    recovery = stages["bob_recovered"]
    require(all(x["authority_fingerprint"] == b["identity_fingerprint"] for x in recovery.values()),
            "bob_not_sole_recovered_authority")
    require(len({(x["epoch"], x["fencing_token"]) for x in recovery.values()}) == 1,
            "recovery_generation_divergence")
    rb = recovery["bob"]
    require(rb["epoch"] is not None and a["epoch"] is not None and rb["epoch"] > a["epoch"] and
            rb["fencing_token"] is not None and a["fencing_token"] is not None and
            rb["fencing_token"] > a["fencing_token"], "unfenced_or_stale_takeover")
    require(rb["runtime_ready"] and rb["minecraft_server_process_observed"], "bob_real_recovery_runtime_missing")
    require(isinstance(rb.get("java_runtime_version"), str) and rb["java_runtime_version"],
            "bob_java_runtime_version_missing")
    require(rb.get("minecraft_server_heap_mib") == consent_b["heap_mib"],
            "bob_real_java_heap_exceeds_or_ignores_consent")
    require(rb.get("hosting_consent") == consent_b, "bob_host_consent_changed_during_takeover")
    require(not recovery["carol"]["minecraft_server_process_observed"], "carol_unexpected_server_after_recovery")
    require(all(v["desktop_process_observed"] and v["minecraft_client_process_observed"]
                for v in recovery.values()), "surviving_real_players_not_observed")
    require(all(x["state_root"] == a["state_root"] for x in recovery.values()),
            "latest_accepted_checkpoint_not_restored")
    returned = stages["alice_returned"]
    require(all(x["authority_fingerprint"] == b["identity_fingerprint"] and
                x["epoch"] == rb["epoch"] and x["fencing_token"] == rb["fencing_token"] for x in returned.values()),
            "stale_alice_or_split_brain")
    require(len({x["snapshot_hash"] for x in returned.values()}) == 1 and
            len({x["state_root"] for x in returned.values()}) == 1,
            "return_replica_divergence")
    require(returned["alice"]["desktop_process_observed"], "returning_alice_gui_missing")
    require(returned["carol"].get("hosting_consent") == consent_c and
            not returned["carol"]["minecraft_server_process_observed"],
            "carol_zero_budget_or_player_role_violated")
    require(not returned["alice"]["runtime_ready"] and not returned["alice"]["minecraft_server_process_observed"],
            "stale_alice_runtime_still_active")
    require(all(x["minecraft_client_process_observed"] for x in returned.values()),
            "real_player_reconnection_unobserved")

    # Human examination of redacted native GUI, Minecraft-client and network
    # evidence is mandatory; process presence never proves a successful join.
    require(review_file is not None, "independent_player_evidence_review_missing")
    review = load_json(review_file)
    require(review.get("commit") == c["commit"] and review.get("package_sha256") == c["package_sha256"],
            "review_candidate_mismatch")
    require(review.get("observations_sha256") == observations_sha256(output),
            "reviewed_stage_observations_changed_or_unbound")
    required = ("three_independent_gui_installations", "explicit_eula_by_alice",
                "signed_invites_and_real_client_join", "shared_ingame_marker_three_clients",
                "bob_opt_in_budget_enforced", "carol_zero_host_budget_enforced",
                "direct_or_hole_punch_transport_observed", "relay_fallback_observed",
                "measured_packet_loss_disconnect_and_reconnection",
                "alice_hard_crash_and_bob_client_reconnection", "alice_return_fenced",
                "screenshots_redacted", "real_world_network_classes_verified")
    require(all(review.get(k) is True for k in required), "real_player_or_network_review_incomplete")
    require(isinstance(review.get("reviewer"), str) and len(review["reviewer"].strip()) >= 3,
            "independent_review_identity_missing")
    url = review.get("evidence_url")
    require(isinstance(url, str) and urlparse(url).scheme == "https" and
            bool(urlparse(url).hostname) and "example." not in urlparse(url).hostname,
            "immutable_redacted_evidence_url_missing")
    # External media still require independent visual review. This guard
    # only proves that the reviewed per-role evidence files are present,
    # distinct, unmodified and bound to the verdict by their SHA-256.
    try:
        manifest_sha = verify_review_artifacts(review, output)
    except EvidenceError as exc:
        raise Blocked("unverified_real_player_media_" + str(exc)) from exc
    return {"verdict": "READY", "commit": c["commit"], "package_sha256": c["package_sha256"],
            "reviewer": review["reviewer"], "evidence_url": url,
            "redacted_media_manifest_sha256": manifest_sha,
            "observations_sha256": observations_sha256(output)}


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("action", choices=["preflight", "capture", "crash", "verify", "observations-digest"])
    p.add_argument("--config", required=True)
    p.add_argument("--output", required=True)
    p.add_argument("--stage", choices=list(STAGES))
    p.add_argument("--world")
    p.add_argument("--pids")
    p.add_argument("--pid", type=int)
    p.add_argument("--desktop-pid", type=int)
    p.add_argument("--daemon-pid", type=int)
    p.add_argument("--server-pid", type=int)
    p.add_argument("--review")
    a = p.parse_args()
    try:
        c = config(a.config)
        out = Path(a.output)
        if a.action == "preflight":
            preflight(c, out)
        elif a.action == "capture":
            require(a.stage and a.world, "capture_needs_stage_and_world")
            capture(c, out, a.stage, a.world, a.pids)
        elif a.action == "crash":
            require(a.world, "crash_needs_world")
            crash(c, out, a.world, a.desktop_pid, a.daemon_pid, a.server_pid)
        elif a.action == "observations-digest":
            print(json.dumps({"observations_sha256": observations_sha256(out),
                              "verdict": "EVIDENCE_DIGEST_ONLY_NOT_CERTIFIED"}))
            return
        else:
            result = verify(c, out, a.review)
            save(out / "verdict.json", result)
            print(json.dumps(result))
            return
        print(json.dumps({"action": a.action, "verdict": "EVIDENCE_RECORDED_NOT_CERTIFIED"}))
    except (Blocked, OSError, ValueError, KeyError, TypeError) as error:
        code = str(error) if isinstance(error, Blocked) else "missing_or_invalid_evidence"
        print(json.dumps({"verdict": "BLOCKED", "reason": code}), file=sys.stderr)
        if a.action == "verify":
            save(Path(a.output) / "verdict.json", {"verdict": "BLOCKED", "reason": code})
        raise SystemExit(2) from None


if __name__ == "__main__":
    main()
