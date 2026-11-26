#!/usr/bin/env python3
"""Replay retained #68 restoration evidence; not remote attestation."""
import argparse
import json
from pathlib import Path
import re
import subprocess
import tempfile

from certified_restore_lab import CLASSIFICATION
from combined_ownerless_lab import (ARTIFACTS, FALSE_FLAGS, combined_authority, compatibility_for,
                                    restoration_source_pin, review_checkpoint, snapshot)
from ownerless_permission import parts, verify_profile
from player_runtime import fixture
from region_runtime import canonical, digest, integer, require
from regional_authority import validated_state, wire
from transition_runtime import transition_view
from verify_combined_ownerless_evidence import hexbytes, same, verify_sync_signatures

LABELS = ["genesis","invite_survivor","allow_worker","first-checkpoint",
          "surviving_recovery","replacement-checkpoint"]


def verify_observations(data):
    require(type(data["schema"]) is int and data["schema"]==1
            and data["classification"]==CLASSIFICATION and data["physical_machines"]==1
            and data["live_minecraft_players"]==0 and data["runtime_activation"] is False
            and data["canonical_minecraft_authority"] is False and data["actual_kernel_caps"] is True
            and data["all_workers_exited"] is True and data["founder_journal_unchanged"] is True,
            "wrong restore classification")
    world,policy,compatibility = parts(data["binding"])
    require(data["world_id"]=="scownerless:"+world and digest(data["restore_lab_sha256"])
            and set(data["artifact_manifest"])==set(ARTIFACTS)|{"rust-cli"}
            and all(digest(v) for v in data["artifact_manifest"].values())
            and compatibility_for(data["artifact_manifest"])==compatibility,"wrong artifact pin")
    keys = data["participant_keys"]
    require(len(keys)==len(set(keys))==6 and all(digest(k) for k in keys),"wrong participant mapping")
    mapping = [keys[0],keys[5]]
    records,history = data["certified_cuts"],data["history"]
    require([r["label"] for r in records]==LABELS and len(history["records"])==5,
            "missing certified restore history")
    heads = [r["head"] for r in records]
    ids = {hexbytes(v["public_key"]):hexbytes(v["peer_id"]) for v in history["creation"]["genesis"]["validators"]}
    require(set(ids)==set(keys[:3]),"wrong validator roster")
    for i,(row,fence,tick) in enumerate(zip(records,(1,2,3,3,4,4),(0,0,0,1,1,2))):
        head = row["head"]
        require(head["sequence"]==i and head["reference_fence"]==fence
                and head["world_id"]==data["world_id"] and head["policy_hash"]==policy
                and head["compatibility"]==compatibility and head["participation_mode"]=="invite_only"
                and head["majority_threshold"]==2 and row["checkpoint"]["tick"]==tick
                and all(head[k] is False for k in ("runtime_activation","live_quorum_verified",
                    "player_admission","v1_canonical_world_created")),"wrong certified head")
        if i:
            voters = [0,1] if i<=3 else [1,2]
            entry = history["records"][i-1]
            require(row["voter_slots"]==voters and len(entry["votes"])==2
                    and {hexbytes(v["validator"]) for v in entry["votes"]}=={ids[keys[s]] for s in voters},
                    "wrong restore signer set")
            action = entry["proposal"]["action"]
            if i in (3,5):
                require(hexbytes(action["checkpoint"]["commitment"])==head["checkpoint"],"wrong checkpoint decision")
                network = row["network"]
                recipients = [0,2] if i==3 else [2]
                require(network["collector_slot"]==1 and network["voter_slots"]==voters
                        and network["pre_sync_client_access_refused"] is True
                        and [r["recipient_slot"] for r in network["sync"]]==recipients,
                        "wrong restore network observations")
                def listener(obs,kind,slot,field):
                    require(set(obs)=={"classification","address","validator","world_id",field,"runtime_activation"}
                            and obs["classification"]==kind and obs["validator"]=="scpeer:"+ids[keys[slot]]
                            and obs["world_id"]==data["world_id"] and obs[field]==head["head"]
                            and obs["runtime_activation"] is False
                            and re.fullmatch(r"127\.0\.0\.1:[1-9][0-9]{0,4}",obs["address"]) is not None
                            and int(obs["address"].rsplit(":",1)[1])<=65535,"wrong restore listener")
                listener(network["vote_listener"],"ownerless_reference_decision_listener",
                         0 if i==3 else 2,"proposal")
                require(same(network["reviews"],[{"slot":s,"base":heads[i-1]["head"],"proposal":head["head"],
                        "base_checkpoint":heads[i-1]["checkpoint"],"candidate_checkpoint":head["checkpoint"],
                        "replayed_root":row["checkpoint"]["runtime_root"]} for s in voters]),"wrong restore review")
                for obs in network["sync"]:
                    result = obs["result"]
                    receipt = result["receipt"]
                    slot = obs["recipient_slot"]
                    listener(obs["listener"],"ownerless_reference_sync_listener",slot,"head")
                    require(result["classification"]=="ownerless_reference_checkpoint_synced"
                            and all(result[k] is False for k in ("runtime_activation","live_quorum_verified",
                                "canonical_authority","write_authority","worker_admission","player_admission"))
                            and same(receipt["body"],{"world":list(bytes.fromhex(world)),
                                "base":list(bytes.fromhex(heads[i-1]["head"])),"head":list(bytes.fromhex(head["head"])),
                                "sequence":i,"fence":fence,"checkpoint":list(bytes.fromhex(head["checkpoint"])),
                                "recipient":list(bytes.fromhex(ids[keys[slot]]))}),"wrong signed sync receipt")
                    hexbytes(receipt["signature"],64)
            else:
                require("network" not in row and same(row["checkpoint"],records[i-1]["checkpoint"])
                        and head["checkpoint"]==heads[i-1]["checkpoint"],"governance changed checkpoint")
                if i==1:
                    role = action["set_player"]
                    require(role["allowed"] is True and hexbytes(role["player"]["public_key"])==keys[5],
                            "wrong private invitation")
                elif i==2:
                    role = action["set_worker"]
                    require(hexbytes(role["worker"]["public_key"])==keys[3]
                            and role["maximum"]=={"memory_mib":512,"cpu_millicores":1000},"wrong volunteer eligibility")
                else:
                    require(action=="recover","missing certified recovery fence")
    genesis = fixture()
    initial = snapshot(mapping,genesis,0,{"genesis":wire(genesis),"batches":[],"commands":[]})
    require(same(initial,records[0]["checkpoint"]),"wrong genesis state")
    stages = data["stages"]
    require([s["label"] for s in stages]==["first","abandoned","replacement"],"missing restore stage")
    Authority = combined_authority(data["binding"])
    pids,commands = set(),set()
    challenges = set()
    def process(pid):
        require(integer(pid,1,2**31-1) and pid not in pids,"duplicate process observation")
        pids.add(pid)
    def proof(row,slot,seq,cpu=None):
        result = row["result"]
        request = row["request"]
        challenge = request["request"]["challenge"]
        head = heads[seq]
        require(row["subject_slot"]==slot and challenge==row["response"]["challenge"]
                and hexbytes(challenge["subject"]["public_key"])==keys[slot]
                and hexbytes(challenge["verifier"]["public_key"])==keys[1]
                and hexbytes(challenge["verifier"]["peer_id"])==ids[keys[1]]
                and hexbytes(challenge["world"])==world and hexbytes(challenge["head"])==head["head"]
                and challenge["fence"]==head["reference_fence"]
                and hexbytes(challenge["policy"])==policy and hexbytes(challenge["compatibility"])==compatibility
                and hexbytes(challenge["nonce"])!="0"*64
                and challenge["intent"]==("play" if cpu is None else {"volunteer":{"budget":{"memory_mib":512,"cpu_millicores":cpu}}})
                and digest(request["challenge_hash"]) and request["challenge_hash"] not in challenges
                and result["challenge"]==request["challenge_hash"] and result["head"]==head["head"]
                and result["classification"]=="verified_ownerless_reference_participation"
                and result["subject"]=="scpeer:"+hexbytes(challenge["subject"]["peer_id"])
                and result["role"]==("reference_player" if cpu is None else "reference_volunteer")
                and all(result[k] is False for k in FALSE_FLAGS),"wrong restore role observation")
        challenges.add(request["challenge_hash"])
        hexbytes(request["request"]["signature"],64)
        hexbytes(row["response"]["signature"],64)
        if cpu is not None:
            require(result["offered_budget"]=={"memory_mib":512,"cpu_millicores":cpu},"wrong signed budget")
    def cut_view(checkpoint):
        state,_ = validated_state(checkpoint["regions"])
        return transition_view(world,checkpoint["tick"],Authority.cut(world,checkpoint["tick"],
                               checkpoint["runtime_root"],[0,1]),state,[0,1],1)
    for index,(stage,prior,seq,generation,direction) in enumerate(zip(stages,
            (initial,records[3]["checkpoint"],records[3]["checkpoint"]),(2,4,4),(1,2,4),(1,1,-1))):
        candidate = stage["checkpoint"]
        require(review_checkpoint(data["binding"],mapping,prior,candidate)==candidate["runtime_root"],
                "restored stage replay differs")
        if index!=1:
            require(same(candidate,records[3 if index==0 else 5]["checkpoint"]),"stage differs from certified cut")
        require(stage["worker_retired"] is True and stage["uncertified_access_refused"] is True,
                "uncertified access or worker retirement not observed")
        before,after = stage["before"],stage["after"]
        require(before["generation"]==after["generation"]==generation
                and before["tick"]==prior["tick"] and after["tick"]==candidate["tick"]
                and before["runtime_root"]==prior["runtime_root"] and after["runtime_root"]==candidate["runtime_root"]
                and before["world"]==after["world"]==world and same(before["regions"],prior["regions"])
                and same(after["regions"],candidate["regions"])
                and before["cut_root"]==Authority.cut(world,prior["tick"],prior["runtime_root"],[0,1])
                and after["cut_root"]==Authority.cut(world,candidate["tick"],candidate["runtime_root"],[0,1])
                and before["claimed"] is False and before["players"]==after["players"]==[0,1]
                and after["accepted_generation"]==generation and after["claimed"] is True,
                "wrong restored generation or state")
        process(stage["worker_pid"])
        process(stage["client"]["pid"])
        for kernel in (stage["pre_kernel"],stage["post_kernel"]):
            require(kernel["accounting"]=="linux_cgroup_v2_process_charges" and kernel["java_heap_mib"] is None
                    and kernel["kernel_memory_max_bytes"]==80*1024**2
                    and 0<kernel["kernel_memory_current_bytes"]<=kernel["kernel_memory_max_bytes"]
                    and kernel["kernel_memory_current_bytes"]<=kernel["kernel_memory_peak_bytes"]<=kernel["kernel_memory_max_bytes"]
                    and 0<kernel["effective_cpu_millicores"]<=750
                    and kernel["memory_oom_events"]==kernel["memory_oom_kills"]==0,"wrong restore kernel cap")
        proof(stage["player_proof"],5,seq)
        proof(stage["worker_proof"],3,seq,750)
        client = stage["client"]
        request = client["request"]
        row = candidate["transaction_journal"]["commands"][-1]
        require(request["id"] not in commands and request["direction"]==request["action"]==direction
                and request["tick"]==candidate["tick"] and same(request,row["request"]) and row["player"]==1
                and client["submission"]=={"op":"queued","id":request["id"],"tick":request["tick"]}
                and same(client["observed"]["joined"],cut_view(prior))
                and client["observed"]["player"]==1
                and same(client["observed"]["reply"],None if index==1 else row["response"]),
                "wrong staged or certified client response")
        commands.add(request["id"])
    require(not same(stages[1]["checkpoint"],stages[2]["checkpoint"]),"abandoned and replacement cuts identical")
    source = restoration_source_pin(stages[1]["after"])
    expected = {"classification":"ownerless_reference_staging_restored_quarantined",
                "source":source,"world_id":data["world_id"],"head":heads[4]["head"],"checkpoint":heads[4]["checkpoint"],
                "restored_tick":1,"retired_generation":2,"quarantine_generation":3,"discarded_ticks":1,
                "source_preserved":True,"rotation_required":True,"live_quorum_verified":False,"runtime_activation":False,
                "canonical_authority":False,"write_authority":False,"player_admission":False,"worker_admission":False}
    require(same(data["restoration"],expected) and digest(data["source_sha256"])
            and data["source_sha256"]==data["source_sha256_after"]
            and same(data["restored_checkpoint"],records[3]["checkpoint"]),"wrong restoration observation")
    q = data["quarantine"]
    require(q["generation"]==3 and q["accepted_generation"]==1 and q["claimed"] is True and q["tick"]==1
            and q["runtime_root"]==records[3]["checkpoint"]["runtime_root"]
            and q["world"]==world and q["players"]==[0,1]
            and q["cut_root"]==stages[0]["after"]["cut_root"]
            and same(q["regions"],records[3]["checkpoint"]["regions"])
            and data["claim_refusal"]=="generation already claimed; rotate before worker restart"
            and data["stale_refusal"]=="stale ownerless receipt session"
            and data["discarded_receipt_refusal"]=="no exact certified command receipt","wrong quarantine or refusal")
    source_history,restored_history = data["source_history"],data["restored_history"]
    require(len(source_history["commits"])==3 and len(source_history["epochs"])==2
            and [c[1] for c in source_history["commits"]]==[0,1,2]
            and [e[0] for e in source_history["epochs"]]==[1,2]
            and all(digest(c[-1]) for c in source_history["commits"][1:])
            and same(restored_history["commits"],source_history["commits"][:2])
            and len(restored_history["epochs"])==2
            and same(restored_history["epochs"][0],source_history["epochs"][0])
            and same(restored_history["epochs"][1],[3,1,q["runtime_root"],q["grants"],True]),
            "restored history fabricated or retained uncertified suffix")
    for i,cp in enumerate((initial,records[3]["checkpoint"],stages[1]["checkpoint"])):
        row = source_history["commits"][i]
        root = cp["runtime_root"]
        require(row[:5]==[i,i,root,Authority.cut(world,i,root,[0,1]),
                         None if i==0 else source_history["commits"][i-1][2]]
                and (row[5] is None if i==0 else digest(row[5])),"source commit differs from replayed state")
    for i,stage in enumerate(stages[:2],1):
        epoch = source_history["epochs"][i-1]
        require(same(epoch,[i,i-1,stage["before"]["runtime_root"],stage["before"]["grants"],True]),
                "retained worker epoch differs from actual source observation")
    sessions = [g["session"] for e in source_history["epochs"] for g in e[3]]
    sessions += [g["session"] for g in q["grants"]]+[g["session"] for g in stages[2]["after"]["grants"]]
    require(len(set(sessions))==len(sessions) and all(digest(s) for s in sessions),"retired session reused")
    replay = data["recovered_client"]
    process(replay["pid"])
    proof(data["recovered_player_proof"],5,4)
    first = stages[0]["client"]
    require(same(replay["request"],first["request"])
            and same(replay["submission"],first["observed"]["reply"])
            and same(replay["observed"]["reply"],first["observed"]["reply"])
            and same(replay["observed"]["joined"],cut_view(records[3]["checkpoint"])),
            "recovered client receipt differs")
    return {"certified_ticks":2,"abandoned_ticks":1,"restored_tick":1,"workers":3,
            "recovered_receipts":1,"signed_decisions":5,"signed_sync_receipts":3}


def verify(data,cli):
    result = verify_observations(data)
    verify_sync_signatures({**data,"schema":3})
    cli = str(Path(cli).resolve(strict=True))
    with tempfile.TemporaryDirectory(prefix="sc68-restore-verify-") as temp:
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
        cert = write("creation.json",history["creation"])
        require(verify_profile(cli,cert,data["world_id"],parts(data["binding"])[2],absent)==data["binding"],
                "wrong signed creation binding")
        for i,row in enumerate(data["certified_cuts"]):
            prefix = write("history.json",{"creation":history["creation"],"records":history["records"][:i]})
            checkpoint = write("checkpoint.bin",row["checkpoint"])
            require(auth("inspect","--history",prefix,"--expected-world",data["world_id"])==row["head"]
                    and auth("state-hash","--checkpoint",checkpoint)["checkpoint"]==row["head"]["checkpoint"],
                    "signed restored history differs")
        require(not absent.exists(),"read-only verification created a profile")
    return result


if __name__=="__main__":
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("evidence",type=Path)
    p.add_argument("--cli",type=Path,required=True)
    args = p.parse_args()
    print(json.dumps(verify(json.loads(args.evidence.read_bytes()),args.cli),sort_keys=True))
