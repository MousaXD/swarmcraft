"""#65 isolated signed-policy pin for authenticated reference regional workers.

Rust verifies the creation certificate against independently selected identity
and compatibility pins before this trusted local controller starts workers.
The pin and lab MACs do not establish a canonical ownerless head or quorum.
"""
from __future__ import annotations

import hashlib
import hmac
import json
import subprocess
import time
from pathlib import Path

from region_runtime import digest, fields, require, canonical
from regional_authority import Authority, FencedWorker, checked_grant

POLICY_HASH = "c694ae9067292c7ea3da7cc5a825e8ff0b370cccfa6267e45dc3a6c96f76ef4f"
DOMAIN = b"sc65-ownerless-reference-permission-receipt-v1\0"


def parts(binding):
    require(isinstance(binding, str), "missing ownerless binding")
    values = binding.split(":")
    require(len(values) == 4 and values[0] == "sc65v1" and values[2] == POLICY_HASH
            and all(digest(v) and v != "0"*64 for v in values[1:]), "invalid ownerless binding")
    return values[1], values[2], values[3]


def verify_profile(cli, certificate, expected_world, expected_compatibility, empty_profile):
    output = subprocess.run([str(Path(cli).resolve()), "--data-dir", str(empty_profile),
                             "ownerless", "permission-profile", "--certificate", str(certificate),
                             "--expected-world", expected_world,
                             "--expected-compatibility", expected_compatibility],
                            check=True, capture_output=True, timeout=10)
    report = json.loads(output.stdout)
    world, policy, compatibility = parts(report["binding"])
    require(report == {"classification": "verified_ownerless_permission_lab_profile",
                       "binding": report["binding"], "world_id": "scownerless:"+world,
                       "policy_hash": policy, "compatibility": compatibility,
                       "canonical_authority": False, "runtime_activation": False,
                       "player_admission": False}
            and report["world_id"] == expected_world and compatibility == expected_compatibility
            and not Path(empty_profile).exists(), "permission profile not independently verified")
    return report["binding"]


def policy_mac(grant, binding, receipt):
    return hmac.digest(bytes.fromhex(grant["key"]),
                       DOMAIN + canonical([binding, receipt]), "sha256").hex()


class PolicyWorker(FencedWorker):
    def __init__(self, binding):
        super().__init__()
        self.world, _, _ = parts(binding)
        self.binding = binding

    def handle(self, request):
        op = request.get("op") if isinstance(request, dict) else None
        if op == "bind_policy_grant":
            fields(request, "op", "grant", "binding")
            grant = checked_grant(request["grant"])
            if request["binding"] != self.binding or grant["world"] != self.world or self.grant is not None:
                return {"op": "refused", "reason": "policy grant mismatch or already bound"}
            reply = super().handle({"op": "bind_grant", "grant": grant})
            return {**reply, "binding": self.binding}
        if op == "bind_grant":
            return {"op": "refused", "reason": "policy binding required"}
        if op in ("op", "set_policy", "console", "rcon", "grant_vote"):
            return {"op": "refused", "reason": "no privileged worker operation"}
        if op == "fenced_snapshot":
            receipt = super().handle(request)
            return {**receipt, "binding": self.binding,
                    "policy_mac": policy_mac(self.grant, self.binding, receipt)}
        return super().handle(request)


class PolicyChannel:
    """Adapt the existing fenced barrier without weakening its checks."""
    def __init__(self, channel, binding):
        self.channel, self.binding = channel, binding
        self.awaiting_bind = False

    def send(self, request, deadline):
        if request.get("op") == "bind_grant":
            self.awaiting_bind = True
            request = {"op": "bind_policy_grant", "grant": request["grant"], "binding": self.binding}
        self.channel.send(request, deadline)

    def recv(self, deadline):
        response = self.channel.recv(deadline)
        if self.awaiting_bind:
            self.awaiting_bind = False
            fields(response, "op", "session", "generation", "binding")
            require(response["binding"] == self.binding, "worker returned wrong policy")
            return {k: v for k, v in response.items() if k != "binding"}
        return response


def authority_for(binding):
    """Separate database application ID and ruleset pin. Not a live realm."""
    world, _, _ = parts(binding)
    class PolicyAuthority(Authority):
        app_id = 0x53433635
        ruleset = "swarmcraft-reference-ownerless-permission-lab/" + binding

        @classmethod
        def create(cls, path, identity, state, players, groups):
            require(identity == world, "wrong ownerless world identity")
            return super().create(path, identity, state, players, groups)

        def _read(self):
            current = super()._read()
            require(current["world"] == world, "wrong ownerless world identity")
            return current

        @staticmethod
        def cut(identity, tick, root, players):
            require(identity == world, "wrong ownerless cut identity")
            return hashlib.sha256(b"sc65-reference-policy-cut-v1\0" +
                                  canonical([binding, tick, root, players])).hexdigest()

        def _validate_receipt(self, current, receipt):
            fields(receipt, "body", "mac", "binding", "policy_mac")
            require(receipt["binding"] == binding and digest(receipt["policy_mac"]),
                    "ownerless receipt policy mismatch")
            session = receipt["body"].get("session") if isinstance(receipt["body"], dict) else None
            grants = [g for g in current["grants"] if g["session"] == session]
            require(len(grants) == 1, "stale ownerless receipt session")
            inner = {"body": receipt["body"], "mac": receipt["mac"]}
            require(hmac.compare_digest(receipt["policy_mac"], policy_mac(grants[0], binding, inner)),
                    "ownerless policy receipt authentication failed")
            return inner

    return PolicyAuthority


def rpc(channel, request):
    deadline = time.monotonic() + 3
    channel.send(request, deadline)
    return channel.recv(deadline)
