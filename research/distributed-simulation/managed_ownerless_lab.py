#!/usr/bin/env python3
"""Isolated real-process consent withdrawal, cleanup, restart and certification."""
import argparse
from dataclasses import asdict
import json
import os
from pathlib import Path
import secrets
import socket
import tempfile
import time

from certified_restore_lab import refuse
from combined_ownerless_lab import (CappedPolicyLab, CertifiedClientGate, FULL, Quorum,
                                    artifact_manifest, observed_snapshot, sha)
from managed_ownerless import certify_managed_checkpoint, managed_authority
from network_lab import cpu_plan, safe_namespace
from ownerless_permission import PolicyChannel, parts
from placement_lab import Scope, values
from player_runtime import fixture
from region_runtime import Rejected, canonical, require
from regional_authority import candidate_receipts
from transition_lab import Clients
from transition_runtime import DurableRuntime
from worker_placement import Offer, choose, region_load

CLASSIFICATION = "LOCAL_MANAGED_OWNERLESS_CONTRIBUTION_NOT_MINECRAFT_NOT_MULTI_HOST"
EXTRA_ARTIFACTS = ("managed_ownerless.py","managed_ownerless_lab.py","recovery_runtime.py","certified_restore_lab.py")
CERTIFICATION_ARTIFACTS = ("managed_certification.py","verify_combined_ownerless_evidence.py")
SUBJECTS = (3,4,None,None)


def choices(revision,a,b,cpu=1000):
    return [Offer(0,revision,a,cpu if a else 0),Offer(1,revision,b,cpu if b else 0),
            Offer(2,1,0,0),Offer(3,1,0,0)]


def certifications(ledger):
    return [{"scope":scope,"plan":json.loads(plan),"completion":json.loads(done) if done is not None else None}
            for scope,plan,done in ledger.db.execute("SELECT * FROM managed_certifications ORDER BY scope")]


