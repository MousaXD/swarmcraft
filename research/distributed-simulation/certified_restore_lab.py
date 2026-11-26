#!/usr/bin/env python3
"""Actual isolated TCP/process/cgroup restoration of an uncertified reference advance."""
import argparse
import json
import os
from pathlib import Path
import secrets
import socket
import tempfile
import time

from combined_ownerless_lab import (CappedPolicyLab, CertifiedClientGate, FULL, Quorum,
                                    artifact_manifest, combined_authority, observed_snapshot,
                                    restore_certified_staging, restoration_source_pin, sha)
from network_lab import cpu_plan, safe_namespace
from ownerless_permission import PolicyChannel, parts
from placement_lab import Scope
from player_runtime import fixture
from region_runtime import Rejected, canonical, require
from regional_authority import candidate_receipts
from transition_lab import Clients
from transition_runtime import DurableRuntime
from worker_placement import Offer, assess, evaluate, region_load

CLASSIFICATION = "LOCAL_CERTIFIED_REFERENCE_STAGING_RESTORE_NOT_MINECRAFT_NOT_MULTI_HOST"


def refuse(action, reason):
    try:
        action()
    except Rejected as error:
        require(str(error)==reason,"unexpected reference refusal: "+str(error))
        return str(error)
    raise Rejected("unexpected reference acceptance")


