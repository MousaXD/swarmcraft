#!/usr/bin/env python3
"""Real isolated signed friend presence service; no WAN or user profile."""
import argparse
import json
import os
from pathlib import Path
import tempfile
import time

from private_infrastructure_smoke import ROOT, Service, command, isolated_env, WORLD


def smoke(cli, runtime, infra):
    env = isolated_env()
    env.pop("SWARMCRAFT_DISCOVERY_BOOTSTRAP", None)
    evidence = {"scope": "real loopback authenticated publisher, three independent local identities", "checks": {}}
    checks = evidence["checks"]
    with tempfile.TemporaryDirectory(prefix="swarmcraft-friends-") as directory:
        root = Path(directory)
        alice, bob, eve = [root / name for name in ("alice", "bob", "eve")]
        def local(profile, *args, expect=0):
            return command([cli, "--data-dir", profile, *args], env=env, expect=expect)
        def contact(profile, *args):
            return json.loads(local(profile, "contact", *args))
        def settings(profile):
            return contact(profile, "settings")
        def add(owner, other, label):
            identity = settings(other)
            result = contact(owner, "add", identity["own_peer_id"], "--public-key", identity["own_public_key_hex"], "--label", label)
            assert result["share_presence"] is False
            return identity["own_peer_id"]
        alice_id = add(bob, alice, "Alice")
        bob_id = add(alice, bob, "Bob")
        eve_id = add(alice, eve, "Eve")
        add(eve, alice, "Alice")
        # Keep a public world to prove that presence-only start is not a
        # public discovery switch.
        created = local(alice, "world", "create", "--name", "Presence smoke public", "--minecraft", "26.1.2",
                        "--fabric-loader", "0.19.3", "--visibility", "public")
        world = WORLD.search(created).group(0)
        args = [runtime, "--data-dir", alice, "discovery-serve", "--presence-only", "--listen", "/ip4/127.0.0.1/tcp/0"]
        command(args, env=env, expect=1)
        checks["default_hidden_start_refused"] = True
        contact(alice, "presence-mode", "--enabled=true")
        command(args, env=env, expect=1)
        checks["global_enable_alone_refused"] = True
        contact(alice, "share-presence", bob_id, "--enabled=true")
        locator = Service([infra, "--state-dir", root / "locator", "locator", "--listen", "/ip4/127.0.0.1/tcp/0"], root / "locator.log", env)
        service = None
        try:
            locator_address = locator.address("Locator address: ")
            service = Service([*args, "--bootstrap", locator_address], root / "publisher.log", env)
            address = service.address("Discovery provider address: ")
            def probe(profile, peer, bootstrap=None):
                start = time.monotonic()
                result = contact(profile, "presence", peer, "--bootstrap", bootstrap or address)
                assert result["peer_id"] == peer
                assert set(result) == {"peer_id","state","observed_unix_ms","expires_unix_ms","detail"}
                assert result["detail"] is None
                if result["state"] == "online":
                    assert result["observed_unix_ms"] <= int(time.time()*1000) < result["expires_unix_ms"]
                    assert 0 < result["expires_unix_ms"]-result["observed_unix_ms"] <= 30_000
                else:
                    assert result["observed_unix_ms"] is None and result["expires_unix_ms"] is None
                    assert result["state"] != "offline"
                return {"state": result["state"], "seconds":round(time.monotonic()-start,3)}
            allowed = probe(bob,alice_id)
            assert allowed["state"] == "online", allowed
            checks["allowed_contact"] = allowed
            # The client knows only the locator, never the publisher
            # address. mDNS is disabled on every process.
            deadline = time.monotonic() + 45
            while True:
                routed = probe(bob, alice_id, locator_address)
                if routed["state"] == "online":
                    break
                assert time.monotonic() < deadline, "locator did not discover explicitly consented presence"
                time.sleep(.25)
            checks["locator_only_allowed_contact"] = routed
            denied = probe(eve,alice_id)
            assert denied["state"] != "online", denied
            checks["ungranted_contact"] = denied
            result = json.loads(command([runtime,"--data-dir",bob,"discovery-resolve",world,"--bootstrap",address],env=env))
            assert result["state"] != "found" and result["world"] is None
            checks["presence_only_world_metadata_refused"] = True
            # Keep Eve explicitly consented so the process survives Bob's
            # revocation. Stale DHT hints and direct address cannot authorize.
            contact(alice,"share-presence",eve_id,"--enabled=true")
            contact(alice,"share-presence",bob_id,"--enabled=false")
            denied = probe(bob,alice_id)
            assert denied["state"] != "online",denied
            checks["withdrawal_while_running"] = denied
            contact(alice,"share-presence",bob_id,"--enabled=true")
            removed=contact(alice,"remove",bob_id)
            assert removed=={"removed":True,"world_membership_changed":False}
            denied=probe(bob,alice_id)
            assert denied["state"]!="online",denied
            checks["removed_contact_stale_locator"] = denied
            add(alice,bob,"Bob")
            denied=probe(bob,alice_id)
            assert denied["state"]!="online",denied
            checks["readded_contact_hidden"] = denied
            contact(bob,"remove",alice_id)
            local(bob,"contact","presence",alice_id,"--bootstrap",address,expect=1)
            checks["removed_local_contact_probe_refused"]=True
            contact(alice,"presence-mode","--enabled=false")
            service.process.wait(timeout=8)
            assert service.process.returncode==0
            checks["hide_stops_presence_only_process"]=True
            assert local(bob,"world","list").strip()=="No local worlds."
            checks["no_membership_created"]=True
            state=settings(alice)
            assert not state["presence_enabled"] and next(c for c in state["contacts"] if c["peer_id"]==eve_id)["share_presence"]
            checks["hide_retains_explicit_grants_without_sharing"]=True
        finally:
            if service:
                service.close()
            locator.close()
    return evidence


if __name__ == "__main__":
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir",type=Path,default=ROOT/"target/debug")
    parser.add_argument("--evidence",type=Path)
    args=parser.parse_args()
    os.umask(0o077)
    result=smoke(args.bin_dir/"swarmcraft",args.bin_dir/"swarmcraft-runtime",args.bin_dir/"swarmcraft-infra")
    if args.evidence:
        args.evidence.write_text(json.dumps(result,indent=2)+"\n")
    print(json.dumps(result,indent=2))
