#!/usr/bin/env python3
"""#68 local combined reference session, NOT Minecraft/production authority.

Use disposable Linux user/network namespaces and a delegated cgroup scope.
The trusted controller stages a local transaction, obtains signed ownerless
checkpoint certification, and only then acknowledges reference clients.
No real-account login, independently owned machine, or live quorum lease.
"""
from __future__ import annotations

import argparse
from contextlib import contextmanager
import hashlib
import json
import os
from pathlib import Path
import re
import secrets
import selectors
import socket
import subprocess
import tempfile
import time

from network_lab import cpu_plan, safe_namespace
from ownerless_permission import PolicyChannel, authority_for, parts, rpc, verify_profile
from placement_lab import CappedLab, Scope
from player_runtime import fixture, player_index
from region_runtime import (MAX_TICK, Rejected, canonical, digest, fields, integer, region_roots, require,
                            serial_step, world_root)
from regional_authority import (MAX_GENERATION, candidate_receipts, checked_grant, checked_groups,
                                new_grants, public_grants, validated_state, wire)
from transition_lab import Clients
from transition_runtime import (DurableRuntime, TransitionAuthority, checked_batch, checked_command,
                                response, transition_view)
from worker_placement import Offer, assess, evaluate, region_load

CLASSIFICATION = "LOCAL_COMBINED_CFT_REFERENCE_SESSION_NOT_MINECRAFT_NOT_MULTI_HOST"
ROOT = Path(__file__).resolve().parent
FULL = list(range(8))
HALVES = [list(range(4)), list(range(4, 8))]
ARTIFACTS = ("combined_ownerless_lab.py", "ownerless_permission.py", "placement_lab.py",
             "network_lab.py", "region_runtime.py", "regional_authority.py", "transition_runtime.py",
             "transition_lab.py", "player_runtime.py", "worker_placement.py")
FALSE_FLAGS = ("validator_granted", "write_authority", "live_quorum_verified",
               "resource_enforcement_verified", "runtime_activation", "worker_admission", "player_admission")


def sha(data):
    return hashlib.sha256(data).hexdigest()


def artifact_manifest(cli):
    # Pin the exact local reference implementation, not Minecraft artifacts.
    result = {name: sha((ROOT/name).read_bytes()) for name in ARTIFACTS}
    with Path(cli).open("rb") as f:
        result["rust-cli"] = hashlib.file_digest(f, "sha256").hexdigest()
    return result


def compatibility_for(manifest):
    return sha(b"sc68-combined-reference-artifacts-v1\0" + canonical(manifest))


def combined_authority(binding):
    Policy = authority_for(binding)

    class Combined(Policy, TransitionAuthority):
        app_id = 0x53433638
        ruleset = "swarmcraft-reference-combined-ownerless/v1/" + binding

        @staticmethod
        def cut(world, tick, root, players):
            require(world == parts(binding)[0], "combined world differs from signed policy pin")
            return sha(b"sc68-combined-reference-cut-v1\0" +
                       canonical([binding, tick, root, players]))

    return Combined


class CappedPolicyLab(CappedLab):
    def __init__(self, scope, offers, cpus, binding):
        self.binding = binding
        super().__init__(scope, offers, {}, cpus)

    def worker_command(self, index):
        command = super().worker_command(index)
        require(command[-1] == "--worker", "unexpected capped worker command")
        return command[:-1] + ["--ownerless-worker-binding", self.binding]


def snapshot(mapping, state, tick, audit):
    return {"format": "sc68-reference-checkpoint-v1", "player_keys": mapping,
            "players": [0, 1], "tick": tick, "runtime_root": world_root(tick, region_roots(state)),
            "regions": wire(state), "transaction_journal": audit}


def observed_snapshot(ledger, mapping):
    with ledger.transaction():
        current = ledger._read()
        return committed_snapshot(ledger, mapping, current)


def committed_snapshot(ledger, mapping, current):
    """Exact certified bytes from one audited SQLite transaction.

    Pending inputs are local staging, not part of the accepted cut.
    Audit still validates them; only committed rows enter the snapshot.
    """
    journal = ledger._audit(current)
    committed = {"genesis": journal["genesis"],
                 "batches": [b for b in journal["batches"] if b["tick"] <= current["tick"]],
                 "commands": [c for c in journal["commands"] if c["request"]["tick"] <= current["tick"]]}
    return snapshot(mapping, current["state"], current["tick"], committed)


def review_checkpoint(binding, mapping, prior, candidate):
    """Replay the exact new cut from a validator's retained certified bytes.

    The previous checkpoint has already passed signed-history verification.
    This review uses no regional receipt, coordinator state or worker code.
    It is invoked by the trusted local fixture before each network vote,
    not a general validator-side sandbox for arbitrary Minecraft saves.
    """
    keys = ("format","player_keys","players","tick","runtime_root","regions","transaction_journal")
    fields(prior,*keys)
    fields(candidate,*keys)
    for cut in (prior,candidate):
        require(cut["format"] == "sc68-reference-checkpoint-v1"
                and cut["player_keys"] == list(mapping) and cut["players"] == [0,1],
                "checkpoint identity mapping differs")
        fields(cut["transaction_journal"],"genesis","batches","commands")
    require(integer(prior["tick"],0,999_999),"invalid review base tick")
    tick = prior["tick"] + 1
    require(integer(candidate["tick"],tick,tick),"review requires exactly one next tick")
    before, _ = validated_state(prior["regions"])
    require(prior["runtime_root"] == world_root(tick-1,region_roots(before)),
            "review base root differs")
    world = parts(binding)[0]
    cut = combined_authority(binding).cut
    parent = cut(world,tick-1,prior["runtime_root"],[0,1])
    old, new = prior["transaction_journal"], candidate["transaction_journal"]
    require(canonical(old["genesis"]) == canonical(new["genesis"])
            and len(old["batches"]) == tick-1 and len(new["batches"]) == tick
            and canonical(new["batches"][:-1]) == canonical(old["batches"])
            and canonical(new["commands"][:len(old["commands"])]) == canonical(old["commands"]),
            "review checkpoint journal is not an exact extension")
    batch = new["batches"][-1]
    fields(batch,"tick","parent","batch","result")
    require(integer(batch["tick"],tick,tick) and batch["parent"] == parent,
            "review wrong sealed parent")
    commands = new["commands"][len(old["commands"]):]
    ids = {r["request"]["id"] for r in old["commands"]}
    expected_batch = [[0,0,0,None],[1,0,0,None]]
    previous_player = -1
    for row in commands:
        fields(row,"player","request","response")
        player, request = row["player"], row["request"]
        checked_command(player,request,world,[0,1])
        require(player > previous_player and request["id"] not in ids
                and request["tick"] == tick and request["parent"] == parent,
                "review duplicate or stale command")
        previous_player = player
        ids.add(request["id"])
        expected_batch[player] = [player,request["direction"],request["action"],request["id"]]
    require(canonical(batch["batch"]) == canonical(expected_batch),"review input batch differs")
    next_state = serial_step(before,tick,controls={p:(d,a) for p,d,a,_ in expected_batch})
    root = world_root(tick,region_roots(next_state))
    result = cut(world,tick,root,[0,1])
    require(batch["result"] == result and candidate["runtime_root"] == root
            and canonical(candidate["regions"]) == canonical(wire(next_state)),
            "review replay differs from candidate")
    positions = player_index(before,{0,1})
    for row in commands:
        player, request = row["player"], row["request"]
        require(canonical(row["response"]) == canonical(response(world,tick,result,next_state,[0,1],
                    player,request,positions[player][1])), "review committed receipt differs")
    return root