def run(cli,scope,cpus,root,report):
    q = Quorum(cli,root,report["artifact_manifest"])
    q.player("invite_survivor",5)
    q.worker("allow_worker",3)
    report.update(world_id=q.world,binding=q.binding,participant_keys=q.keys)
    Authority = combined_authority(q.binding)
    source = root/"source.db"
    ledger = Authority.create(source,parts(q.binding)[0],fixture(),[0,1],[FULL])
    stages = []

    def step(label,ledger,revision,direction,certify):
        before = ledger.save()
        gate = CertifiedClientGate(ledger,q,[1])
        player = q.role(label+"-player",5)
        worker = q.role(label+"-worker",3,750)
        offer = Offer(0,revision,80,750)
        with CappedPolicyLab(scope,[offer],cpus,q.binding) as lab:
            samples,probes = lab.probe()
            decision_time = time.monotonic_ns()
            require(assess(offer,samples[0],decision_time)=="eligible"
                    and evaluate([0]*8,region_load(ledger.read()["state"]),samples) is not None,
                    "restoration worker placement not measured")
            require(offer.process_memory_mib<=worker["result"]["offered_budget"]["memory_mib"],
                    "worker exceeds signed budget")
            runtime = DurableRuntime(ledger,[PolicyChannel(lab.channels[0],q.binding)])
            with Clients(gate,ack_window=60) as clients:
                req = {"op":"act","world":before["world"],"id":secrets.token_hex(32),
                       "tick":before["tick"]+1,"parent":before["cut_root"],
                       "direction":direction,"action":direction}
                clients.spawn(1,req)
                require(clients.sessions[0]["submission"]["op"]=="queued","request not queued")
                pre = lab.observe(0)
                scope.recheck([pre])
                runtime.advance()
                candidate = observed_snapshot(ledger,q.mapping)
                receipts = candidate_receipts(runtime.runtime.coordinator)
                post = lab.observe(0)
                scope.recheck([post])
                worker_pid = lab.processes[0].pid
                lab.close()
                runtime.runtime.frozen = runtime.runtime.coordinator.poisoned = True
                require(not scope.active and not scope.poisoned and lab.processes[0].returncode==0,
                        "worker not retired")
                def blocked():
                    for attempt in (lambda:gate.view(1),lambda:gate.submit(1,req),
                                    lambda:gate.committed_receipts([(1,req)])):
                        refuse(attempt,"uncertified staged tick")
                blocked()
                if certify:
                    accepted = q.certify_network_checkpoint(label+"-checkpoint",candidate,before_sync=blocked)
                    gate.publish(accepted)
                    rows = clients.finish()
                else:
                    rows = clients.finish(lose_ack=True)
                require(rows[0]["observed"]["reply"] is not None if certify else
                        rows[0]["observed"]["reply"] is None,"wrong client acknowledgment")
            stages.append({"label":label,"before":before,"after":ledger.save(),"checkpoint":candidate,
                           "client":rows[0],"worker_pid":worker_pid,"worker_retired":True,
                           "uncertified_access_refused":True,"pre_kernel":pre,"post_kernel":post,
                           "probes":probes,"player_proof":player,"worker_proof":worker})
        require(not scope.active and {name for _,name in socket.if_nameindex()}=={"lo"},
                "restoration stage scope not released")
        print(label+": tick "+str(candidate["tick"])+(", certified" if certify else ", no acknowledgment"),flush=True)
        return receipts

    try:
        step("first",ledger,1,1,True)
        q.depart()
        q.certify("surviving_recovery",["--recover"])
        before = ledger.save()
        ledger.rotate(before["generation"],before["tick"],before["runtime_root"],[FULL])
        stale = step("abandoned",ledger,2,1,False)
        pin = restoration_source_pin(ledger.save())
        source_history = ledger.history()
    finally:
        ledger.close()
    source_hash = sha(source.read_bytes())
    destination = root/"restored.db"
    restoration = restore_certified_staging(q,source,destination,expected_source=pin,
                                            source_retired=not scope.active and not scope.poisoned)
    restored = Authority(destination)
    try:
        checkpoint = observed_snapshot(restored,q.mapping)
        q.assert_state(checkpoint)
        quarantine = restored.save()
        restored_history = restored.history()
        claim_refusal = refuse(lambda:restored.claim(quarantine["generation"],quarantine["tick"],
                            quarantine["runtime_root"]),"generation already claimed; rotate before worker restart")
        stale_refusal = refuse(lambda:restored.commit(stale),"stale ownerless receipt session")
        # Reauthenticate the reference identity before recovering the old
        # certified result over an actual fresh TCP connection.
        replay_proof = q.role("restored-replay-player",5)
        gate = CertifiedClientGate(restored,q,[1])
        with Clients(gate) as clients:
            clients.spawn(1,stages[0]["client"]["request"])
            replay = clients.finish()[0]
        require(replay["observed"]["reply"]==stages[0]["client"]["observed"]["reply"],
                "restored client result differs")
        discarded = refuse(lambda:gate.committed_receipts([(1,stages[1]["client"]["request"])]),
                            "no exact certified command receipt")
        restored.rotate(quarantine["generation"],quarantine["tick"],quarantine["runtime_root"],[FULL])
        step("replacement",restored,3,-1,True)
    finally:
        restored.close()
    require(sha(source.read_bytes())==source_hash,"original staging source changed")
    require(sha(q.founder_path.read_bytes())==q.founder_digest,"departed founder changed")
    report.update(stages=stages,restoration=restoration,source_sha256=source_hash,
                  source_sha256_after=sha(source.read_bytes()),source_history=source_history,
                  restored_history=restored_history,restored_checkpoint=checkpoint,quarantine=quarantine,
                  claim_refusal=claim_refusal,stale_refusal=stale_refusal,discarded_receipt_refusal=discarded,
                  recovered_client=replay,recovered_player_proof=replay_proof,
                  certified_cuts=q.records,history=json.loads(q.history.read_bytes()),
                  founder_journal_unchanged=True,all_workers_exited=not scope.active and not scope.poisoned)


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
    report = {"schema":1,"classification":CLASSIFICATION,"physical_machines":1,"live_minecraft_players":0,
              "runtime_activation":False,"canonical_minecraft_authority":False,"actual_kernel_caps":True,
              "artifact_manifest":artifact_manifest(args.cli),"restore_lab_sha256":sha(Path(__file__).read_bytes())}
    try:
        with tempfile.TemporaryDirectory(prefix="sc68-restore-lab-") as temp:
            run(args.cli,scope,cpus,Path(temp),report)
        with args.output.open("x") as f:
            f.write(canonical(report).decode()+"\n")
        print(json.dumps({"certified_ticks":2,"abandoned_ticks":1,"restored_tick":1,"workers":3,
                          "source_preserved":True,"recovered_receipts":1}))
    finally:
        os.sched_setaffinity(0,affinity)


if __name__ == "__main__":
    main()
