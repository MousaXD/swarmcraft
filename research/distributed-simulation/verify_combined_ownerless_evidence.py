#!/usr/bin/env python3
"""Check #68 retained local observations and signed history, not live attestation."""
import argparse
import json
from pathlib import Path
import subprocess
import tempfile

from combined_ownerless_lab import (ARTIFACTS, CLASSIFICATION, FALSE_FLAGS, FULL, HALVES,
                                    combined_authority, compatibility_for, sha, snapshot)
from ownerless_permission import parts, verify_profile
from player_runtime import fixture
from regional_authority import wire
from region_runtime import canonical, digest, integer, region_roots, require, serial_step, world_root
from transition_runtime import response, transition_view
from worker_placement import Offer, Sample, assess, evaluate, region_load

LABELS = ["genesis", "invite_founder", "invite_surviving_player", "allow_founder_worker",
          "allow_surviving_worker", "allow_extra_worker", "founder-checkpoint",
          "remove_founder_invitation", "retire_founder_compute", "founder_independent_recovery",
          "surviving_quorum_scale_out-checkpoint", "withdraw_extra_worker", "scale_in_recovery",
          "surviving_quorum_scale_in-checkpoint"]
STAGES = [
    ("founder", 5, 6, [(0, 0, 1, 96, 1000)], [FULL], [(0, 0), (1, 5)], 1),
    ("surviving_quorum_scale_out", 9, 10, [(1, 3, 1, 96, 1000), (2, 4, 1, 96, 1000)], HALVES, [(1, 5)], -1),
    ("surviving_quorum_scale_in", 12, 13, [(1, 3, 2, 80, 750)], [FULL], [(1, 5)], 1),
]
TICKS = [0]*6 + [1]*4 + [2]*3 + [3]
PLAYERS = [0, 1]


def same(a, b):
    return canonical(a) == canonical(b)


def hexbytes(value, size=32):
    require(isinstance(value, list) and len(value) == size and
            all(integer(n, 0, 255) for n in value), "invalid byte array")
    return bytes(value).hex()


