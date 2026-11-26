#!/usr/bin/env python3
"""Independent replay of managed reference contribution observations."""
import argparse
from dataclasses import asdict
import json
from pathlib import Path
import re
import subprocess
import tempfile
from types import SimpleNamespace

from combined_ownerless_lab import (ARTIFACTS, FALSE_FLAGS, combined_authority, compatibility_for,
                                    review_checkpoint, snapshot)
from managed_certification import check_completion, check_plan
from managed_ownerless_lab import CERTIFICATION_ARTIFACTS, CLASSIFICATION, EXTRA_ARTIFACTS, SUBJECTS
from ownerless_permission import parts, verify_profile
from player_runtime import fixture
from region_runtime import canonical, digest, integer, require
from regional_authority import validated_state, wire
from transition_runtime import transition_view
from verify_combined_ownerless_evidence import hexbytes, same, verify_sync_signatures
from worker_placement import Offer, Sample, choose, region_load

LABELS = ["genesis","invite_survivor","allow_primary","allow_extra","surviving_recovery",
          "first-checkpoint","revoke_extra","recovered-checkpoint"]
EVENTS = ["consent_changed","begin_probe","admitted","claimed","staged_cut_fenced","released",
          "begin_probe","admitted","claimed","consent_changed","released","consent_changed","consent_changed",
          "begin_probe","admitted","claimed","staged_cut_fenced","released"]


def verify_certification_records(data,expected):
    """Bind durable intent/completion rows to independently verified cuts."""
    records,history = data["certified_cuts"],data["history"]
    q = SimpleNamespace(world=data["world_id"],binding=data["binding"],keys=data["participant_keys"],
                        mapping=[data["participant_keys"][0],data["participant_keys"][5]])
    require(len(data["certifications"])==len(expected),"managed certification record count differs")
    for row,(scope,seq,stage) in zip(data["certifications"],expected):
        plan,done = row["plan"],row["completion"]
        require(row["scope"]==plan["scope"]==scope and plan["generation"]==stage["after"]["generation"]
                and plan["base"]==records[seq-1]["head"] and plan["base_checkpoint"]==records[seq-1]["checkpoint"]
                and plan["candidate"]==records[seq]["checkpoint"] and plan["label"]==records[seq]["label"]
                and plan["offers"]==stage["scope_record"]["offers"]
                and plan["blocked"]==stage["released"]["blocked"]
                and plan["voters"]==records[seq]["voter_slots"]
                and [p["slot"] for p in plan["active"]]==[1,2]
                and plan["base_history"]=={"creation":history["creation"],"records":history["records"][:seq-1]}
                and plan["proposal"]=={"proposal_hash":records[seq]["head"]["head"],
                                       "proposal":history["records"][seq-1]["proposal"]}
                and done=={"head":records[seq]["head"],"history":{"creation":history["creation"],
                        "records":history["records"][:seq]},"sync":records[seq]["network"]["sync"]},
                "managed certification evidence differs from retained cut")
        check_plan(q,plan,check_active=False)
        check_completion(q,plan,done,verify_signatures=True)


