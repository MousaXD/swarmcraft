#!/usr/bin/env python3
"""Real signed directory moderation, report review, and canonical isolation."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import tempfile
import time

from private_infrastructure_smoke import ROOT, WORLD, Service, command, isolated_env


def canonical_bytes(profile):
    """Hash the retained signed protocol files, not local preference markers."""
    result = {}
    for file in sorted((profile / "worlds").rglob("*.postcard")):
        if file.is_file():
            result[str(file.relative_to(profile))] = hashlib.sha256(file.read_bytes()).hexdigest()
    assert result, "no canonical fixture evidence"
    return result


def smoke(cli, runtime):
    evidence = {"scope": "real signed loopback publisher; isolated owner, client and reviewing operator; mDNS disabled", "checks": {}}
    checks = evidence["checks"]
    with tempfile.TemporaryDirectory(prefix="swarmcraft-directory-moderation-") as folder:
        root = Path(folder)
        owner, client, operator = (root / name for name in ("owner", "client", "operator"))
        env = isolated_env()
        env.pop("SWARMCRAFT_DISCOVERY_BOOTSTRAP", None)

        def local(profile, *argv, expect=0):
            return command([cli, "--data-dir", profile, *argv], env=env, expect=expect)

        def run(profile, *argv, expect=0):
            result = command([runtime, "--data-dir", profile, *argv], env=env, expect=expect, timeout=40)
            return json.loads(result) if expect == 0 else None

        def policy(profile, *argv, expect=0):
            return run(profile, "moderation", *argv, expect=expect)

        worlds = {}
        for name, visibility, region, language in [
            ("Governance Arabic", "public", "middle-east", "ar"),
            ("Governance English", "public", "europe", "en"),
            ("Governance Unlisted", "unlisted", "europe", "en"),
            ("Governance Private", "private", None, None),
        ]:
            args = ["world", "create", "--name", name, "--minecraft", "26.1.2",
                    "--fabric-loader", "0.19.3", "--visibility", visibility]
            if region:
                args.extend(["--region", region, "--language", language])
            worlds[name] = WORLD.search(local(owner, *args)).group(0)
        before = canonical_bytes(owner)
        local(owner, "world", "create", "--name", "Invalid private metadata", "--minecraft", "26.1.2",
              "--fabric-loader", "0.19.3", "--visibility", "private", "--region", "europe", expect=1)
        assert canonical_bytes(owner) == before
        checks["private_metadata_rejected_before_creation"] = True
        arabic, english, unlisted, private = (worlds["Governance " + name] for name in ("Arabic", "English", "Unlisted", "Private"))

        publisher = Service([runtime, "--data-dir", owner, "discovery-serve", "--listen", "/ip4/127.0.0.1/tcp/0"],
                            root / "publisher.log", env)
        try:
            address = publisher.address("Discovery provider address: ")

            def browse(profile=client, *args):
                return run(profile, "discovery-search", "--query", "Governance", *args, "--bootstrap", address)

            def resolve(profile, world):
                return run(profile, "discovery-resolve", world, "--bootstrap", address)

            def require_public(profile, *args, expected):
                # A partial result is allowed only when all expected matching
                # fixture cards are verified; retries never weaken proof.
                deadline = time.monotonic() + 45
                while True:
                    report = browse(profile, *args)
                    ids = {card["world_id"] for card in report["results"]}
                    assert ids <= expected, "wrong metadata, hidden or nonpublic result"
                    if ids == expected and report["providers_responded"] >= 1:
                        return report
                    assert time.monotonic() < deadline, "expected signed public listing unavailable"
                    time.sleep(.2)

            report = require_public(client, expected={arabic, english})
            by_id = {card["world_id"]: card for card in report["results"]}
            assert by_id[arabic]["approximate_region"] == "middle-east" and by_id[arabic]["tags"] == ["lang:ar"]
            assert by_id[english]["approximate_region"] == "europe" and by_id[english]["tags"] == ["lang:en"]
            assert all(card["visibility"] == "public" and card["join_action"] == "invite_required" for card in by_id.values())
            config = by_id[arabic]["config_hash"]
            assert len(config) == 64
            checks["signed_coarse_metadata_public_only"] = True
            require_public(client, "--region", "middle-east", "--language", "ar", expected={arabic})
            require_public(client, "--region", "europe", "--language", "en", expected={english})
            mismatch = browse(client, "--region", "europe", "--language", "ar")
            assert not mismatch["results"] and mismatch["providers_responded"] >= 1
            checks["region_and_language_filter"] = True

            # The report references the exact signed config, not a mutable
            # display name. Unlisted/private reports must never be stored.
            unlisted_result = resolve(client, unlisted)
            assert unlisted_result["state"] == "found" and unlisted_result["world"]["visibility"] == "unlisted"
            policy(client, "report", unlisted, "--config", unlisted_result["world"]["config_hash"],
                   "--reason", "spam", "--bootstrap", address, expect=1)
            policy(client, "report", private, "--config", config, "--reason", "spam", "--bootstrap", address, expect=1)
            policy(client, "report", arabic, "--config", "0" * 64, "--reason", "spam", "--bootstrap", address, expect=1)
            assert not policy(client, "state")["reports"]
            checks["nonpublic_and_mismatched_reports_refused"] = True
            saved = policy(client, "report", arabic, "--config", config, "--reason", "misleading_metadata", "--bootstrap", address)
            assert len(saved["reports"]) == 1 and not saved["hidden_worlds"] and not saved["directory_suppressions"]
            entry = saved["reports"][0]
            assert set(entry) == {"report_id", "world_id", "config_hash", "reason", "verified_unix_ms", "recorded_unix_ms"}
            assert entry["world_id"] == arabic and entry["config_hash"] == config and entry["reason"] == "misleading_metadata"
            duplicate = policy(client, "report", arabic, "--config", config, "--reason", "misleading_metadata", "--bootstrap", address)
            assert duplicate["reports"] == [entry]
            assert not policy(owner, "state")["reports"]
            checks["report_local_minimal_idempotent_not_sent"] = True

            exported = policy(client, "export-report", entry["report_id"])
            assert exported == {"schema": "swarmcraft-local-listing-report-v1", "allegation": True, **entry}
            report_file = root / "explicitly-shared-report.json"
            report_file.write_text(json.dumps(exported), encoding="utf-8")
            review = policy(operator, "review-report", "--report-file", report_file, "--bootstrap", address)
            assert review["matching_current_public_listing"] is True and review["reporter_allegation_verified"] is False
            assert review["automatic_decision"] is False and not policy(operator, "state")["reports"]
            report_file.write_text(json.dumps({**exported, "unexpected_secret": "scinvite:00"}))
            policy(operator, "review-report", "--report-file", report_file, "--bootstrap", address, expect=1)
            checks["independent_review_no_automatic_decision"] = True

            policy(client, "hide", arabic, "--enabled=true")
            hidden = require_public(client, expected={english})
            assert hidden["hidden_results"] == 1
            all_hidden = browse(client, "--region", "middle-east")
            assert not all_hidden["results"] and all_hidden["hidden_results"] == 1 and all_hidden["state"] in ("ok", "partial")
            assert resolve(client, arabic)["state"] == "found"
            require_public(operator, expected={arabic, english})
            policy(client, "hide", arabic, "--enabled=false")
            require_public(client, expected={arabic, english})
            checks["local_hide_is_reversible_and_not_global"] = True

            policy(owner, "suppress", arabic, "--reason", "misleading_metadata")
            require_public(client, expected={english})
            assert resolve(client, arabic)["state"] == "found"
            policy(owner, "restore", arabic)
            require_public(client, expected={arabic, english})
            checks["publisher_browse_suppression_preserves_exact_lookup"] = True

            # Malformed directory policy must not resume public browse, but
            # does not veto canonical state or independent exact discovery.
            policy_path = owner / "directory-policy.json"
            good = policy_path.read_bytes()
            policy_path.write_bytes(b"{corrupted")
            denied = browse(client)
            assert not denied["results"] and denied["state"] != "ok"
            assert resolve(client, arabic)["state"] == "found"
            policy_path.write_bytes(good)
            require_public(client, expected={arabic, english})
            checks["malformed_directory_policy_fails_closed_browse_only"] = True

            pause = json.loads(local(owner, "world", "discovery-listing", arabic, "--paused", "true"))
            assert pause["locally_paused"] and pause["signed_visibility"] == "public"
            assert pause["signed_visibility_changed"] is False and pause["world_membership_changed"] is False
            denied = resolve(client, arabic)
            assert denied["state"] != "found" and denied["world"] is None
            resumed = json.loads(local(owner, "world", "discovery-listing", arabic, "--paused", "false"))
            assert resumed["locally_paused"] is False and resumed["signed_visibility"] == "public"
            require_public(client, expected={arabic, english})
            checks["local_publication_pause_and_resume_preserve_signed_visibility"] = True

            dismissed = policy(client, "dismiss", entry["report_id"])
            assert not dismissed["reports"] and not dismissed["hidden_worlds"]
            assert canonical_bytes(owner) == before, "moderation changed canonical protocol records"
            assert local(client, "world", "list").strip() == "No local worlds."
            checks["canonical_protocol_files_unchanged"] = len(before)
            checks["no_membership_created"] = True
            checks["report_deletion_independent"] = True
        finally:
            publisher.close()
    return evidence


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", type=Path, default=ROOT / "target/debug")
    parser.add_argument("--evidence", type=Path)
    args = parser.parse_args()
    os.umask(0o077)
    result = smoke(args.bin_dir / "swarmcraft", args.bin_dir / "swarmcraft-runtime")
    if args.evidence:
        args.evidence.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))