def verify_observations(data):
    require(data["schema"] == 1 and data["classification"] == CLASSIFICATION
            and data["physical_machines"] == 1 and data["live_minecraft_players"] == 0
            and data["canonical_minecraft_authority"] is False and data["runtime_activation"] is False
            and data["actual_kernel_caps"] is True and data["all_workers_exited"] is True
            and data["founder_journal_unchanged_after_departure"] is True, "wrong combined classification")
    world, policy, compatibility = parts(data["binding"])
    require(data["world_id"] == "scownerless:"+world, "wrong world pin")
    manifest = data["artifact_manifest"]
    require(set(manifest) == set(ARTIFACTS) | {"rust-cli"} and all(digest(v) for v in manifest.values())
            and compatibility_for(manifest) == compatibility, "artifact compatibility differs")
    keys = data["participant_keys"]
    require(len(keys) == len(set(keys)) == 6 and all(digest(k) for k in keys), "wrong participant keys")
    mapping = [keys[0], keys[5]]
    records, history = data["certified_cuts"], data["history"]
    require([r["label"] for r in records] == LABELS and len(history["records"]) == 13,
            "missing certified decision")
    require(len(data["stages"]) == 3, "missing stage")
    validators = history["creation"]["genesis"]["validators"]
    ids = {hexbytes(v["public_key"]): hexbytes(v["peer_id"]) for v in validators}
    require(set(ids) == set(keys[:3]) and len(validators) == 3, "wrong genesis validator roster")
    heads = [r["head"] for r in records]
    actions = [r["proposal"]["action"] for r in history["records"]]

    def role_action(seq, kind, slot, field, value):
        action = actions[seq-1]
        require(set(action) == {kind}, "wrong role decision")
        inner = action[kind]
        subject = inner["player" if kind == "set_player" else "worker"]
        require(hexbytes(subject["public_key"]) == keys[slot] and inner[field] == value,
                "wrong role subject or decision")

    cap = {"memory_mib":512,"cpu_millicores":1000}
    for seq, slot, allowed in [(1,0,True),(2,5,True),(7,0,False)]:
        role_action(seq, "set_player", slot, "allowed", allowed)
    for seq, slot, maximum in [(3,0,cap),(4,3,cap),(5,4,cap),(8,0,None),(11,4,None)]:
        role_action(seq, "set_worker", slot, "maximum", maximum)
    require(actions[8] == actions[11] == "recover", "missing founder-independent recovery")
    require(len({h["head"] for h in heads}) == 14, "duplicate certified heads")
    fence = 1
    invited, workers = 0, 0
    for seq, row in enumerate(records):
        head = row["head"]
        if seq:
            entry = history["records"][seq-1]
            voters = [0,1] if seq <= 6 else [1,2]
            require(row["voter_slots"] == voters and
                    {hexbytes(v["validator"]) for v in entry["votes"]} == {ids[keys[i]] for i in voters}
                    and len(entry["votes"]) == 2, "wrong signer set after departure")
            if seq in (6,10,13):
                require(hexbytes(entry["proposal"]["action"]["checkpoint"]["commitment"]) == head["checkpoint"],
                        "checkpoint decision differs")
            else:
                fence += 1
                require(head["checkpoint"] == heads[seq-1]["checkpoint"], "role/recovery altered checkpoint")
            invited += {1:1,2:1,7:-1}.get(seq,0)
            workers += {3:1,4:1,5:1,8:-1,11:-1}.get(seq,0)
        require(head["world_id"] == data["world_id"] and head["sequence"] == seq
                and head["reference_fence"] == fence and head["compatibility"] == compatibility
                and head["policy_hash"] == policy and head["majority_threshold"] == 2
                and head["participation_mode"] == "invite_only"
                and head["invited_player_count"] == invited and head["eligible_worker_count"] == workers
                and head["runtime_activation"] is False and head["live_quorum_verified"] is False
                and head["player_admission"] is False and head["v1_canonical_world_created"] is False
                and digest(head["head"]) and digest(head["checkpoint"]), "wrong reference head")

    # Role proof logs are observations of the actual CLI verifier. The full
    # authority history below is cryptographically reverified separately.
    challenges = set()
    def proof(row, slot, seq, cpu=None, refused=False):
        q, resp, result = row["request"], row["response"], row["result"]
        challenge = q["request"]["challenge"]
        head = heads[seq]
        require(row["subject_slot"] == slot and challenge == resp["challenge"]
                and hexbytes(challenge["subject"]["public_key"]) == keys[slot]
                and hexbytes(challenge["verifier"]["public_key"]) == keys[1]
                and hexbytes(challenge["verifier"]["peer_id"]) == ids[keys[1]]
                and hexbytes(challenge["world"]) == world and hexbytes(challenge["head"]) == head["head"]
                and hexbytes(challenge["policy"]) == policy and hexbytes(challenge["compatibility"]) == compatibility
                and challenge["fence"] == head["reference_fence"]
                and hexbytes(challenge["nonce"]) != "0"*64 and digest(q["challenge_hash"])
                and q["challenge_hash"] not in challenges, "wrong or replayed role challenge")
        challenges.add(q["challenge_hash"])
        hexbytes(q["request"]["signature"],64)
        hexbytes(resp["signature"],64)
        budget = {"memory_mib":512,"cpu_millicores":cpu}
        require(challenge["intent"] == ("play" if cpu is None else {"volunteer":{"budget":budget}}),
                "wrong consent intent")
        if refused:
            require(result == "worker has no accepted eligibility", "withdrawal not refused")
            return
        require(result["world_id"] == data["world_id"] and result["head"] == head["head"]
                and result["challenge"] == q["challenge_hash"]
                and result["subject"] == "scpeer:"+hexbytes(challenge["subject"]["peer_id"])
                and result["classification"] == "verified_ownerless_reference_participation"
                and result["role"] == ("reference_player" if cpu is None else "reference_volunteer")
                and all(result[f] is False for f in FALSE_FLAGS)
                and result["validator_already_enrolled"] is (slot < 3)
                and (cpu is None or result["offered_budget"] == budget), "wrong role result")

    state = fixture()
    audit = {"genesis":wire(state),"batches":[],"commands":[]}
    snapshots = [snapshot(mapping,state,0,audit)]
    # Copy snapshots so subsequent replay cannot mutate earlier journals.
    snapshots[0] = json.loads(canonical(snapshots[0]))
    Authority = combined_authority(data["binding"])
    root = world_root(0,region_roots(state))
    cut = Authority.cut(world,0,root,PLAYERS)
    pids, sessions, requests = set(), set(), set()
    prior_save = None
    def pid(p):
        require(integer(p,1,2**31-1) and p not in pids,"duplicate process observation")
        pids.add(p)

    def kernel(obs, memory, cpu):
        require(obs["accounting"] == "linux_cgroup_v2_process_charges"
                and obs["java_heap_mib"] is None and obs["kernel_memory_max_bytes"] == memory*1024**2
                and 0 < obs["kernel_memory_current_bytes"] <= obs["kernel_memory_max_bytes"]
                and obs["kernel_memory_current_bytes"] <= obs["kernel_memory_peak_bytes"] <= obs["kernel_memory_max_bytes"]
                and 0 < obs["effective_cpu_millicores"] <= cpu
                and obs["memory_oom_events"] == obs["memory_oom_kills"] == 0, "wrong kernel observation")

    for tick, (stage, spec) in enumerate(zip(data["stages"],STAGES),1):
        label, base, accepted, offers, groups, players, d = spec
        before, after = stage["before"], stage["after"]
        require(stage["label"] == label and stage["generation"] == tick
                and stage["base_head"] == heads[base]["head"] and stage["accepted_head"] == heads[accepted]["head"]
                and stage["worker_cleanup"] is True and stage["all_worker_privilege_requests_refused"] is True,
                "wrong stage observations")
        require(before["world"] == after["world"] == world and before["tick"] == tick-1
                and before["runtime_root"] == root and before["cut_root"] == cut
                and same(before["regions"],wire(state)) and (prior_save is None or before == prior_save),
                "stage did not resume previous cut")
        require(stage.get("stale_receipt_refusal") == (None if tick == 1 else "stale ownerless receipt session"),
                "wrong retired-session result")
        expected_workers = [{"slot":s,"subject_slot":k,"revision":r,"memory_mib":m,"cpu_millicores":c,
                             "regions":g} for (s,k,r,m,c),g in zip(offers,groups)]
        require(stage["workers"] == expected_workers and
                all(len(stage[k]) == len(offers) for k in
                    ("worker_pids","worker_namespaces","worker_proofs","pre_kernel","post_kernel")),
                "wrong worker arrangement")
        require(len(set(stage["worker_namespaces"]+[stage["coordinator_namespace"]])) == len(offers)+1,
                "workers lack distinct network namespaces")
        require(set(stage["probes"]) == {str(o[0]) for o in offers}, "wrong probe set")
        samples = {}
        for i,(slot,subject,rev,mem,cpu) in enumerate(offers):
            pid(stage["worker_pids"][i])
            proof(stage["worker_proofs"][i],subject,base,cpu)
            sample = Sample(**stage["probes"][str(slot)]["sample"])
            require(assess(Offer(slot,rev,mem,cpu),sample,stage["decision_time_ns"]) == "eligible",
                    "recorded worker was not eligible")
            samples[slot] = sample
            for obs in (stage["probes"][str(slot)]["kernel"],stage["pre_kernel"][i],stage["post_kernel"][i]):
                kernel(obs,mem,cpu)
        owner = [next(o[0] for o,g in zip(offers,groups) if r in g) for r in FULL]
        require(stage["plan"] == evaluate(owner,region_load(state),samples), "placement replay differs")
        require(len(stage["clients"]) == len(stage["player_proofs"]) == len(players),"wrong client count")
        next_state = serial_step(state,tick,controls={p:(d,d) if p in dict(players) else (0,0) for p in PLAYERS})
        next_root = world_root(tick,region_roots(next_state))
        next_cut = Authority.cut(world,tick,next_root,PLAYERS)
        batch = [[p,0,0,None] for p in PLAYERS]
        for i,(p,subject) in enumerate(players):
            proof(stage["player_proofs"][i],subject,base)
            row = stage["clients"][i]
            pid(row["pid"])
            req, obs = row["request"],row["observed"]
            joined = transition_view(world,tick-1,cut,state,PLAYERS,p)
            require(digest(req["id"]) and req["id"] not in requests
                    and req == {"op":"act","world":world,"id":req["id"],"tick":tick,"parent":cut,
                                "direction":d,"action":d}, "wrong client command")
            requests.add(req["id"])
            reply = response(world,tick,next_cut,next_state,PLAYERS,p,req,joined["inventory"])
            require(obs == {"joined":joined,"player":p,"reply":reply}
                    and row["submission"] == {"op":"queued","id":req["id"],"tick":tick},
                    "client acknowledgement replay differs")
            if tick == 2:
                require(joined["region"] == 0 and reply["view"]["region"] == 7
                        and stage["player_cross_worker"] == [owner[0],owner[7]] == [1,2],
                        "surviving player did not cross workers")
            batch[p] = [p,d,d,req["id"]]
            audit["commands"].append({"player":p,"request":req,"response":reply})
        audit["batches"].append({"tick":tick,"parent":cut,"batch":batch,"result":next_cut})
        require(after["tick"] == tick and after["runtime_root"] == next_root and after["cut_root"] == next_cut
                and after["generation"] == after["accepted_generation"] == tick and after["claimed"] is True
                and same(after["regions"],wire(next_state)), "accepted state replay differs")
        grants = after["grants"]
        require([g["regions"] for g in grants] == groups, "wrong assigned regions")
        for g in grants:
            require(g == {"world":world,"generation":tick,"regions":g["regions"],
                          "session":g["session"],"base_tick":tick-1,"base_root":root}
                    and digest(g["session"]) and g["session"] not in sessions, "wrong or reused regional grant")
            sessions.add(g["session"])
        snapshots.append(json.loads(canonical(snapshot(mapping,next_state,tick,audit))))
        prior_save = after
        state, root, cut = next_state,next_root,next_cut
    for seq, row in enumerate(records):
        require(same(row["checkpoint"],snapshots[TICKS[seq]]), "certified checkpoint replay differs")
    proof(data["withdrawal_refusal"],4,11,1000,True)
    minority = data["minority"]
    copy = minority["copy_report"]
    raw = canonical(snapshots[-1])
    require(data["final_tick"] == 3 and data["final_head"] == heads[-1]
            and minority["refusal"] == "missing current validator majority"
            and hexbytes(minority["vote"]["validator"]) == ids[keys[1]]
            and minority["proposal"]["proposal"]["action"] == "recover"
            and minority["copy_sha256"] == sha(raw)
            and copy == {"bytes":len(raw),"checkpoint":heads[-1]["checkpoint"],
                         "classification":"ownerless_reference_recovery_copy_only","head":heads[-1]["head"],
                         "world_id":data["world_id"],"runtime_activation":False,"write_authority":False},
            "minority recovery copy or refusal differs")
    return {"ticks":3,"decisions":13,"workers":len(sessions),"client_receipts":len(requests),
            "role_proofs":len(challenges)}