SOURCE_PIN = ("world","generation","tick","runtime_root","cut_root")


def restoration_source_pin(save):
    return {key:save[key] for key in SOURCE_PIN}


def _restoration_status(quorum, head, verifier):
    status = quorum.auth(1,"status","--expected-world",quorum.world)
    expected = {**head,"local_identity":verifier,"local_validator":True,"pending_proposal":None}
    require(canonical(status) == canonical(expected),"restoration authority pin, identity or pending vote differs")
    return status


def _certified_prefix(ledger, current, raw, mapping):
    """Retain only the exact signed prefix, with actual local provenance.

    The full source is audited first, including its unaccepted suffix.
    No worker receipt digest or historical generation is synthesized.
    """
    candidate = json.loads(raw)
    fields(candidate,"format","player_keys","players","tick","runtime_root","regions","transaction_journal")
    tick = candidate["tick"]
    require(integer(tick,0,current["tick"]) and current["players"] == [0,1],
            "certified checkpoint not available in source prefix")
    journal = ledger._audit(current)
    prefix = {"genesis":journal["genesis"],
              "batches":[b for b in journal["batches"] if b["tick"] <= tick],
              "commands":[c for c in journal["commands"] if c["request"]["tick"] <= tick]}
    genesis, _ = validated_state(prefix["genesis"])
    state = genesis
    for batch in prefix["batches"]:
        state = serial_step(state,batch["tick"],controls={p:(d,a) for p,d,a,_ in batch["batch"]})
    require(canonical(snapshot(list(mapping),state,tick,prefix)) == raw,
            "certified checkpoint differs from audited source prefix")
    commits = ledger.db.execute("SELECT * FROM commits WHERE tick<=? ORDER BY tick",(tick,)).fetchall()
    accepted_generation = commits[-1][1]
    epochs = ledger.db.execute("SELECT * FROM epochs WHERE generation<=? ORDER BY generation",
                               (accepted_generation,)).fetchall()
    roots = {row[0]:row[2] for row in commits}
    epoch_by_generation, sessions = {},set()
    last_base = 0
    for generation, base_tick, root, grants_raw, claimed in epochs:
        require(integer(generation,1,accepted_generation) and integer(base_tick,last_base,tick)
                and roots.get(base_tick) == root and integer(claimed,0,1),
                "invalid retained epoch ancestry")
        last_base = base_tick
        grants = json.loads(grants_raw)
        require(canonical(grants) == grants_raw and isinstance(grants,list),"invalid retained public grants")
        checked_groups([g["regions"] for g in grants])
        for grant in grants:
            fields(grant,"world","generation","session","regions","base_tick","base_root")
            checked_grant({**grant,"key":"0"*64}) # shape check only; no fabricated key is stored
            require(grant["world"] == current["world"] and grant["generation"] == generation
                    and grant["base_tick"] == base_tick and grant["base_root"] == root
                    and grant["session"] not in sessions,"invalid retained grant provenance")
            sessions.add(grant["session"])
        epoch_by_generation[generation] = (base_tick,claimed)
    require(commits[0][1] == 0 and commits[0][4:] == (None,None),"invalid retained genesis")
    previous = 0
    for t,g,root,cut,parent,receipt in commits[1:]:
        epoch = epoch_by_generation.get(g)
        require(integer(g,max(1,previous),accepted_generation) and epoch is not None
                and epoch[0] < t and epoch[1] == 1 and digest(receipt),
                "missing retained worker receipt provenance")
        previous = g
    batches = ledger.db.execute("SELECT * FROM batches WHERE tick<=? ORDER BY tick",(tick,)).fetchall()
    commands = ledger.db.execute("SELECT * FROM commands WHERE tick<=? ORDER BY tick,player",(tick,)).fetchall()
    return genesis,state,tick,commits,epochs,batches,commands


