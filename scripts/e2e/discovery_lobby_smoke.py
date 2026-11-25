#!/usr/bin/env python3
"""Exercise a bounded signed multi-world discovery snapshot using real local sidecars.

No Minecraft process, account, hosted service, or operator store is used.
"""
import argparse
import json
import os
from pathlib import Path
import tempfile
import time

from private_infrastructure_smoke import ROOT, WORLD, Service, command, isolated_env


def smoke(cli, runtime):
    evidence = {"scope": "real loopback publisher and runtime CLI; mDNS disabled", "checks": {}}
    with tempfile.TemporaryDirectory(prefix="swarmcraft-discovery-lobby-") as folder:
        root = Path(folder)
        owner, client = root / "owner", root / "client"
        env = isolated_env()
        env.pop("SWARMCRAFT_DISCOVERY_BOOTSTRAP", None)
        worlds = {}
        for name, visibility in [(f"Lobby village {i:02}", "public") for i in range(9, 0, -1)] + [
            ("Lobby unlisted", "unlisted"), ("Lobby private", "private"),
        ]:
            output = command([cli, "--data-dir", owner, "world", "create", "--name", name,
                              "--minecraft", "26.1.2", "--fabric-loader", "0.19.3",
                              "--visibility", visibility], env=env)
            worlds[name] = WORLD.search(output).group(0)
        publisher = Service([runtime, "--data-dir", owner, "discovery-serve",
                             "--listen", "/ip4/127.0.0.1/tcp/0"], root / "publisher.log", env)
        try:
            address = publisher.address("Discovery provider address: ")

            def invoke(kind, *arguments):
                started = time.monotonic()
                result = json.loads(command([runtime, "--data-dir", client, kind, *arguments,
                                             "--bootstrap", address], env=env, timeout=35))
                return result, round(time.monotonic() - started, 3)

            def check_card(card, name, visibility):
                assert card["world_id"] == worlds[name]
                assert card["visibility"] == visibility and card["join_action"] == "invite_required"
                assert card["membership_policy"] == "invite_only"
                assert card["issued_unix_ms"] <= card["verified_unix_ms"] < card["expires_unix_ms"]
                assert card["verified_unix_ms"] <= int(time.time() * 1000)
                assert card["expires_unix_ms"] > int(time.time() * 1000)

            expected = sorted(name for name in worlds if name.startswith("Lobby village"))
            # Wait for publication only, not for a different or weaker proof.
            deadline = time.monotonic() + 40
            while True:
                full, elapsed = invoke("discovery-search", "--query", "Lobby")
                if len(full["results"]) == len(expected):
                    break
                assert time.monotonic() < deadline, "publisher did not expose all verified fixture worlds"
                time.sleep(0.25)
            assert [card["name"] for card in full["results"]] == expected
            for card in full["results"]:
                check_card(card, card["name"], "public")
            assert full["providers_responded"] >= 1
            assert not full["truncated"]
            evidence["checks"]["public_snapshot"] = {
                "verified_count": len(full["results"]), "state": full["state"], "seconds": elapsed,
                "private_unlisted_excluded": True, "point_in_time_verified": True,
            }

            capped, elapsed = invoke("discovery-search", "--query", "Lobby", "--limit", "3")
            assert capped["state"] == "partial" and capped["truncated"]
            assert [card["name"] for card in capped["results"]] == expected[:3]
            evidence["checks"]["capped_snapshot"] = {
                "verified_count": len(capped["results"]), "truncated": True, "deterministic_order": True,
                "seconds": elapsed,
            }

            absent, elapsed = invoke("discovery-search", "--query", "Nonmatching test query")
            assert absent["results"] == [] and absent["providers_responded"] >= 1
            assert absent["state"] in ("empty", "partial"), absent["state"]
            evidence["checks"]["no_matching_listing"] = {"state": absent["state"], "seconds": elapsed}

            unlisted, elapsed = invoke("discovery-resolve", worlds["Lobby unlisted"])
            assert unlisted["state"] == "found", unlisted["state"]
            check_card(unlisted["world"], "Lobby unlisted", "unlisted")
            evidence["checks"]["unlisted_exact_lookup"] = {"verified": True, "seconds": elapsed}

            private, elapsed = invoke("discovery-resolve", worlds["Lobby private"])
            assert private["state"] != "found" and private["world"] is None
            evidence["checks"]["private_exact_lookup"] = {"refused": True, "state": private["state"], "seconds": elapsed}
            assert command([cli, "--data-dir", client, "world", "list"], env=env).strip() == "No local worlds."
            evidence["checks"]["membership_not_created"] = True
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
