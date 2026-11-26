#!/usr/bin/env python3
"""#68 local combined reference session, NOT Minecraft/production authority.

Use disposable Linux user/network namespaces and a delegated cgroup scope.
The trusted controller stages a local transaction, obtains signed ownerless
checkpoint certification, and only then acknowledges reference clients.
No real-account login, independently owned machine, or live quorum lease.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import secrets
import socket
import subprocess
import tempfile
import time

from network_lab import cpu_plan, safe_namespace
from ownerless_permission import PolicyChannel, authority_for, parts, rpc, verify_profile
from placement_lab import CappedLab, Scope
from player_runtime import fixture, player_index
from region_runtime import Rejected, canonical, digest, region_roots, require, world_root
from regional_authority import candidate_receipts, wire
from transition_lab import Clients
from transition_runtime import DurableRuntime, TransitionAuthority, checked_batch, transition_view
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
            with Clients(gate) as clients:
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
                evidence["uncertified_client_access_refused"] = True
                evidence["post_kernel"] = [lab.observe(i) for i in range(len(offers))]
                scope.recheck(evidence["post_kernel"])
                retired_receipts = candidate_receipts(runtime.runtime.coordinator)
                candidate = observed_snapshot(ledger,quorum.mapping)
                # The transaction ledger is local staging, NOT a global head.
                # Only the exact replay-checked checkpoint certified by the
                # surviving majority may be acknowledged to clients.
                prior_head = quorum.head["head"]
                require(quorum.state_hash(quorum.checkpoint) == quorum.head["checkpoint"],
                        "previous certified bytes missing")
                accepted = quorum.certify(label+"-checkpoint",["checkpoint"],candidate)
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
    report = {"schema":2,"classification":CLASSIFICATION,"physical_machines":1,"live_minecraft_players":0,
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