def _fsync_path(path, directory=False):
    fd = os.open(path,os.O_RDONLY | (os.O_DIRECTORY if directory else 0))
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def restore_certified_staging(quorum, source, destination, *, expected_source, source_retired,
                              before_publish=None):
    """Build a separate, quarantined reference database from a certified prefix.

    This trusted-fixture operation requires an explicitly retired source,
    an exact source pin and an existing independently verified local head.
    It never opens the original for writing, replaces a path, signs a vote,
    admits a worker or switches a running service to the new database.
    The original must remain retired; process ownership is not proven by
    a Boolean. A new placement/role check and rotation are required.
    """
    require(source_retired is True,"source runtime must be explicitly retired")
    fields(expected_source,*SOURCE_PIN)
    require(integer(expected_source["generation"],1,MAX_GENERATION-2)
            and integer(expected_source["tick"],0,MAX_TICK)
            and all(digest(expected_source[k]) for k in ("world","runtime_root","cut_root")),
            "invalid source restoration pin or generation exhausted")
    source = Path(source)
    destination = Path(destination)
    require(source.is_file() and not source.is_symlink(),"missing or linked restoration source")
    require(destination.name not in ("",".",".."),"invalid restoration destination")
    destination = destination.parent.resolve(strict=True)/destination.name
    def vacant():
        require(not any(os.path.lexists(str(destination)+suffix) for suffix in ("","-journal","-wal","-shm")),
                "restoration destination or sidecar already exists")
    vacant()
    head = dict(quorum.head)
    mapping = tuple(quorum.mapping)
    require(len(mapping) == len(set(mapping)) == 2 and all(digest(k) for k in mapping)
            and parts(quorum.binding) == (expected_source["world"],head["policy_hash"],head["compatibility"])
            and quorum.world == head["world_id"] == "scownerless:"+expected_source["world"]
            and head["participation_mode"] == "invite_only","restoration world/policy pin differs")
    verifiers = [v["peer_id"] for v in head["validators"] if v["public_key"] == quorum.keys[1]]
    require(len(verifiers)==1,"restoration verifier not in pinned roster")
    verifier = verifiers[0]
    Authority = combined_authority(quorum.binding)
    with tempfile.TemporaryDirectory(prefix=".sc68-restore-",dir=destination.parent) as temp:
        temp = Path(temp)
        checkpoint = temp/"certified.bin"
        _restoration_status(quorum,head,verifier)
        copied = quorum.auth(1,"recovery-copy","--expected-world",quorum.world,
                             "--expected-head",head["head"],"--output",checkpoint)
        raw = checkpoint.read_bytes()
        require(copied == {"classification":"ownerless_reference_recovery_copy_only",
                           "world_id":quorum.world,"head":head["head"],"checkpoint":head["checkpoint"],
                           "bytes":len(raw),"runtime_activation":False,"write_authority":False}
                and 0 < len(raw) <= 1024*1024
                and quorum.state_hash(checkpoint) == head["checkpoint"],"restoration checkpoint commitment differs")
        original = Authority(source,read_only=True)
        try:
            # Hold a consistent source read transaction through publication.
            # The caller still owns the retirement/process-lifecycle check.
            with original.transaction():
                current = original._read()
                require(restoration_source_pin(current)==expected_source,"restoration source pin changed")
                genesis,state,tick,commits,epochs,batches,commands = _certified_prefix(original,current,raw,mapping)
                require(tick < current["tick"],"source has not advanced beyond certified checkpoint")
                generation = current["generation"]+1
                accepted = commits[-1]
                grants = new_grants(current["world"],generation,tick,accepted[2],[FULL])
                old_sessions = {g["session"] for _,_,_,blob,_ in original.db.execute("SELECT * FROM epochs")
                                for g in json.loads(blob)}
                require(all(g["session"] not in old_sessions for g in grants),"restoration reused a worker session")
                candidate_path = temp/"restored.db"
                restored = Authority.create(candidate_path,current["world"],genesis,[0,1],[FULL])
                try:
                    with restored.transaction(write=True):
                        restored.db.execute("DELETE FROM commits")
                        restored.db.execute("DELETE FROM epochs")
                        restored.db.executemany("INSERT INTO commits VALUES(?,?,?,?,?,?)",commits)
                        restored.db.executemany("INSERT INTO epochs VALUES(?,?,?,?,?)",epochs)
                        restored.db.executemany("INSERT INTO batches VALUES(?,?,?,?)",batches)
                        restored.db.executemany("INSERT INTO commands VALUES(?,?,?,?,?)",commands)
                        # Consume the quarantine generation without any
                        # worker. An explicit future rotation is mandatory.
                        restored.db.execute("INSERT INTO epochs VALUES(?,?,?,?,1)",
                                            (generation,tick,accepted[2],canonical(public_grants(grants))))
                        restored.db.execute("""UPDATE world SET generation=?,accepted_generation=?,claimed=1,
                            tick=?,runtime_root=?,cut_root=?,state=?,grants=? WHERE singleton=1""",
                            (generation,accepted[1],tick,accepted[2],accepted[3],canonical(wire(state)),canonical(grants)))
                        restored._state_cache = None
                        require(canonical(committed_snapshot(restored,mapping,restored._read()))==raw,
                                "restored staging differs from certified bytes")
                finally:
                    restored.close()
                reopened = Authority(candidate_path,read_only=True)
                try:
                    result = reopened.save()
                    require(result["generation"]==generation and result["claimed"]
                            and canonical(observed_snapshot(reopened,list(mapping)))==raw,
                            "cold restored staging verification differs")
                finally:
                    reopened.close()
                _fsync_path(candidate_path)
                if before_publish is not None:
                    before_publish() # isolated failure/race injection only
                _restoration_status(quorum,head,verifier)
                vacant()
                # The temporary database is already complete and fsynced.
                # Link is atomic and refuses an existing destination.
                os.link(candidate_path,destination)
                _fsync_path(destination.parent,directory=True)
        finally:
            original.close()
    return {"classification":"ownerless_reference_staging_restored_quarantined",
            "source":expected_source.copy(),"world_id":quorum.world,"head":head["head"],
            "checkpoint":head["checkpoint"],"restored_tick":tick,"retired_generation":current["generation"],
            "quarantine_generation":generation,"discarded_ticks":current["tick"]-tick,
            "source_preserved":True,"rotation_required":True,"live_quorum_verified":False,
            "runtime_activation":False,"canonical_authority":False,"write_authority":False,
            "player_admission":False,"worker_admission":False}


