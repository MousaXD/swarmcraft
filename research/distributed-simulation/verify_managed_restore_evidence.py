#!/usr/bin/env python3
"""Independently replay managed restoration evidence and signed checkpoints."""
import argparse
from dataclasses import asdict
import json
from pathlib import Path
import subprocess
import tempfile

from combined_ownerless_lab import (ARTIFACTS as BASE_ARTIFACTS, FALSE_FLAGS, combined_authority,
                                    compatibility_for, restoration_source_pin, review_checkpoint, sha, snapshot)
from managed_restore_lab import ARTIFACTS, CLASSIFICATION, SUBJECTS
from managed_ownerless_lab import CERTIFICATION_ARTIFACTS
from ownerless_permission import parts, verify_profile
from player_runtime import fixture
from region_runtime import canonical, digest, integer, require
from regional_authority import validated_state, wire
from transition_runtime import transition_view
from verify_combined_ownerless_evidence import hexbytes, same, verify_sync_signatures
from verify_managed_ownerless_evidence import verify_certification_records
from worker_placement import Offer, Sample, choose, region_load

LABELS = ["genesis","invite_survivor","allow_worker","surviving_recovery","first-checkpoint",
          "revoke_player","reinvite_player","replacement-checkpoint"]


def verify_observations(data):
    require(type(data["schema"]) is int and data["schema"] in (1,2)
            and data["classification"]==CLASSIFICATION and data["physical_machines"]==1
            and data["live_minecraft_players"]==0 and data["runtime_activation"] is False
            and data["canonical_minecraft_authority"] is False
            and all(data[k] is True for k in ("actual_kernel_caps","founder_journal_unchanged","all_scopes_released")),
            "wrong managed restore classification")
    world,policy,compatibility = parts(data["binding"])
    require(data["world_id"]=="scownerless:"+world and same(data["slot_subjects"],list(SUBJECTS))
            and set(data["artifact_manifest"])==set(BASE_ARTIFACTS)|set(ARTIFACTS)|{"rust-cli"}|
                (set(CERTIFICATION_ARTIFACTS) if data["schema"]==2 else set())
            and all(digest(v) for v in data["artifact_manifest"].values())
            and compatibility_for(data["artifact_manifest"])==compatibility,"wrong managed restore artifact pin")
    keys = data["participant_keys"]
    require(len(keys)==len(set(keys))==6 and all(digest(k) for k in keys),"wrong restore identities")
    mapping = [keys[0],keys[5]]
    records,history = data["certified_cuts"],data["history"]
    require([r["label"] for r in records]==LABELS and len(history["records"])==7,"wrong restore history")
    heads = [r["head"] for r in records]
    ids = {hexbytes(v["public_key"]):hexbytes(v["peer_id"]) for v in history["creation"]["genesis"]["validators"]}
    require(set(ids)==set(keys[:3]),"wrong restore validator roster")
    for seq,(row,fence,tick) in enumerate(zip(records,(1,2,3,4,4,5,6,6),(0,0,0,0,1,1,1,2))):
        head = row["head"]
        require(head["sequence"]==seq and head["reference_fence"]==fence and row["checkpoint"]["tick"]==tick
                and head["world_id"]==data["world_id"] and head["policy_hash"]==policy
                and head["compatibility"]==compatibility and head["participation_mode"]=="invite_only"
                and head["majority_threshold"]==2 and all(head[k] is False for k in
                    ("runtime_activation","live_quorum_verified","player_admission","v1_canonical_world_created")),
                "wrong restore certified head")
        if not seq:
            continue
        entry = history["records"][seq-1]
        voters = [0,1] if seq<3 else [1,2]
        require(same(row["voter_slots"],voters) and len(entry["votes"])==2
                and {hexbytes(v["validator"]) for v in entry["votes"]}=={ids[keys[s]] for s in voters},
                "wrong restore signer set")
        action = entry["proposal"]["action"]
        if seq in (4,7):
            require(hexbytes(action["checkpoint"]["commitment"])==head["checkpoint"],"wrong certified restore checkpoint")
            net = row["network"]
            require(net["collector_slot"]==1 and net["voter_slots"]==voters
                    and net["pre_sync_client_access_refused"] is True and len(net["sync"])==1
                    and net["sync"][0]["recipient_slot"]==2,"wrong managed restore network exchange")
            require(same(net["reviews"],[{"slot":s,"base":heads[seq-1]["head"],"proposal":head["head"],
                "base_checkpoint":heads[seq-1]["checkpoint"],"candidate_checkpoint":head["checkpoint"],
                "replayed_root":row["checkpoint"]["runtime_root"]} for s in voters]),"wrong managed restore review")
            for listener,kind,field in ((net["vote_listener"],"ownerless_reference_decision_listener","proposal"),
                                      (net["sync"][0]["listener"],"ownerless_reference_sync_listener","head")):
                require(listener["classification"]==kind and listener["validator"]=="scpeer:"+ids[keys[2]]
                        and listener["world_id"]==data["world_id"] and listener[field]==head["head"]
                        and listener["runtime_activation"] is False,"wrong restore listener")
            result = net["sync"][0]["result"]
            require(result["classification"]=="ownerless_reference_checkpoint_synced"
                    and all(result[k] is False for k in ("runtime_activation","live_quorum_verified",
                        "write_authority","canonical_authority","player_admission","worker_admission"))
                    and same(result["receipt"]["body"],{"world":list(bytes.fromhex(world)),
                        "base":list(bytes.fromhex(heads[seq-1]["head"])),"head":list(bytes.fromhex(head["head"])),
                        "sequence":seq,"fence":fence,"checkpoint":list(bytes.fromhex(head["checkpoint"])),
                        "recipient":list(bytes.fromhex(ids[keys[2]]))}),"wrong restore signed sync")
            hexbytes(result["receipt"]["signature"],64)
        else:
            require("network" not in row and same(row["checkpoint"],records[seq-1]["checkpoint"])
                    and head["checkpoint"]==heads[seq-1]["checkpoint"],"governance changed restore checkpoint")
            if seq in (1,5,6):
                role = action["set_player"]
                require(role["allowed"] is (seq!=5) and hexbytes(role["player"]["public_key"])==keys[5],
                        "wrong restore invitation history")
            elif seq==2:
                role = action["set_worker"]
                require(hexbytes(role["worker"]["public_key"])==keys[3]
                        and role["maximum"]=={"memory_mib":512,"cpu_millicores":1000},"wrong restore eligibility")
            else:
                require(action=="recover","wrong restore recovery fence")
    initial = snapshot(mapping,fixture(),0,{"genesis":wire(fixture()),"batches":[],"commands":[]})
    require(same(initial,records[0]["checkpoint"]),"wrong restore genesis")
    stages = data["stages"]
    require([s["label"] for s in stages]==["first","abandoned","replacement"],"wrong managed restore stages")
    Authority = combined_authority(data["binding"])
    pids,sessions,challenges,requests = set(),set(),set(),set()
    def pid(value):
        require(integer(value,1,2**31-1) and value not in pids,"duplicate restore process")
        pids.add(value)
    def proof(row,subject,seq,cpu=None):
        request,response,result = row["request"],row["response"],row["result"]
        ch = request["request"]["challenge"]
        budget = {"memory_mib":512,"cpu_millicores":cpu}
        require(row["subject_slot"]==subject and ch==response["challenge"]
                and hexbytes(ch["subject"]["public_key"])==keys[subject]
                and hexbytes(ch["verifier"]["public_key"])==keys[1]
                and hexbytes(ch["verifier"]["peer_id"])==ids[keys[1]]
                and hexbytes(ch["world"])==world and hexbytes(ch["head"])==heads[seq]["head"]
                and hexbytes(ch["policy"])==policy and hexbytes(ch["compatibility"])==compatibility
                and ch["fence"]==heads[seq]["reference_fence"] and hexbytes(ch["nonce"])!="0"*64
                and ch["intent"]==("play" if cpu is None else {"volunteer":{"budget":budget}})
                and digest(request["challenge_hash"]) and request["challenge_hash"] not in challenges
                and result["challenge"]==request["challenge_hash"] and result["head"]==heads[seq]["head"]
                and result["world_id"]==data["world_id"]
                and result["subject"]=="scpeer:"+hexbytes(ch["subject"]["peer_id"])
                and result["classification"]=="verified_ownerless_reference_participation"
                and result["role"]==("reference_player" if cpu is None else "reference_volunteer")
                and all(result[k] is False for k in FALSE_FLAGS)
                and (cpu is None or result["offered_budget"]==budget),"wrong restore role proof")
        challenges.add(request["challenge_hash"])
        hexbytes(request["request"]["signature"],64)
        hexbytes(response["signature"],64)
    def kernel(row,sample):
        require(row["accounting"]=="linux_cgroup_v2_process_charges" and row["java_heap_mib"] is None
                and row["kernel_memory_max_bytes"]==80*1024**2
                and 0<row["kernel_memory_current_bytes"]<=row["kernel_memory_high_bytes"]<row["kernel_memory_max_bytes"]
                and row["kernel_memory_current_bytes"]<=row["kernel_memory_peak_bytes"]<=row["kernel_memory_max_bytes"]
                and sample.effective_cpu_millicores<=row["effective_cpu_millicores"]<=750
                and row["memory_oom_events"]==row["memory_oom_kills"]==0,"wrong restore kernel cap")
    def auth_row(req,seq):
        return [req["id"],1,sha(canonical(req)),seq,heads[seq]["head"],keys[5]]
    for i,(stage,base,end,gen,rev,direction) in enumerate(zip(stages,(3,4,6),(4,None,7),
                                                              (2,3,5),(2,2,4),(1,1,-1)),1):
        prior = records[base]["checkpoint"]
        candidate = stage["candidate"]
        state,_ = validated_state(prior["regions"])
        require(review_checkpoint(data["binding"],mapping,prior,candidate)==candidate["runtime_root"]
                and (end is None or same(candidate,records[end]["checkpoint"])),"wrong managed restore replay")
        before,after = stage["before"],stage["after"]
        require(before["generation"]==gen-1 and after["generation"]==after["accepted_generation"]==gen
                and before["tick"]==prior["tick"] and after["tick"]==candidate["tick"]
                and before["world"]==after["world"]==world and before["players"]==after["players"]==[0,1]
                and before["runtime_root"]==prior["runtime_root"] and after["runtime_root"]==candidate["runtime_root"]
                and before["cut_root"]==Authority.cut(world,prior["tick"],prior["runtime_root"],[0,1])
                and after["cut_root"]==Authority.cut(world,candidate["tick"],candidate["runtime_root"],[0,1])
                and same(before["regions"],prior["regions"]) and same(after["regions"],candidate["regions"])
                and after["claimed"] is True,"wrong managed restored generation")
        probe,ready,released = (stage[k] for k in ("probe","ready","released"))
        require([s["phase"] for s in (probe,ready,released)]==["probing","ready","paused"]
                and all(s["scope"]==i for s in (probe,ready,released))
                and probe["generation"]==gen-1 and ready["generation"]==released["generation"]==gen
                and probe["probes"]==ready["probes"]==[0] and ready["assignment"]==[[0,rev]]
                and released["probes"]==released["assignment"]==[],"wrong restored scope")
        offers = [Offer(**o) for o in probe["offers"]]
        require(offers==[Offer(0,rev,80,750),Offer(1,rev,0,0),Offer(2,1,0,0),Offer(3,1,0,0)],
                "wrong restored offers")
        samples = {int(s):Sample(**row["sample"]) for s,row in stage["probes"].items()}
        require(set(samples)=={0} and same(stage["decision"],choose(offers,samples,region_load(state),stage["decision_ns"]))
                and stage["decision"]["plan"]["owner_by_region"]==[0]*8,"wrong restored measured placement")
        record = stage["scope_record"]
        require(record["released"] is True and same(record["offers"],probe["offers"])
                and same(record["base"],{"head":heads[base],"tick":before["tick"],
                                        "root":before["runtime_root"],"cut":before["cut_root"]})
                and same(record["samples"],{"0":asdict(samples[0])})
                and same(record["staged"],{"tick":after["tick"],"root":after["runtime_root"],
                    "cut":after["cut_root"],"generation":gen})
                and len(record["proofs"])==1 and record["proofs"][0]["slot"]==0
                and record["proofs"][0]["revision"]==rev,"wrong durable restored scope")
        proof(record["proofs"][0]["proof"],3,base,750)
        proof(stage["player_proof"],5,base)
        kernel(stage["probes"]["0"]["kernel"],samples[0])
        require(len(stage["guards"])==3 and stage["worker_retired"] is True,"missing restored resource guards")
        for row in stage["guards"]:
            kernel(row,samples[0])
        pid(stage["worker_pid"])
        grant, = after["grants"]
        require(grant["world"]==world and grant["generation"]==gen and grant["base_tick"]==before["tick"]
                and grant["base_root"]==before["runtime_root"] and grant["regions"]==list(range(8))
                and digest(grant["session"]) and grant["session"] not in sessions,"wrong restored worker grant")
        sessions.add(grant["session"])
        client = stage["client"]
        pid(client["pid"])
        req = client["request"]
        command = candidate["transaction_journal"]["commands"][-1]
        require(req["id"] not in requests and req["direction"]==req["action"]==direction
                and same(req,command["request"]) and command["player"]==1
                and client["submission"]=={"op":"queued","id":req["id"],"tick":req["tick"]}
                and same(client["observed"]["joined"],transition_view(world,before["tick"],before["cut_root"],state,[0,1],1))
                and same(client["observed"]["reply"],None if end is None else command["response"])
                and stage["uncertified_refusal"]=="uncertified staged tick"
                and stage["cleanup_refusal"]=="managed scope cleanup not released"
                and same(stage["journal"],candidate["transaction_journal"]),"wrong managed restore client outcome")
        requests.add(req["id"])
        expected_auth = [auth_row(req,base)]
        if i>1:
            expected_auth.append(auth_row(stages[0]["client"]["request"],3))
            require(stage["stale_receipt_refusal"]=="stale ownerless receipt session","old grant not refused")
        require(same(stage["authorizations"],sorted(expected_auth,key=lambda row:row[0])),"wrong command authorization anchor")
        if end is not None:
            expected_check = {**heads[end],"classification":"verified_local_ownerless_reference_cut",
                "exact_reference_cut_verified":True,"checkpoint_transition_verified":True,
                "expected_parent":heads[base]["head"],"reference_player_eligible":True,"player_key":keys[5],
                "local_identity":"scpeer:"+ids[keys[1]],"local_validator":True,"pending_proposal":None,
                "canonical_authority":False,"write_authority":False,"worker_admission":False,"freshness_lease":False}
            require(stage["accepted_head"]==heads[end]["head"] and same(stage["publication_checks"],[expected_check]),
                    "wrong restored publication check")
    first,abandoned,replacement = stages
    require(not same(abandoned["candidate"],replacement["candidate"]) and
            abandoned["after"]["runtime_root"]!=replacement["after"]["runtime_root"],"abandoned branch reused")
    pin = data["source_pin"]
    require(same(pin["source"],restoration_source_pin(abandoned["after"])) and pin["phase"]=="paused"
            and pin["scope"]==2 and pin["blocked"]==[0] and same(pin["offers"],[asdict(o) for o in
                [Offer(0,2,80,750),Offer(1,3,0,0),Offer(2,1,0,0),Offer(3,1,0,0)]])
            and pin["journal_sha256"]==sha(canonical(abandoned["journal"])) and digest(pin["metadata_sha256"]),
            "wrong managed restore source pin")
    abandoned_id = abandoned["client"]["request"]["id"]
    require(same(data["restoration"],{"classification":"ownerless_reference_staging_restored_quarantined",
        "source":pin["source"],"world_id":data["world_id"],"head":heads[6]["head"],"checkpoint":heads[6]["checkpoint"],
        "restored_tick":1,"retired_generation":3,"quarantine_generation":4,"discarded_ticks":1,
        "source_preserved":True,"rotation_required":True,"live_quorum_verified":False,"runtime_activation":False,
        "canonical_authority":False,"write_authority":False,"player_admission":False,"worker_admission":False,
        "managed_consent_preserved":True,"scope_sequence":2,"abandoned_request_ids":[abandoned_id],
        "fresh_measured_admission_required":True}) and digest(data["source_sha256"])
        and data["source_sha256"]==data["source_sha256_after"],"wrong managed restoration outcome")
    q = data["quarantine"]
    require(q["generation"]==4 and q["accepted_generation"]==2 and q["claimed"] is True and q["tick"]==1
            and q["runtime_root"]==first["after"]["runtime_root"] and q["cut_root"]==first["after"]["cut_root"]
            and same(q["regions"],first["after"]["regions"]) and same(q,replacement["before"])
            and same(data["restored_checkpoint"],records[4]["checkpoint"]),"wrong quarantined state")
    grant, = q["grants"]
    require(grant["generation"]==4 and grant["base_tick"]==1 and grant["base_root"]==q["runtime_root"]
            and grant["session"] not in sessions and digest(grant["session"]),"quarantine reused a grant")
    status = data["restored_status"]
    require(status["phase"]=="paused" and status["scope"]==2 and status["generation"]==4 and status["tick"]==1
            and status["root"]==q["runtime_root"] and status["blocked"]==[0] and same(status["offers"],pin["offers"])
            and status["assignment"]==status["probes"]==[],"restored consent differs")
    old,new = data["source_history"],data["restored_history"]
    require([c[:2] for c in old["commits"]]==[[0,0],[1,2],[2,3]]
            and same(new["commits"],old["commits"][:2]) and [e[0] for e in old["epochs"]]==[1,2,3]
            and same(new["epochs"][:-1],old["epochs"][:2])
            and same(new["epochs"][-1],[4,1,q["runtime_root"],q["grants"],True]),"wrong managed restore provenance")
    require(same(data["restored_events"][:-1],data["source_events"])
            and same(data["restored_events"][-1],{"event":"certified_restore_quarantine",**status})
            and same(data["final_events"][:len(data["restored_events"])],data["restored_events"])
            and same(data["restored_authorizations"],first["authorizations"])
            and same(data["final_authorizations"],replacement["authorizations"])
            and same(data["final"],replacement["after"])
            and same(data["final_journal"],records[7]["checkpoint"]["transaction_journal"]),
            "managed restore lost consent or authorization provenance")
    replay = data["recovered_client"]
    pid(replay["pid"])
    state,_ = validated_state(records[4]["checkpoint"]["regions"])
    require(same(replay["request"],first["client"]["request"])
            and same(replay["submission"],first["client"]["observed"]["reply"])
            and same(replay["observed"]["reply"],first["client"]["observed"]["reply"])
            and same(replay["observed"]["joined"],transition_view(world,1,q["cut_root"],state,[0,1],1))
            and data["claim_refusal"]=="generation already claimed; rotate before worker restart"
            and data["stale_refusal"]=="stale ownerless receipt session"
            and data["discarded_receipt_refusal"]=="no exact certified command receipt"
            and data["abandoned_id_refusal"]=="abandoned managed command ID"
            and data["certification_refusal"]=="managed consent changed before certification","wrong restored client replay")
    if data["schema"]==2:
        verify_certification_records(data,[(1,4,data["stages"][0]),(3,7,data["stages"][2])])
        require(data["source_certifications"]==data["restored_certifications"]==data["certifications"][:1],
                "managed restore certification lineage differs")
    else:
        require(not any(k in data for k in ("source_certifications","restored_certifications","certifications")),
                "unexpected legacy certification records")
    return {"certified_ticks":2,"abandoned_ticks":1,"restored_tick":1,"workers":3,
            "recovered_receipts":1,"tombstoned_commands":1,"signed_decisions":7,"signed_sync_receipts":2}