def verify_observations(data):
    require(type(data["schema"]) is int and data["schema"] in (1,2) and data["classification"]==CLASSIFICATION
            and data["physical_machines"]==1 and data["live_minecraft_players"]==0
            and data["runtime_activation"] is False and data["canonical_minecraft_authority"] is False
            and all(data[k] is True for k in ("actual_kernel_caps","founder_journal_unchanged",
                "all_scopes_released","reopened_equal","zero_consent_no_spawn","revoked_role_refused")),
            "wrong managed classification")
    world,policy,compatibility = parts(data["binding"])
    require(data["world_id"]=="scownerless:"+world and same(data["slot_subjects"],list(SUBJECTS))
            and set(data["artifact_manifest"])==set(ARTIFACTS)|set(EXTRA_ARTIFACTS)|{"rust-cli"}|
                (set(CERTIFICATION_ARTIFACTS) if data["schema"]==2 else set())
            and all(digest(v) for v in data["artifact_manifest"].values())
            and compatibility_for(data["artifact_manifest"])==compatibility,"wrong managed artifact pin")
    keys = data["participant_keys"]
    require(len(keys)==len(set(keys))==6 and all(digest(k) for k in keys),"wrong managed identities")
    records,history = data["certified_cuts"],data["history"]
    require([r["label"] for r in records]==LABELS and len(history["records"])==7,"wrong managed history")
    heads = [r["head"] for r in records]
    ids = {hexbytes(v["public_key"]):hexbytes(v["peer_id"]) for v in history["creation"]["genesis"]["validators"]}
    require(set(ids)==set(keys[:3]),"wrong managed validators")
    for seq,(row,fence,tick) in enumerate(zip(records,(1,2,3,4,5,5,6,6),(0,0,0,0,0,1,1,2))):
        head = row["head"]
        require(head["sequence"]==seq and head["reference_fence"]==fence and row["checkpoint"]["tick"]==tick
                and head["world_id"]==data["world_id"] and head["policy_hash"]==policy
                and head["compatibility"]==compatibility and head["participation_mode"]=="invite_only"
                and head["majority_threshold"]==2 and all(head[k] is False for k in (
                    "runtime_activation","live_quorum_verified","player_admission","v1_canonical_world_created")),
                "wrong managed certified head")
        if not seq:
            continue
        entry = history["records"][seq-1]
        voters = [0,1] if seq<=3 else [1,2]
        require(same(row["voter_slots"],voters) and len(entry["votes"])==2
                and {hexbytes(v["validator"]) for v in entry["votes"]}=={ids[keys[s]] for s in voters},
                "wrong managed signer set")
        action = entry["proposal"]["action"]
        if seq in (5,7):
            require(hexbytes(action["checkpoint"]["commitment"])==head["checkpoint"],"wrong managed checkpoint")
            net = row["network"]
            require(net["collector_slot"]==1 and net["voter_slots"]==voters
                    and net["pre_sync_client_access_refused"] is True and len(net["sync"])==1
                    and net["sync"][0]["recipient_slot"]==2,"wrong managed network exchange")
            require(same(net["reviews"],[{"slot":s,"base":heads[seq-1]["head"],"proposal":head["head"],
                        "base_checkpoint":heads[seq-1]["checkpoint"],"candidate_checkpoint":head["checkpoint"],
                        "replayed_root":row["checkpoint"]["runtime_root"]} for s in voters]),"wrong managed review")
            for listener,kind,field in ((net["vote_listener"],"ownerless_reference_decision_listener","proposal"),
                                      (net["sync"][0]["listener"],"ownerless_reference_sync_listener","head")):
                require(listener["classification"]==kind and listener["validator"]=="scpeer:"+ids[keys[2]]
                        and listener["world_id"]==data["world_id"] and listener[field]==head["head"]
                        and listener["runtime_activation"] is False
                        and re.fullmatch(r"127\.0\.0\.1:[1-9][0-9]{0,4}",listener["address"]) is not None
                        and int(listener["address"].rsplit(":",1)[1])<=65535,"wrong managed listener")
            result = net["sync"][0]["result"]
            require(result["classification"]=="ownerless_reference_checkpoint_synced" and all(result[k] is False
                        for k in ("runtime_activation","live_quorum_verified","write_authority","canonical_authority",
                                  "player_admission","worker_admission"))
                    and same(result["receipt"]["body"],{"world":list(bytes.fromhex(world)),
                         "base":list(bytes.fromhex(heads[seq-1]["head"])),"head":list(bytes.fromhex(head["head"])),
                         "sequence":seq,"fence":fence,"checkpoint":list(bytes.fromhex(head["checkpoint"])),
                         "recipient":list(bytes.fromhex(ids[keys[2]]))}),"wrong managed signed sync")
            hexbytes(result["receipt"]["signature"],64)
        else:
            require("network" not in row and same(row["checkpoint"],records[seq-1]["checkpoint"])
                    and head["checkpoint"]==heads[seq-1]["checkpoint"],"governance changed managed checkpoint")
            if seq==1:
                require(action["set_player"]["allowed"] is True
                        and hexbytes(action["set_player"]["player"]["public_key"])==keys[5],"wrong managed invitation")
            elif seq in (2,3,6):
                role = action["set_worker"]
                require(hexbytes(role["worker"]["public_key"])==keys[3 if seq==2 else 4]
                        and role["maximum"]==({"memory_mib":512,"cpu_millicores":1000} if seq!=6 else None),
                        "wrong managed eligibility")
            else:
                require(action=="recover","wrong managed recovery fence")

    mapping = [keys[0],keys[5]]
    genesis = fixture()
    initial = snapshot(mapping,genesis,0,{"genesis":wire(genesis),"batches":[],"commands":[]})
    require(same(initial,records[0]["checkpoint"]),"wrong managed genesis")
    for base,end in ((4,5),(6,7)):
        require(review_checkpoint(data["binding"],mapping,records[base]["checkpoint"],records[end]["checkpoint"])
                ==records[end]["checkpoint"]["runtime_root"],"wrong managed independent replay")
    Authority = combined_authority(data["binding"])
    stages = data["stages"]
    require([s["label"] for s in stages]==["first","withdrawal","recovered"],"wrong managed stages")
    challenges,pids,sessions = set(),set(),set()
    def pid(value):
        require(integer(value,1,2**31-1) and value not in pids,"duplicate managed process")
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
                and (cpu is None or result["offered_budget"]==budget),"wrong managed role proof")
        challenges.add(request["challenge_hash"])
        hexbytes(request["request"]["signature"],64)
        hexbytes(response["signature"],64)
    def kernel(row,offer):
        require(row["accounting"]=="linux_cgroup_v2_process_charges" and row["java_heap_mib"] is None
                and row["kernel_memory_max_bytes"]==offer.process_memory_mib*1024**2
                and 0<row["kernel_memory_current_bytes"]<=row["kernel_memory_high_bytes"]<row["kernel_memory_max_bytes"]
                and row["kernel_memory_current_bytes"]<=row["kernel_memory_peak_bytes"]<=row["kernel_memory_max_bytes"]
                and 0<row["effective_cpu_millicores"]<=offer.cpu_millicores
                and row["memory_oom_events"]==row["memory_oom_kills"]==0,"wrong managed kernel cap")
    for i,(stage,base,end,selected,revision) in enumerate(zip(stages,(4,5,6),(5,None,7),
                                                              ([0,1],[0,1],[0]),(2,2,5)),1):
        prior = records[base]["checkpoint"]
        target = records[end]["checkpoint"] if end is not None else prior
        state,_ = validated_state(prior["regions"])
        before,after = stage["before"],stage["after"]
        require(before["tick"]==prior["tick"] and before["runtime_root"]==prior["runtime_root"]
                and before["generation"]==i and after["generation"]==i+1
                and after["tick"]==target["tick"] and after["runtime_root"]==target["runtime_root"]
                and same(before["regions"],prior["regions"]) and same(after["regions"],target["regions"])
                and after["cut_root"]==Authority.cut(world,target["tick"],target["runtime_root"],[0,1])
                and before["world"]==after["world"]==world and after["claimed"] is True,
                "wrong managed stage cut")
        if i>1:
            require(same(before,stages[i-2]["after"]),"managed reopen changed staging")
        probe,ready,claimed,draining,released = (stage[k] for k in ("probe","ready","claimed","draining","released"))
        require([x["phase"] for x in (probe,ready,claimed,draining,released)]==
                ["probing","ready","running","draining","paused"]
                and all(x["scope"]==i for x in (probe,ready,claimed,draining,released))
                and probe["generation"]==i and all(x["generation"]==i+1 for x in (ready,claimed,draining,released))
                and probe["probes"]==ready["probes"]==claimed["probes"]==draining["probes"]==selected
                and released["probes"]==released["assignment"]==[]
                and ready["assignment"]==claimed["assignment"]==[[s,revision] for s in selected],
                "wrong managed lifecycle")
        offers = [Offer(**o) for o in probe["offers"]]
        expected = [Offer(0,revision,56 if i<3 else 80,1000 if i<3 else 750),
                    Offer(1,revision,56 if i<3 else 0,1000 if i<3 else 0),Offer(2,1,0,0),Offer(3,1,0,0)]
        require(offers==expected,"wrong managed explicit offers")
        samples = {int(s):Sample(**v["sample"]) for s,v in stage["probes"].items()}
        require(set(samples)==set(selected) and same(stage["decision"],choose(offers,samples,region_load(state),
                    stage["decision_ns"])) and sorted(set(stage["decision"]["plan"]["owner_by_region"]))==selected,
                "wrong measured managed admission")
        record = stage["scope_record"]
        require(record["released"] is True and same(record["offers"],probe["offers"])
                and same(record["base"],{"head":heads[base],"tick":before["tick"],
                    "root":before["runtime_root"],"cut":before["cut_root"]})
                and same(record["samples"],{str(s):asdict(v) for s,v in samples.items()})
                and [p["slot"] for p in record["proofs"]]==selected,"wrong durable managed scope authorization")
        for p in record["proofs"]:
            slot = p["slot"]
            require(p["revision"]==revision,"wrong durable consent revision")
            proof(p["proof"],SUBJECTS[slot],base,offers[slot].cpu_millicores)
            kernel(stage["probes"][str(slot)]["kernel"],offers[slot])
        proof(stage["player_proof"],5,base)
        require(len(stage["guards"])==(2 if i==2 else 3),"missing claim/seal/commit guard")
        for guard in stage["guards"]:
            require(guard["slots"]==selected and len(guard["kernel"])==len(selected),"wrong managed guard coverage")
            for slot,row in zip(selected,guard["kernel"]):
                kernel(row,offers[slot])
                require(row["effective_cpu_millicores"]>=samples[slot].effective_cpu_millicores,"capacity reduced")
        require([w["slot"] for w in stage["workers"]]==selected and len(stage["cleanup"])==len(selected),
                "wrong managed worker coverage")
        for worker,cleanup,grant in zip(stage["workers"],stage["cleanup"],after["grants"]):
            pid(worker["pid"])
            require(same(cleanup,{"slot":worker["slot"],"pid":worker["pid"],"exitcode":0,"leaf_removed":True})
                    and grant["generation"]==i+1 and grant["base_tick"]==before["tick"]
                    and grant["base_root"]==before["runtime_root"] and grant["world"]==world
                    and grant["regions"]==[r for r,s in enumerate(stage["decision"]["plan"]["owner_by_region"])
                                           if s==worker["slot"]]
                    and digest(grant["session"]) and grant["session"] not in sessions,"wrong managed worker cleanup/grant")
            sessions.add(grant["session"])
        client = stage["client"]
        pid(client["pid"])
        req = client["request"]
        require(client["submission"]=={"op":"queued","id":req["id"],"tick":req["tick"]}
                and same(client["observed"]["joined"],transition_view(world,before["tick"],before["cut_root"],
                    state,[0,1],1)) and client["observed"]["player"]==1,"wrong managed client request")
        if i==2:
            require(record["staged"] is None and released["offers"][1]["process_memory_mib"]==0
                    and stage["withdrawal_refusal"]=="contribution scope fenced"
                    and stage["seal_refusal"]=="retired runtime cannot seal input"
                    and stage["overlap_refusal"]=="previous scope not released"
                    and stage["receipt_refusal"]=="no exact certified command receipt"
                    and stage["certification_refusal"]=="no released managed staged cut"
                    and client["observed"]["reply"] is None
                    and stage["journal"]["commands"][-1]=={"player":1,"request":req,"response":None}
                    and stage["journal"]["batches"][-1]["result"] is None,"withdrawn cut escaped")
        else:
            require(same(record["staged"],{"tick":after["tick"],"root":after["runtime_root"],
                         "cut":after["cut_root"],"generation":after["generation"]})
                    and stage["uncertified_refusal"]=="uncertified staged tick"
                    and stage["cleanup_refusal"]=="managed scope cleanup not released"
                    and same(stage["candidate"],target) and stage["accepted_head"]==heads[end]["head"]
                    and same(stage["journal"],target["transaction_journal"])
                    and same(client["observed"]["reply"],target["transaction_journal"]["commands"][-1]["response"])
                    and same(req,target["transaction_journal"]["commands"][-1]["request"]),"wrong managed certified receipt")
            expected_check = {**heads[end],"classification":"verified_local_ownerless_reference_cut",
                "exact_reference_cut_verified":True,"checkpoint_transition_verified":True,
                "expected_parent":heads[base]["head"],"reference_player_eligible":True,"player_key":keys[5],
                "local_identity":"scpeer:"+ids[keys[1]],"local_validator":True,"pending_proposal":None,
                "canonical_authority":False,"write_authority":False,"worker_admission":False,"freshness_lease":False}
            require(same(stage["publication_checks"],[expected_check]),"wrong managed publication proof")
    require(same(stages[1]["client"]["request"],stages[2]["client"]["request"])
            and stages[1]["tentative_root"]==stages[2]["after"]["runtime_root"]
            and stages[2]["stale_receipt_refusal"]=="stale ownerless receipt session"
            and same(data["final"],stages[2]["after"]) and same(data["final_journal"],records[7]["checkpoint"]["transaction_journal"])
            and [e["event"] for e in data["events"]]==EVENTS,"managed recovery differs")
    first,withdrawn,recovered = stages
    def event(kind,status,**changes):
        return {"event":kind,**status,**changes}
    expected_events = [
        event("consent_changed",first["probe"],phase="paused",scope=0,probes=[]),
        event("begin_probe",first["probe"]),
        event("admitted",first["ready"]),
        event("claimed",first["claimed"]),
        event("staged_cut_fenced",first["draining"],tick=first["before"]["tick"],root=first["before"]["runtime_root"]),
        event("released",first["released"]),
        event("begin_probe",withdrawn["probe"]),
        event("admitted",withdrawn["ready"]),
        event("claimed",withdrawn["claimed"]),
        event("consent_changed",withdrawn["draining"]),
        event("released",withdrawn["released"]),
        event("consent_changed",withdrawn["released"],offers=[asdict(o) for o in
            [Offer(0,4,80,750),Offer(1,4,56,750),Offer(2,1,0,0),Offer(3,1,0,0)]]),
        event("consent_changed",withdrawn["released"],offers=recovered["probe"]["offers"]),
        event("begin_probe",recovered["probe"]),
        event("admitted",recovered["ready"]),
        event("claimed",recovered["claimed"]),
        event("staged_cut_fenced",recovered["draining"],tick=recovered["before"]["tick"],
              root=recovered["before"]["runtime_root"]),
        event("released",recovered["released"]),
    ]
    require(same(data["events"],expected_events),"durable lifecycle event differs from observed state")
    if data["schema"]==2:
        verify_certification_records(data,[(1,5,data["stages"][0]),(3,7,data["stages"][2])])
    else:
        require("certifications" not in data,"unexpected legacy certification records")
    return {"certified_ticks":2,"withdrawn_ticks":1,"worker_processes":5,"scopes":3,
            "signed_decisions":7,"signed_sync_receipts":2,"role_proofs":8}


def verify(data,cli):
    result = verify_observations(data)
    verify_sync_signatures({**data,"schema":3})
    cli = str(Path(cli).resolve(strict=True))
    with tempfile.TemporaryDirectory(prefix="sc68-managed-verify-") as temp:
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
                "wrong managed signed creation")
        for seq,row in enumerate(data["certified_cuts"]):
            prefix = write("history.json",{"creation":history["creation"],"records":history["records"][:seq]})
            checkpoint = write("checkpoint.bin",row["checkpoint"])
            require(auth("inspect","--history",prefix,"--expected-world",data["world_id"])==row["head"]
                    and auth("state-hash","--checkpoint",checkpoint)["checkpoint"]==row["head"]["checkpoint"],
                    "wrong managed signed history")
        require(not absent.exists(),"read-only verification created profile")
    return result


if __name__=="__main__":
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("evidence",type=Path)
    p.add_argument("--cli",type=Path,required=True)
    args = p.parse_args()
    print(json.dumps(verify(json.loads(args.evidence.read_bytes()),args.cli),sort_keys=True))