class CertifiedClientGate:
    """Private reference-client boundary; no live authority or lease.

    The staging ledger remains internal. Every view, command admission
    and receipt lookup replays one consistent cut and checks its exact
    bytes against an independently pinned local signed validator journal.
    Connection identity proof is the caller's responsibility. A local
    journal check is not a distributed lock across the eventual TCP send.
    """
    def __init__(self, ledger, quorum, players):
        self.ledger = ledger
        self.quorum = quorum
        self.world = quorum.world
        self.binding = quorum.binding
        players = tuple(players)
        require(players and all(type(p) is int and p in (0, 1) for p in players)
                and len(set(players)) == len(players), "invalid reference client roster")
        self.players = frozenset(players)
        self.mapping = tuple(quorum.mapping)
        require(len(self.mapping) == len(set(self.mapping)) == 2 and all(digest(k) for k in self.mapping),
                "invalid reference player mapping")
        self.head = dict(quorum.head)
        self.parent = None
        current = ledger.read()
        self.tick = current["tick"]
        require(parts(self.binding) == (current["world"], self.head["policy_hash"], self.head["compatibility"])
                and self.world == "scownerless:" + current["world"] == self.head["world_id"],
                "reference client binding mismatch")
        verifiers = [v["peer_id"] for v in self.head["validators"] if v["public_key"] == quorum.keys[1]]
        require(len(verifiers) == 1, "reference verifier not in pinned roster")
        self.verifier = verifiers[0]

    def save(self):
        # Clients.spawn needs only the pinned world identifier, not a
        # staging save. All state-bearing methods below are guarded.
        return {"world": parts(self.binding)[0]}

    def _verify(self, current, player, head, parent):
        require(type(player) is int and player in self.players and player in current["players"],
                "reference client not authorized for this session")
        require(current["world"] == self.save()["world"], "reference client world changed")
        candidate = committed_snapshot(self.ledger, list(self.mapping), current)
        with tempfile.TemporaryDirectory(prefix="sc68-cut-check-") as tmp:
            path = Path(tmp)/"exact-cut.bin"
            path.write_bytes(canonical(candidate))
            args = ["verify-reference-cut", "--expected-world", self.world,
                    "--expected-head", head["head"], "--expected-fence", head["reference_fence"],
                    "--expected-policy", head["policy_hash"], "--expected-compatibility", head["compatibility"],
                    "--checkpoint", path, "--player-key", self.mapping[player]]
            if parent is not None:
                args += ["--expected-parent", parent]
            # Slot 1 is the explicitly selected local surviving validator
            # in this disposable reference fixture, not a founder privilege.
            result = self.quorum.auth(1, *args)
        require(result["classification"] == "verified_local_ownerless_reference_cut"
                and all(result[k] == head[k] for k in ("world_id", "head", "sequence", "reference_fence",
                                                       "checkpoint", "policy_hash", "compatibility"))
                and result["exact_reference_cut_verified"] is True
                and result["checkpoint_transition_verified"] is (parent is not None)
                and result["expected_parent"] == parent and result["player_key"] == self.mapping[player]
                and result["reference_player_eligible"] is True and result["local_validator"] is True
                and result["local_identity"] == self.verifier
                and result["pending_proposal"] is None
                and all(result[k] is False for k in ("freshness_lease", "canonical_authority", "write_authority",
                    "live_quorum_verified", "runtime_activation", "player_admission", "worker_admission",
                    "v1_canonical_world_created")), "reference publication verification mismatch")
        return result, candidate

    def view(self, player):
        with self.ledger.transaction():
            current = self.ledger._read()
            require(current["tick"] == self.tick, "uncertified staged tick")
            self._verify(current, player, self.head, self.parent)
            return transition_view(current["world"], current["tick"], current["cut_root"],
                                   current["state"], current["players"], player)

    def submit(self, player, request):
        with self.ledger.transaction(write=True):
            current = self.ledger._read()
            require(current["tick"] == self.tick, "uncertified staged tick")
            self._verify(current, player, self.head, self.parent)
            return self.ledger._submit_locked(current, player, request)

    def committed_receipts(self, inputs):
        """Release only exact results from the certified, independently audited cut.

        Never enqueue on receipt lookup, even if a request was never seen.
        A later eligible session may read an exact historical receipt already
        included in the current certified checkpoint.
        """
        with self.ledger.transaction():
            current = self.ledger._read()
            require(current["tick"] == self.tick, "uncertified staged tick")
            rows = checked_batch(inputs, current)
            results = []
            for player, request in rows:
                _, candidate = self._verify(current, player, self.head, self.parent)
                matches = [row["response"] for row in candidate["transaction_journal"]["commands"]
                           if row["player"] == player and row["request"] == request]
                require(len(matches) == 1 and matches[0]["op"] == "committed",
                        "no exact certified command receipt")
                results.append(matches[0])
            return results

    def publish(self, accepted):
        # Only a direct same-fence checkpoint extension may change the
        # pin of an existing session. Governance changes require a new
        # session and newly verified role proofs.
        require(accepted["sequence"] == self.head["sequence"] + 1
                and accepted["head"] != self.head["head"]
                and all(accepted[k] == self.head[k] for k in ("world_id", "policy_hash", "compatibility",
                                                               "reference_fence")), "invalid reference publication transition")
        parent = self.head["head"]
        with self.ledger.transaction():
            current = self.ledger._read()
            require(current["tick"] == self.tick + 1, "reference publication must certify exactly the next tick")
            checks = [self._verify(current, p, accepted, parent)[0] for p in sorted(self.players)]
            tick = current["tick"]
        self.head, self.parent, self.tick = dict(accepted), parent, tick
        return checks


