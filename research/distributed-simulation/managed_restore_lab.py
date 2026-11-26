#!/usr/bin/env python3
"""Actual isolated managed restoration and revoked-command abandonment."""
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
from managed_ownerless import certify_managed_checkpoint, managed_authority, restore_managed_staging
from managed_ownerless_lab import CERTIFICATION_ARTIFACTS, EXTRA_ARTIFACTS, SUBJECTS, choices, certifications
from network_lab import cpu_plan, safe_namespace
from ownerless_permission import PolicyChannel, parts
from placement_lab import Scope
from player_runtime import fixture
from region_runtime import canonical, require
from regional_authority import candidate_receipts
from transition_lab import Clients
from transition_runtime import DurableRuntime
from worker_placement import Offer, choose, region_load

CLASSIFICATION = "LOCAL_MANAGED_CERTIFIED_RESTORE_NOT_MINECRAFT_NOT_MULTI_HOST"
ARTIFACTS = (*EXTRA_ARTIFACTS,"managed_restore_lab.py")


def authorizations(ledger):
    return [list(row) for row in ledger.db.execute(
        "SELECT * FROM command_authorizations ORDER BY request_id")]


def run(cli,scope,cpus,root,report):
    q = Quorum(cli,root,report["artifact_manifest"])
    q.player("invite_survivor",5)
    q.worker("allow_worker",3)
    q.depart()
    q.certify("surviving_recovery",["--recover"])
    Authority = managed_authority(q,SUBJECTS)
    source = root/"managed.db"
    ledger = Authority.create(source,parts(q.binding)[0],fixture(),[0,1],[FULL])
    stages = []
    stale_receipts = None
    report.update(world_id=q.world,binding=q.binding,participant_keys=q.keys,slot_subjects=list(SUBJECTS))

    def step(label,ledger,direction,certify,*,block=False):
        nonlocal stale_receipts
        before = ledger.save()
        q.assert_state(observed_snapshot(ledger,q.mapping))
        player_proof = q.role(label+"-player",5)
        offers = ledger.begin_probe()
        status = ledger.status()
        require(len(offers)==1 and offers[0].slot==0,"managed restore probe differs")
        row = {"label":label,"before":before,"probe":status,"player_proof":player_proof,"guards":[]}
        with CappedPolicyLab(scope,offers,cpus,q.binding) as lab:
            samples,probes = lab.probe()
            decision_ns = time.monotonic_ns()
            decision = choose([Offer(**o) for o in status["offers"]],samples,
                              region_load(ledger.read()["state"]),decision_ns)
            require(decision["plan"] is not None and decision["plan"]["owner_by_region"]==[0]*8
                    and ledger.admit(status["scope"],samples,[],decision_ns)==decision,"managed restore admission differs")
            row.update(probes=probes,decision_ns=decision_ns,decision=decision,
                       worker_pid=lab.processes[0].pid,worker_namespace=lab.namespaces[0],
                       ready=ledger.status())
            def guard(state,slots):
                require(slots==(0,),"managed restore guard differs")
                obs = lab.observe(0)
                require(obs["effective_cpu_millicores"]>=samples[0].effective_cpu_millicores,"managed restore capacity reduced")
                scope.recheck([obs])
                row["guards"].append(obs)
                return True
            ledger.attach_resource_guard(status["scope"],guard)
            runtime = DurableRuntime(ledger,[PolicyChannel(lab.channels[0],q.binding)])
            if stale_receipts is not None:
                row["stale_receipt_refusal"] = refuse(lambda:ledger.commit(stale_receipts),"stale ownerless receipt session")
            gate = CertifiedClientGate(ledger,q,[1])
            with Clients(gate,ack_window=60) as clients:
                req = {"op":"act","world":before["world"],"id":secrets.token_hex(32),
                       "tick":before["tick"]+1,"parent":before["cut_root"],
                       "direction":direction,"action":direction}
                clients.spawn(1,req)
                require(clients.sessions[0]["submission"]["op"]=="queued","restore command not queued")
                runtime.advance()
                stale_receipts = candidate_receipts(runtime.runtime.coordinator)
                row["candidate"] = observed_snapshot(ledger,q.mapping)
                row["uncertified_refusal"] = refuse(lambda:gate.committed_receipts([(1,req)]),"uncertified staged tick")
                row["cleanup_refusal"] = refuse(ledger.certification_candidate,"managed scope cleanup not released")
                if block:
                    s = ledger.status()
                    ledger.fence(s["scope"],s["generation"],"policy",[0])
                runtime.runtime.frozen = runtime.runtime.coordinator.poisoned = True
                leaves = list(lab.leaves)
                lab.close()
                require(not scope.active and not scope.poisoned and lab.processes[0].returncode==0
                        and all(not leaf.exists() for leaf in leaves)
                        and {n for _,n in socket.if_nameindex()}=={"lo"},"managed restore cleanup failed")
                row["worker_retired"] = True
                s = ledger.status()
                ledger.release(s["scope"],s["generation"],s["probes"])
                row["released"] = ledger.status()
                row["scope_record"] = ledger.scope_record()
                if certify:
                    accepted,checks = certify_managed_checkpoint(ledger,q,gate,label+"-checkpoint",
                        before_sync=lambda:refuse(lambda:gate.committed_receipts([(1,req)]),"uncertified staged tick"))
                    row["publication_checks"] = checks
                    row["accepted_head"] = accepted["head"]
                    row["client"] = clients.finish()[0]
                else:
                    row["client"] = clients.finish(lose_ack=True)[0]
            row["after"] = ledger.save()
            row["journal"] = ledger.audit()
            row["authorizations"] = authorizations(ledger)
        stages.append(row)
        print(label+": tick "+str(row["after"]["tick"])+", generation "+str(row["after"]["generation"])+
              (", certified" if certify else ", not acknowledged"),flush=True)

    try:
        ledger.configure(choices(2,80,0,750))
        step("first",ledger,1,True)
        step("abandoned",ledger,1,False,block=True)
        # A withdrawal on an unrelated slot must not clear the failed
        # worker's blocked revision. Retain latest choices on restore.
        ledger.configure([Offer(0,2,80,750),Offer(1,3,0,0),Offer(2,1,0,0),Offer(3,1,0,0)])
        q.player("revoke_player",5,False)
        q.player("reinvite_player",5)
        report["certification_refusal"] = refuse(ledger.certification_candidate,"managed consent changed before certification")
        pin = ledger.restoration_pin()
        old_events = ledger.events()
        old_history = ledger.history()
        old_certifications = certifications(ledger)
    finally:
        ledger.close()
    source_hash = sha(source.read_bytes())
    destination = root/"restored.db"
    restoration = restore_managed_staging(q,SUBJECTS,source,destination,
                                          expected_pin=pin,source_retired=not scope.active and not scope.poisoned)
    restored = Authority(destination)
    try:
        quarantine = restored.save()
        status = restored.status()
        require(status["blocked"]==[0] and restored.begin_probe()==[],"restore lost failed-worker block")
        report["claim_refusal"] = refuse(lambda:restored.claim(quarantine["generation"],quarantine["tick"],
                                     quarantine["runtime_root"]),"generation already claimed; rotate before worker restart")
        report["stale_refusal"] = refuse(lambda:restored.commit(stale_receipts),"stale ownerless receipt session")
        q.role("restored-player",5)
        gate = CertifiedClientGate(restored,q,[1])
        with Clients(gate) as clients:
            clients.spawn(1,stages[0]["client"]["request"])
            replay = clients.finish()[0]
        old_request = stages[1]["client"]["request"]
        report["discarded_receipt_refusal"] = refuse(lambda:gate.committed_receipts([(1,old_request)]),
                                                     "no exact certified command receipt")
        report["abandoned_id_refusal"] = refuse(lambda:gate.submit(1,old_request),"abandoned managed command ID")
        require(restored.events()[:-1]==old_events,"restore lost lifecycle history")
        require(certifications(restored)==old_certifications,"restore lost certification provenance")
        report.update(restoration=restoration,source_pin=pin,source_sha256=source_hash,
                      source_history=old_history,source_events=old_events,
                      restored_history=restored.history(),restored_events=restored.events(),
                      restored_checkpoint=observed_snapshot(restored,q.mapping),quarantine=quarantine,
                      restored_status=status,restored_authorizations=authorizations(restored),
                      source_certifications=old_certifications,restored_certifications=certifications(restored),
                      recovered_client=replay)
        restored.configure(choices(4,80,0,750))
        step("replacement",restored,-1,True)
        report.update(final=restored.save(),final_journal=restored.audit(),final_events=restored.events(),
                      final_authorizations=authorizations(restored),certifications=certifications(restored))
    finally:
        restored.close()
    require(sha(source.read_bytes())==source_hash and sha(q.founder_path.read_bytes())==q.founder_digest,
            "source or departed founder changed")
    report.update(stages=stages,certified_cuts=q.records,history=json.loads(q.history.read_bytes()),
                  source_sha256_after=sha(source.read_bytes()),founder_journal_unchanged=True,
                  all_scopes_released=not scope.active and not scope.poisoned)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--cli",type=Path,required=True)
    p.add_argument("--output",type=Path,required=True)
    args = p.parse_args()
    require(not args.output.exists(),"new evidence path required")
    safe_namespace()
    os.umask(0o077)
    coordinator,cpus,affinity = cpu_plan()
    scope = Scope(affinity)
    os.sched_setaffinity(0,{coordinator})
    manifest = artifact_manifest(args.cli)
    manifest.update({name:sha((Path(__file__).parent/name).read_bytes()) for name in (*ARTIFACTS,*CERTIFICATION_ARTIFACTS)})
    report = {"schema":2,"classification":CLASSIFICATION,"physical_machines":1,"live_minecraft_players":0,
              "runtime_activation":False,"canonical_minecraft_authority":False,"actual_kernel_caps":True,
              "artifact_manifest":manifest}
    try:
        with tempfile.TemporaryDirectory(prefix="sc68-managed-restore-") as temp:
            run(args.cli,scope,cpus,Path(temp),report)
        with args.output.open("x") as out:
            out.write(canonical(report).decode()+"\n")
        print(json.dumps({"certified_ticks":2,"abandoned_ticks":1,"restored_tick":1,
                          "workers":3,"recovered_receipts":1,"tombstoned_commands":1}))
    finally:
        os.sched_setaffinity(0,affinity)


if __name__=="__main__":
    main()