def run(cli,scope,cpus,root,report):
    q = Quorum(cli,root,report["artifact_manifest"])
    q.player("invite_survivor",5)
    q.worker("allow_primary",3)
    q.worker("allow_extra",4)
    q.depart()
    q.certify("surviving_recovery",["--recover"])
    Authority = managed_authority(q,SUBJECTS)
    path = root/"managed.db"
    ledger = Authority.create(path,parts(q.binding)[0],fixture(),[0,1],[FULL])
    report.update(world_id=q.world,binding=q.binding,participant_keys=q.keys,slot_subjects=list(SUBJECTS))
    stages = []
    pending = None
    old_receipts = None

    def stage(label,expected,withdraw=False):
        nonlocal pending,old_receipts
        require(not scope.active and not scope.poisoned,"old worker scope not released")
        before = ledger.save()
        q.assert_state(observed_snapshot(ledger,q.mapping))
        # Player authentication is separate from resource consent.
        player_proof = q.role(label+"-player",5)
        positive = ledger.begin_probe()
        probe = ledger.status()
        require(positive and probe["phase"]=="probing","managed probe missing")
        row = {"label":label,"before":before,"probe":probe,"player_proof":player_proof,"guards":[]}
        with CappedPolicyLab(scope,positive,cpus,q.binding) as lab:
            samples,measurements = lab.probe()
            decision_ns = time.monotonic_ns()
            chosen = choose([Offer(**o) for o in probe["offers"]],samples,
                            region_load(ledger.read()["state"]),decision_ns)
            selected = sorted(set(chosen["plan"]["owner_by_region"])) if chosen["plan"] else []
            require(selected==expected,"unexpected managed placement: "+str(chosen))
            retired = []
            for i,offer in enumerate(lab.offers):
                if offer.slot not in selected:
                    lab.retire(i)
                    retired.append(offer.slot)
            require(ledger.admit(probe["scope"],samples,retired,decision_ns)==chosen,"durable placement differs")
            indices = [i for i,o in enumerate(lab.offers) if o.slot in selected]
            row.update(decision=chosen,decision_ns=decision_ns,probes=measurements,
                       workers=[{"slot":o.slot,"pid":p.pid,"namespace":ns} for o,p,ns in
                                zip(lab.offers,lab.processes,lab.namespaces)],machine=lab.machine,
                       ready=ledger.status())
            def guard(state,slots):
                require(list(slots)==selected,"managed guard slot mismatch")
                observations = [lab.observe(i) for i in indices]
                require(all(o["effective_cpu_millicores"]>=samples[s].effective_cpu_millicores
                            for s,o in zip(selected,observations)),"managed CPU capacity reduced")
                scope.recheck(observations)
                row["guards"].append({"slots":list(slots),"kernel":observations})
                return True
            ledger.attach_resource_guard(probe["scope"],guard)
            runtime = DurableRuntime(ledger,[PolicyChannel(lab.channels[i],q.binding) for i in indices])
            row["claimed"] = ledger.status()
            if old_receipts is not None:
                # New generation has exactly one selected worker.
                row["stale_receipt_refusal"] = refuse(lambda:ledger.commit(old_receipts[:len(selected)]),
                                                       "stale ownerless receipt session")
            gate = CertifiedClientGate(ledger,q,[1])
            with Clients(gate,ack_window=60) as clients:
                req = pending or {"op":"act","world":before["world"],"id":secrets.token_hex(32),
                                  "tick":before["tick"]+1,"parent":before["cut_root"],
                                  "direction":-1 if withdraw else 1,"action":-1 if withdraw else 1}
                clients.spawn(1,req)
                require(clients.sessions[0]["submission"]["op"]=="queued","managed request not queued")
                if withdraw:
                    pending = req
                    # Prepare actual authenticated worker receipts first.
                    controls = runtime.seal()
                    runtime.runtime.coordinator.step(controls=controls)
                    old_receipts = candidate_receipts(runtime.runtime.coordinator)
                    row["tentative_root"] = runtime.runtime.coordinator.root
                    # Concurrent connection changes consent before the
                    # ledger's candidate commit. No worker or client can
                    # promote this abandoned candidate.
                    other = Authority(path)
                    try:
                        other.configure(choices(3,80,0,750))
                    finally:
                        other.close()
                    row["withdrawal_refusal"] = refuse(lambda:ledger.commit(old_receipts),"contribution scope fenced")
                    row["seal_refusal"] = refuse(runtime.seal,"retired runtime cannot seal input")
                    row["overlap_refusal"] = refuse(ledger.begin_probe,"previous scope not released")
                    row["receipt_refusal"] = refuse(lambda:gate.committed_receipts([(1,req)]),
                                                   "no exact certified command receipt")
                    require(ledger.save()["tick"]==before["tick"] and ledger.audit()["batches"][-1]["result"] is None,
                            "withdrawn worker published staged result")
                    row["client"] = clients.finish(lose_ack=True)[0]
                else:
                    runtime.advance()
                    row["uncertified_refusal"] = refuse(lambda:gate.committed_receipts([(1,req)]),"uncertified staged tick")
                    row["cleanup_refusal"] = refuse(ledger.certification_candidate,"managed scope cleanup not released")
                row["draining"] = ledger.status()
                require(row["draining"]["phase"]=="draining","scope not fenced before cleanup")
                runtime.runtime.frozen = runtime.runtime.coordinator.poisoned = True
                leaves = list(lab.leaves)
                lab.close()
                require(not scope.active and not scope.poisoned and all(p.returncode==0 for p in lab.processes)
                        and all(not leaf.exists() for leaf in leaves)
                        and {n for _,n in socket.if_nameindex()}=={"lo"},"managed process cleanup incomplete")
                row["cleanup"] = [{"slot":o.slot,"pid":p.pid,"exitcode":p.returncode,"leaf_removed":not leaf.exists()}
                                  for o,p,leaf in zip(lab.offers,lab.processes,leaves)]
                d = ledger.status()
                ledger.release(d["scope"],d["generation"],d["probes"])
                row["released"] = ledger.status()
                row["scope_record"] = ledger.scope_record()
                if not withdraw:
                    row["candidate"] = ledger.certification_candidate()
                    def pre_sync():
                        refuse(lambda:gate.committed_receipts([(1,req)]),"uncertified staged tick")
                    accepted,checks = certify_managed_checkpoint(ledger,q,gate,label+"-checkpoint",before_sync=pre_sync)
                    row["accepted_head"] = accepted["head"]
                    row["publication_checks"] = checks
                    row["client"] = clients.finish()[0]
                    pending = None
                else:
                    row["certification_refusal"] = refuse(ledger.certification_candidate,"no released managed staged cut")
            row["after"] = ledger.save()
            row["journal"] = ledger.audit()
        stages.append(row)
        print(label+": tick "+str(row["after"]["tick"])+", scope "+str(probe["scope"])+", workers "+str(len(selected)),flush=True)

    try:
        require(ledger.begin_probe()==[] and not scope.active,"zero consent launched worker")
        ledger.configure(choices(2,56,56))
        stage("first", [0,1])
        stage("withdrawal", [0,1],withdraw=True)
        saved = (ledger.save(),ledger.status(),ledger.audit(),ledger.scope_record(),ledger.events())
        ledger.close()
        ledger = Authority(path)
        require(saved==(ledger.save(),ledger.status(),ledger.audit(),ledger.scope_record(),ledger.events()),
                "managed reopen changed persisted state")
        q.worker("revoke_extra",4,True)
        # Attempting to opt a revoked subject back in cannot create a probe.
        ledger.configure(choices(4,80,56,750))
        row = ledger.status()
        try:
            ledger.begin_probe()
        except Rejected as error:
            require("worker has no accepted eligibility" in str(error),"unexpected role refusal")
        else:
            raise RuntimeError("revoked volunteer acquired probe")
        require(ledger.status()==row and not scope.active,"rejected role created scope")
        ledger.configure(choices(5,80,0,750))
        stage("recovered",[0])
        report.update(stages=stages,events=ledger.events(),final=ledger.save(),final_journal=ledger.audit(),
                      certifications=certifications(ledger),
                      certified_cuts=q.records,history=json.loads(q.history.read_bytes()),
                      founder_journal_unchanged=sha(q.founder_path.read_bytes())==q.founder_digest,
                      all_scopes_released=not scope.active and not scope.poisoned,
                      reopened_equal=True,zero_consent_no_spawn=True,revoked_role_refused=True)
        require(report["founder_journal_unchanged"] and report["all_scopes_released"],"cleanup or founder pin differs")
    finally:
        ledger.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli",type=Path,required=True)
    parser.add_argument("--output",type=Path,required=True)
    args = parser.parse_args()
    require(not args.output.exists(),"new receipt path required")
    safe_namespace()
    os.umask(0o077)
    coordinator,cpus,affinity = cpu_plan()
    scope = Scope(affinity)
    os.sched_setaffinity(0,{coordinator})
    manifest = artifact_manifest(args.cli)
    manifest.update({name:sha((Path(__file__).parent/name).read_bytes()) for name in (*EXTRA_ARTIFACTS,*CERTIFICATION_ARTIFACTS)})
    report = {"schema":2,"classification":CLASSIFICATION,"physical_machines":1,"live_minecraft_players":0,
              "runtime_activation":False,"canonical_minecraft_authority":False,"actual_kernel_caps":True,
              "artifact_manifest":manifest}
    try:
        with tempfile.TemporaryDirectory(prefix="sc68-managed-") as temp:
            run(args.cli,scope,cpus,Path(temp),report)
        with args.output.open("x") as f:
            f.write(canonical(report).decode()+"\n")
        print(json.dumps({"certified_ticks":2,"withdrawn_ticks":1,"worker_processes":5,"scopes":3}))
    finally:
        os.sched_setaffinity(0,affinity)


if __name__=="__main__":
    main()