class Quorum:
    """Disposable reference CLI profiles. An organizer holds no identity."""
    def __init__(self, cli, root, manifest):
        self.cli = str(Path(cli).resolve())
        self.root = root
        self.organizer = root/"organizer"
        self.profiles = [root/f"peer-{i}" for i in range(6)]
        self.keys = []
        self.records = []
        self.active = [0,1,2]
        self.offline = set()
        for i in range(6):
            output = self.call(i, "identity", parse=False)
            match = re.search(r"^Public key: ([0-9a-f]{64})$", output.decode(), re.M)
            require(match is not None, "identity output missing key")
            self.keys.append(match[1])
        self.mapping = [self.keys[0], self.keys[5]]
        state = fixture()
        checkpoint = snapshot(self.mapping, state, 0, {"genesis": wire(state), "batches": [], "commands": []})
        self.checkpoint = self.write("checkpoint-0.bin", canonical(checkpoint))
        compatibility = compatibility_for(manifest)
        commitment = self.state_hash(self.checkpoint)
        flags = []
        for k in self.keys[:3]:
            flags.extend(["--validator-key", k])
        draft = self.call(None, "ownerless", "draft", "--name", "Combined reference world",
                          "--compatibility-fingerprint", compatibility,
                          "--initial-state-commitment", commitment, *flags)
        draft_path = self.write_json("draft.json", draft)
        inspection = self.call(None, "ownerless", "inspect", "--draft", draft_path)
        self.world = inspection["world_id"]
        approvals = []
        for i in range(3):
            approval = self.call(i, "ownerless", "approve", "--draft", draft_path,
                                 "--expected-world", self.world, "--confirm")
            approvals.extend(["--approval", self.write_json(f"approval-{i}.json", approval)])
        certificate = self.call(None, "ownerless", "assemble", "--draft", draft_path,
                                "--expected-world", self.world, *approvals)
        self.cert = self.write_json("creation.json", certificate)
        self.binding = verify_profile(self.cli, self.cert, self.world, compatibility, root/"absent-profile")
        history = self.auth(None, "genesis-history", "--certificate", self.cert, "--expected-world", self.world)
        self.history = self.write_json("history-0.json", history)
        self.head = self.inspect(self.history)
        require(self.head["checkpoint"] == commitment and self.head["sequence"] == 0, "wrong signed genesis")
        for i in self.active:
            status = self.auth(i, "init", "--history", self.history, *self.context(),
                               "--checkpoint", self.checkpoint)
            self.check_head(status)
        # The exact peer IDs are available from the signed roster and
        # later reference role proofs, not assumed from display names.
        self.records.append({"label": "genesis", "head": self.head, "checkpoint": checkpoint})
        require(not self.organizer.exists(), "organizer unexpectedly acquired storage")

    def write(self, name, data):
        p = self.root/name
        with p.open("xb") as f:
            f.write(data)
        return p

    def write_json(self, name, value):
        return self.write(name, canonical(value))

    def call(self, peer, *args, parse=True, reject=None):
        require(peer not in self.offline, "attempted use of permanently departed founder")
        profile = self.organizer if peer is None else self.profiles[peer]
        result = subprocess.run([self.cli, "--data-dir", str(profile), *map(str,args)],
                                stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=20)
        if reject is not None:
            require(result.returncode != 0 and reject in result.stderr.decode(errors="replace"),
                    "unexpected CLI refusal result: " + result.stderr.decode(errors="replace"))
            return reject
        require(result.returncode == 0, "CLI failed: " + result.stderr.decode(errors="replace"))
        return json.loads(result.stdout) if parse else result.stdout

    def peer_id(self, slot):
        matches = [v["peer_id"] for v in self.head["validators"] if v["public_key"] == self.keys[slot]]
        require(len(matches) == 1 and slot not in self.offline, "invalid active validator")
        return matches[0]

    @contextmanager
    def listener(self, slot, classification, *args, pins):
        """Bounded actual CLI process, exact readiness, no leaked listeners."""
        peer_id = self.peer_id(slot)
        command = [self.cli,"--data-dir",str(self.profiles[slot]),"ownerless","authority","decision",*map(str,args)]
        process = subprocess.Popen(command,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
        try:
            with selectors.DefaultSelector() as selector:
                selector.register(process.stdout,selectors.EVENT_READ)
                require(selector.select(timeout=10),"decision listener readiness timeout")
            line = process.stdout.readline(65537)
            require(0 < len(line) <= 65536 and line.endswith(b"\n"),"missing decision listener readiness")
            ready = json.loads(line)
            require(ready["classification"] == classification and ready["validator"] == peer_id
                    and ready["world_id"] == self.world and ready["runtime_activation"] is False
                    and all(ready[k] == v for k,v in pins.items()),"decision listener context differs")
            require(re.fullmatch(r"127\.0\.0\.1:[1-9][0-9]{0,4}",ready["address"]) is not None
                    and int(ready["address"].rsplit(":",1)[1]) <= 65535,"unexpected listener endpoint")
            yield ready
            out, err = process.communicate(timeout=15)
            require(process.returncode == 0 and not out,"decision listener failed: "+err.decode(errors="replace"))
        finally:
            if process.poll() is None:
                process.kill()
            process.communicate(timeout=5)

    def auth(self, peer, *args, **kw):
        return self.call(peer, "ownerless", "authority", *args, **kw)

    def state_hash(self, path):
        return self.auth(None, "state-hash", "--checkpoint", path)["checkpoint"]

    def inspect(self, history):
        return self.auth(None, "inspect", "--history", history, "--expected-world", self.world)

    def context(self):
        return ["--expected-world", self.world, "--expected-head", self.head["head"]]

    def check_head(self, observed):
        for field in ("world_id","head","sequence","reference_fence","checkpoint","policy_hash","compatibility"):
            require(observed[field] == self.head[field], "replica differs from certified head")
        require(observed["runtime_activation"] is False and observed["player_admission"] is False
                and observed["live_quorum_verified"] is False, "reference head enabled live authority")

    def assert_state(self, checkpoint):
        require(canonical(checkpoint) == self.checkpoint.read_bytes()
                and self.state_hash(self.checkpoint) == self.head["checkpoint"], "local state is not the certified cut")
        for i in self.active:
            self.check_head(self.auth(i, "status", "--expected-world", self.world))

    def proposal(self, action):
        if action[0] == "participation":
            prefix = action[:2]
            rest = action[2:]
        else:
            prefix, rest = ["propose"], action
        return self.auth(None, *prefix, "--history", self.history, *self.context(), *rest)

    def certify(self, label, action, checkpoint=None, voters=None):
        if voters is None:
            voters = self.active[:2]
        require(len(set(voters)) == len(voters) and len(voters) >= 2
                and all(i in self.active for i in voters), "no live local signing majority")
        old = self.head
        if checkpoint is None:
            file = self.checkpoint
        else:
            file = self.write(f"checkpoint-{old['sequence']+1}.bin", canonical(checkpoint))
        commitment = self.state_hash(file)
        if action == ["checkpoint"]:
            action = ["--checkpoint", file]
        proposal = self.proposal(action)
        p = self.write_json(f"proposal-{old['sequence']+1}.json", proposal)
        votes = []
        for i in voters:
            v = self.auth(i, "vote", *self.context(), "--proposal", p, "--expected-proposal",
                          proposal["proposal_hash"], "--checkpoint", file, "--confirm")
            votes.extend(["--vote", self.write_json(f"vote-{old['sequence']+1}-{i}.json", v)])
        history = self.auth(None, "assemble", "--history", self.history, *self.context(),
                            "--proposal", p, "--expected-proposal", proposal["proposal_hash"], *votes)
        new = self.write_json(f"history-{old['sequence']+1}.json", history)
        accepted = self.inspect(new)
        require(accepted["sequence"] == old["sequence"]+1 and accepted["head"] == proposal["proposal_hash"]
                and accepted["checkpoint"] == commitment and accepted["world_id"] == self.world,
                "certified head differs from candidate")
        self.head, self.history, self.checkpoint = accepted, new, file
        for i in self.active:
            self.check_head(self.auth(i, "accept", "--history", new, *self.context(), "--checkpoint", file))
        self.records.append({"label": label, "head": accepted, "voter_slots": voters.copy(),
                             "checkpoint": json.loads(file.read_bytes())})
        return accepted

    def certify_network_checkpoint(self,label,candidate,*,before_retry=None,before_sync=None):
        """Exact reviewed encrypted collection, followed by durable sync.

        Never falls back to offline votes or acceptance after a failure.
        The fixture owns all profiles; this is not independent operator trust.
        """
        old = self.head
        seq = old["sequence"]+1
        voters = self.active[:2]
        collector = 1
        require(collector in voters and len(voters)==2,"no configured reference collector majority")
        remote, = [i for i in voters if i != collector]
        file = self.write(f"checkpoint-{seq}.bin",canonical(candidate))
        commitment = self.state_hash(file)
        proposal = self.proposal(["--checkpoint",file])
        target = proposal["proposal_hash"]
        p = self.write_json(f"proposal-{seq}.json",proposal)
        # Each validator reads its OWN durable prior bytes and accepted
        # history. Derive the reviewed proposal independently from these.
        reviews = []
        for slot in voters:
            copied = self.root/f"review-{seq}-{slot}.bin"
            copy = self.auth(slot,"recovery-copy",*self.context(),"--output",copied)
            require(copy["checkpoint"] == old["checkpoint"] and copy["write_authority"] is False
                    and copied.read_bytes() == self.checkpoint.read_bytes(),"review local certified base differs")
            root = review_checkpoint(self.binding,self.mapping,json.loads(copied.read_bytes()),
                                     json.loads(file.read_bytes()))
            history = self.auth(slot,"export-history","--expected-world",self.world)
            local_history = self.write_json(f"review-history-{seq}-{slot}.json",history)
            local_review = self.auth(None,"propose","--history",local_history,*self.context(),"--checkpoint",file)
            require(canonical(local_review) == canonical(proposal),"validator review differs")
            self.write_json(f"review-proposal-{seq}-{slot}.json",local_review)
            reviews.append({"slot":slot,"base":old["head"],"proposal":target,
                            "base_checkpoint":old["checkpoint"],"candidate_checkpoint":commitment,
                            "replayed_root":root})
        review = [*self.context(),"--proposal",p,"--expected-proposal",target,"--confirm"]
        network = {"collector_slot":collector,"voter_slots":voters,"reviews":reviews,
                   "vote_listener":None,"sync":[]}
        if before_retry is not None:
            # Keep a private port bound but not listening so no remote
            # vote can be obtained. The exact candidate is retained.
            with socket.socket(socket.AF_INET,socket.SOCK_STREAM) as unavailable:
                unavailable.bind(("127.0.0.1",0))
                addr = "127.0.0.1:"+str(unavailable.getsockname()[1])
                failure = self.auth(collector,"decision","collect",*review,"--checkpoint",file,
                                    "--peer",self.peer_id(remote)+"@"+addr,
                                    reject="missing current validator majority")
            locked = self.auth(collector,"status","--expected-world",self.world)
            self.check_head(locked)
            require(locked["pending_proposal"] == target,"failed collection lost exact vote lock")
            unchanged = self.auth(remote,"status","--expected-world",self.world)
            self.check_head(unchanged)
            require(unchanged["pending_proposal"] is None,"unreachable voter changed")
            before_retry()
            network["minority_attempt"] = {"refusal":failure,"collector_status":locked,
                                           "remote_status":unchanged,"client_access_refused":True}
        remote_review = [*self.context(),"--proposal",self.root/f"review-proposal-{seq}-{remote}.json",
                         "--expected-proposal",target,"--confirm"]
        with self.listener(remote,"ownerless_reference_decision_listener","serve-vote",*remote_review,
                           "--from",self.peer_id(collector),"--bind","127.0.0.1:0",
                           pins={"proposal":target}) as ready:
            history = self.auth(collector,"decision","collect",*review,"--checkpoint",file,
                                "--peer",self.peer_id(remote)+"@"+ready["address"])
        network["vote_listener"] = ready
        new = self.write_json(f"history-{seq}.json",history)
        accepted = self.inspect(new)
        require(accepted["sequence"] == seq and accepted["head"] == target
                and accepted["checkpoint"] == commitment and accepted["world_id"] == self.world
                and accepted["reference_fence"] == old["reference_fence"],"network certified cut differs")
        # The collector already committed; peers have not necessarily
        # accepted. Do not move the client-facing pin before reconciliation.
        local = self.auth(collector,"status","--expected-world",self.world)
        require(local["head"] == target and local["pending_proposal"] is None,"collector did not durably accept")
        if before_sync is not None:
            before_sync()
            network["pre_sync_client_access_refused"] = True
        sync = ["--expected-world",self.world,"--expected-base",old["head"],"--expected-head",target,"--confirm"]
        for slot in self.active:
            if slot == collector:
                continue
            with self.listener(slot,"ownerless_reference_sync_listener","serve-sync",*sync,
                               "--from",self.peer_id(collector),"--bind","127.0.0.1:0",
                               pins={"head":target}) as ready:
                result = self.auth(collector,"decision","push-sync",*sync,
                                   "--peer",self.peer_id(slot)+"@"+ready["address"])
            require(result["classification"] == "ownerless_reference_checkpoint_synced"
                    and all(result[k] is False for k in ("live_quorum_verified","runtime_activation","write_authority",
                        "canonical_authority","player_admission","worker_admission")),
                    "sync receipt claimed live authority")
            network["sync"].append({"recipient_slot":slot,"listener":ready,"result":result})
        self.head,self.history,self.checkpoint = accepted,new,file
        for slot in self.active:
            status = self.auth(slot,"status","--expected-world",self.world)
            self.check_head(status)
            require(status["pending_proposal"] is None,"replica retains unresolved vote")
        self.records.append({"label":label,"head":accepted,"voter_slots":voters.copy(),
                             "checkpoint":json.loads(file.read_bytes()),"network":network})
        return accepted

    def player(self, label, subject, allow=True):
        return self.certify(label, ["participation","propose-player","--player-key",self.keys[subject],
                                   "--allow" if allow else "--revoke"])

    def worker(self, label, subject, revoke=False):
        action = ["participation","propose-worker","--worker-key",self.keys[subject]]
        action += ["--revoke"] if revoke else ["--memory-mib","512","--cpu-millicores","1000"]
        return self.certify(label, action)

    def role(self, label, subject, cpu=None, reject=None):
        intent = ["--play"] if cpu is None else ["--memory-mib","512","--cpu-millicores",str(cpu)]
        request = self.auth(1, "participation","challenge",*self.context(),"--subject-key",self.keys[subject],*intent)
        rq = self.write_json(f"{label}-request.json", request)
        confirmation = [] if cpu is None else ["--confirm-volunteer"]
        response = self.auth(subject, "participation","respond","--history",self.history,*self.context(),
                             "--request",rq,"--expected-challenge",request["challenge_hash"],"--confirm",*confirmation)
        rp = self.write_json(f"{label}-response.json", response)
        observed = self.auth(1,"participation","verify",*self.context(),
                             "--expected-challenge",request["challenge_hash"],"--response",rp,reject=reject)
        if reject is None:
            require(observed["head"] == self.head["head"] and
                    observed["role"] == ("reference_player" if cpu is None else "reference_volunteer")
                    and all(observed[f] is False for f in FALSE_FLAGS), "role proof granted live authority")
            if cpu is not None:
                require(observed["offered_budget"] == {"memory_mib":512,"cpu_millicores":cpu},
                        "explicit volunteer cap differs")
        return {"label":label,"subject_slot":subject,"request":request,"response":response,"result":observed}

    def depart(self):
        path = self.profiles[0]/"ownerless-authority-lab-v1"/parts(self.binding)[0]/"journal.json"
        self.founder_path = path
        self.founder_digest = sha(path.read_bytes())
        self.offline.add(0)
        self.active = [1,2]

    def minority(self):
        proposal = self.proposal(["--recover"])
        p = self.write_json("minority-proposal.json",proposal)
        vote = self.auth(1,"vote",*self.context(),"--proposal",p,"--expected-proposal",proposal["proposal_hash"],
                         "--checkpoint",self.checkpoint,"--confirm")
        v = self.write_json("minority-vote.json",vote)
        refusal = self.auth(None,"assemble","--history",self.history,*self.context(),"--proposal",p,
                            "--expected-proposal",proposal["proposal_hash"],"--vote",v,
                            reject="missing current validator majority")
        copy = self.root/"minority-recovery.bin"
        result = self.auth(1,"recovery-copy",*self.context(),"--output",copy)
        require(copy.read_bytes() == self.checkpoint.read_bytes() and result["write_authority"] is False,
                "minority recovery copy changed state or granted authority")
        self.check_head(self.auth(1,"status","--expected-world",self.world))
        return {"refusal":refusal,"proposal":proposal,"vote":vote,
                "copy_sha256":sha(copy.read_bytes()),"copy_report":result}


def run(cli, scope, cpus, temp, report):
    quorum = Quorum(cli, temp, report["artifact_manifest"])
    report.update(world_id=quorum.world, binding=quorum.binding, participant_keys=quorum.keys)
    quorum.player("invite_founder",0)
    quorum.player("invite_surviving_player",5)
    quorum.worker("allow_founder_worker",0)
    quorum.worker("allow_surviving_worker",3)
    quorum.worker("allow_extra_worker",4)
    Authority = combined_authority(quorum.binding)
    db = temp/"combined.db"
    ledger = Authority.create(db, parts(quorum.binding)[0], fixture(), [0,1], [FULL])
    stages = report["stages"]
    retired_receipts = None

    def stage(label, offers, groups, players, direction, action):
        nonlocal retired_receipts
        require(not scope.active and not scope.poisoned, "previous worker generation not released")
        quorum.assert_state(observed_snapshot(ledger, quorum.mapping))
        before = ledger.save()
        evidence = {"label":label, "base_head":quorum.head["head"], "before":before,
                    "worker_proofs":[], "player_proofs":[], "workers":[]}
        for offer,subject in offers:
            proof = quorum.role(f"{label}-worker-{subject}", subject, offer.cpu_millicores)
            # #58 synthetic caps are deliberately smaller than the signed
            # positive reference offer. Never exceed either signed limit.
            require(offer.process_memory_mib <= proof["result"]["offered_budget"]["memory_mib"],
                    "local cap exceeds signed consent")
            evidence["worker_proofs"].append(proof)
        for player,subject in players:
            require(quorum.mapping[player] == quorum.keys[subject], "reference player mapping differs")
            evidence["player_proofs"].append(quorum.role(f"{label}-player-{player}",subject))
        if stages:
            ledger.rotate(before["generation"], before["tick"], before["runtime_root"], groups)
        with CappedPolicyLab(scope,[o for o,_ in offers],cpus,quorum.binding) as lab:
            samples, probes = lab.probe()
            decision_time = time.monotonic_ns()
            require(all(assess(o,samples[o.slot],decision_time) == "eligible" for o,_ in offers),
                    "worker capacity not admitted")
            owner = [next(o.slot for (o,_),g in zip(offers,groups) if r in g) for r in FULL]
            plan = evaluate(owner,region_load(ledger.read()["state"]),samples)
            require(plan is not None, "fixed local placement exceeds measured capacity")
            evidence.update(plan=plan, probes=probes, decision_time_ns=decision_time,
                            worker_pids=[p.pid for p in lab.processes], worker_namespaces=lab.namespaces,
                            coordinator_namespace=lab.parent_ns, generation=ledger.save()["generation"])
            runtime = DurableRuntime(ledger,[PolicyChannel(c,quorum.binding) for c in lab.channels])
            if retired_receipts is not None:
                prior = ledger.save()
                try:
                    ledger.commit([retired_receipts[i % len(retired_receipts)] for i in range(len(offers))])
                except Rejected as error:
                    require(str(error) == "stale ownerless receipt session", "wrong stale receipt refusal")
                    evidence["stale_receipt_refusal"] = str(error)
                else:
                    raise Rejected("old worker receipts unexpectedly accepted")
                require(ledger.save() == prior, "old receipt changed staging cut")
            for channel in lab.channels:
                require(rpc(channel,{"op":"grant_vote"})["op"] == "refused", "worker acquired governance")
                require(rpc(channel,{"op":"op"})["op"] == "refused", "worker acquired operator power")
            evidence["workers"] = [{"slot":o.slot,"subject_slot":subject,"memory_mib":o.process_memory_mib,
                                   "cpu_millicores":o.cpu_millicores,"revision":o.revision,"regions":groups[i]}
                                  for i,(o,subject) in enumerate(offers)]
            evidence["pre_kernel"] = [lab.observe(i) for i in range(len(offers))]
            scope.recheck(evidence["pre_kernel"])
            gate = CertifiedClientGate(ledger, quorum, [p for p,_ in players])
            # This bounded reference ACK wait includes certificate
            # collection and dissemination; not a tick service target.
            with Clients(gate,ack_window=60) as clients:
                for player,_ in players:
                    request = {"op":"act","world":before["world"],"id":secrets.token_hex(32),
                               "tick":before["tick"]+1,"parent":before["cut_root"],
                               "direction":direction,"action":action}
                    clients.spawn(player,request)
                require(all(item["submission"]["op"] == "queued" for item in clients.sessions),
                        "unexpected unsealed input result")
                runtime.advance()
                # Prove the actual client boundary refuses a local FULL
                # commit until the independent authority accepts it.
                def refuse_staged():
                    for item in clients.sessions:
                        for attempt in (lambda: gate.view(item["player"]),
                                        lambda: gate.submit(item["player"], item["request"]),
                                        lambda: gate.committed_receipts([(item["player"], item["request"])])):
                            try:
                                attempt()
                            except Rejected as error:
                                require(str(error) == "uncertified staged tick", "wrong publication refusal")
                            else:
                                raise Rejected("uncertified client data escaped")
                refuse_staged()
                evidence["uncertified_client_access_refused"] = True
                evidence["post_kernel"] = [lab.observe(i) for i in range(len(offers))]
                scope.recheck(evidence["post_kernel"])
                retired_receipts = candidate_receipts(runtime.runtime.coordinator)
                candidate = observed_snapshot(ledger,quorum.mapping)
                # Workers have completed tentative execution. Retire
                # them before the slow governance path instead of
                # keeping idle computation sessions alive or extending
                # their deadlines. No further tick may use these grants.
                lab.close()
                require(not scope.active and not scope.poisoned
                        and all(p.returncode == 0 for p in lab.processes),
                        "workers not retired before checkpoint certification")
                runtime.runtime.frozen = runtime.runtime.coordinator.poisoned = True
                evidence["workers_retired_before_certification"] = True
                # The transaction ledger is local staging, NOT a global head.
                # Only the exact replay-checked checkpoint certified by the
                # surviving majority may be acknowledged to clients.
                prior_head = quorum.head["head"]
                require(quorum.state_hash(quorum.checkpoint) == quorum.head["checkpoint"],
                        "previous certified bytes missing")
                accepted = quorum.certify_network_checkpoint(label+"-checkpoint",candidate,
                    before_retry=refuse_staged if label=="surviving_quorum_scale_out" else None,
                    before_sync=refuse_staged)
                require(accepted["head"] != prior_head, "no certified extension")
                quorum.assert_state(candidate)
                evidence["accepted_head"] = accepted["head"]
                evidence["publication_checks"] = gate.publish(accepted)
                evidence["clients"] = clients.finish()
                require(all(row["observed"]["reply"]["op"] == "committed" for row in evidence["clients"]),
                        "client acknowledgement did not follow certification")
            if label == "surviving_quorum_scale_out":
                view = evidence["clients"][0]["observed"]
                old_region, new_region = view["joined"]["region"], view["reply"]["view"]["region"]
                require(old_region == 0 and new_region == 7 and owner[old_region] != owner[new_region],
                        "surviving player did not cross distinct workers")
                evidence["player_cross_worker"] = [owner[old_region], owner[new_region]]
            evidence["after"] = ledger.save()
            evidence["all_worker_privilege_requests_refused"] = True
        require(not scope.active and all(p.returncode == 0 for p in lab.processes)
                and {n for _,n in socket.if_nameindex()} == {"lo"}, "worker retirement incomplete")
        evidence["worker_cleanup"] = True
        stages.append(evidence)
        print(label+": tick "+str(evidence["after"]["tick"])+", workers "+str(len(offers))+
              ", certified before client ack",flush=True)

    try:
        stage("founder",[(Offer(0,1,96,1000),0)],[FULL],[(0,0),(1,5)],1,1)
        quorum.depart()
        quorum.player("remove_founder_invitation",0,False)
        quorum.worker("retire_founder_compute",0,True)
        quorum.certify("founder_independent_recovery",["--recover"])
        # A fresh coordinator reopens the same durable staging world; the
        # signed checkpoint bytes are compared before any new client view.
        ledger.close()
        ledger = Authority(db)
        stage("surviving_quorum_scale_out",[(Offer(1,1,96,1000),3),(Offer(2,1,96,1000),4)],
              HALVES,[(1,5)],-1,-1)
        quorum.worker("withdraw_extra_worker",4,True)
        report["withdrawal_refusal"] = quorum.role("withdrawn-worker-probe",4,1000,
                                                    reject="worker has no accepted eligibility")
        quorum.certify("scale_in_recovery",["--recover"])
        ledger.close()
        ledger = Authority(db)
        stage("surviving_quorum_scale_in",[(Offer(1,2,80,750),3)],[FULL],[(1,5)],1,1)
        before = ledger.save()
        quorum.assert_state(observed_snapshot(ledger,quorum.mapping))
        report["minority"] = quorum.minority()
        require(ledger.save() == before and not scope.active, "minority changed staged world or launched workers")
        report["founder_journal_unchanged_after_departure"] = sha(quorum.founder_path.read_bytes()) == quorum.founder_digest
        require(report["founder_journal_unchanged_after_departure"], "founder journal changed after departure")
        report["history"] = json.loads(quorum.history.read_bytes())
        report["certified_cuts"] = quorum.records
        report["final_head"] = quorum.head
        report["final_tick"] = before["tick"]
        report["all_workers_exited"] = True
        for profile in quorum.profiles:
            require(not any((profile/"worlds").iterdir()) and not (profile/"control").exists()
                    and not (profile/"runtime").exists(), "lab created live V1 authority")
        require(not quorum.organizer.exists(), "organizer profile created")
    finally:
        ledger.close()


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--cli",type=Path,required=True)
    p.add_argument("--output",type=Path,required=True)
    a = p.parse_args()
    require(not a.output.exists(),"new receipt path required")
    safe_namespace()
    os.umask(0o077)
    coordinator,cpus,affinity = cpu_plan()
    scope = Scope(affinity)
    os.sched_setaffinity(0,{coordinator})
    report = {"schema":3,"classification":CLASSIFICATION,"physical_machines":1,"live_minecraft_players":0,
              "canonical_minecraft_authority":False,"runtime_activation":False,"actual_kernel_caps":True,
              "artifact_manifest":artifact_manifest(a.cli),"stages":[]}
    try:
        with tempfile.TemporaryDirectory(prefix="sc68-combined-") as temp:
            run(a.cli,scope,cpus,Path(temp),report)
        with a.output.open("x") as out:
            json.dump(report,out,separators=(",",":"),sort_keys=True,allow_nan=False)
            out.write("\n")
        print(json.dumps({"ticks":report["final_tick"],"decisions":len(report["certified_cuts"])-1,
                          "stages":len(report["stages"]),"minority_refused":True}))
    finally:
        os.sched_setaffinity(0,affinity)


if __name__ == "__main__":
    main()