def verify(data, cli):
    result = verify_observations(data)
    cli = str(Path(cli).resolve())
    with tempfile.TemporaryDirectory(prefix="sc68-verify-") as directory:
        root = Path(directory)
        absent = root/"absent"
        def write(name,value):
            path = root/name
            path.write_bytes(canonical(value))
            return str(path)
        def auth(*args):
            run = subprocess.run([cli,"--data-dir",str(absent),"ownerless","authority",*args],
                                 capture_output=True,check=True,timeout=20)
            return json.loads(run.stdout)
        history = data["history"]
        certificate = write("creation.json",history["creation"])
        require(verify_profile(cli,certificate,data["world_id"],parts(data["binding"])[2],absent) == data["binding"],
                "signed creation binding differs")
        for i,row in enumerate(data["certified_cuts"]):
            prefix = write("history.json",{"creation":history["creation"],"records":history["records"][:i]})
            head = auth("inspect","--history",prefix,"--expected-world",data["world_id"])
            checkpoint = write("checkpoint.bin",row["checkpoint"])
            require(head == row["head"] and auth("state-hash","--checkpoint",checkpoint)["checkpoint"] == head["checkpoint"],
                    "cryptographic history/checkpoint verification differs")
        require(not absent.exists(), "read-only verification created a profile")
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("evidence",type=Path)
    parser.add_argument("--cli",type=Path,required=True)
    args = parser.parse_args()
    print(json.dumps(verify(json.loads(args.evidence.read_bytes()),args.cli),sort_keys=True))


if __name__ == "__main__":
    main()