def verify(data,cli):
    result = verify_observations(data)
    verify_sync_signatures({**data,"schema":3})
    cli = str(Path(cli).resolve(strict=True))
    with tempfile.TemporaryDirectory(prefix="sc68-managed-restore-verify-") as temp:
        root = Path(temp)
        absent = root/"absent"
        def write(name,obj):
            path = root/name
            path.write_bytes(canonical(obj))
            return str(path)
        def auth(*args):
            run = subprocess.run([cli,"--data-dir",str(absent),"ownerless","authority",*args],
                                 capture_output=True,check=True,timeout=20)
            return json.loads(run.stdout)
        history = data["history"]
        certificate = write("creation.json",history["creation"])
        require(verify_profile(cli,certificate,data["world_id"],parts(data["binding"])[2],absent)==data["binding"],
                "wrong signed managed restore creation")
        for seq,row in enumerate(data["certified_cuts"]):
            prefix = write("history.json",{"creation":history["creation"],"records":history["records"][:seq]})
            checkpoint = write("checkpoint.bin",row["checkpoint"])
            require(auth("inspect","--history",prefix,"--expected-world",data["world_id"])==row["head"]
                    and auth("state-hash","--checkpoint",checkpoint)["checkpoint"]==row["head"]["checkpoint"],
                    "wrong signed managed restore history")
        require(not absent.exists(),"read-only verification created profile")
    return result


if __name__=="__main__":
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("evidence",type=Path)
    p.add_argument("--cli",type=Path,required=True)
    args = p.parse_args()
    print(json.dumps(verify(json.loads(args.evidence.read_bytes()),args.cli),sort_keys=True))
