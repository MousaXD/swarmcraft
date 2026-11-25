#!/usr/bin/env python3
"""Offline replay of #62 reference state, lifecycle and measured admission.

Recorded Linux measurements and process outcomes are observations, not
cryptographic attestations. No private credentials are retained.
"""
import argparse
import json
from pathlib import Path

from player_runtime import fixture
from region_runtime import canonical, digest, fields, integer, region_roots, require, serial_step, totals, world_root
from verify_player_evidence import reference_view
from verify_transition_evidence import cut, RULESET, PLAYERS
from worker_placement import Offer, Sample, MIB, RESERVE_BYTES, choose, region_load

STAGES = [
    ("two_workers", 1, 1, 0, 2, [0, 1], [0, 1], [], [], [1]),
    ("scale_in", 2, 2, 1, 3, [0], [0], [], [], [2]),
    ("worker_loss", 3, 3, 2, 4, [0, 1], [0, 1], [], [1], [3]),
    ("insufficient_after_loss", 4, 4, 3, 4, [0], [], [1], [1], []),
    ("partition", 5, 4, 3, 5, [0], [0], [1], [0, 1], [4]),
    ("recovered", 6, 5, 4, 6, [1], [1], [0], [0], [5, 6]),
]
PHASES = [
    ("tick1", 1, True), ("tick2_withdrawn", 2, False), ("tick2_recovered", 2, True),
    ("tick3", 3, True), ("tick4_worker_killed", 4, False), ("tick4_recovered", 4, True),
    ("tick5_partition", 5, False), ("tick5_recovered", 5, True), ("tick6", 6, True),
]
OBSERVATIONS = [
    ("genesis", 0, 1, "paused", 0, 0),
    ("tick1", 1, 2, "running", 1, 1),
    ("withdrawal_fenced", 1, 2, "draining", 1, 2),
    ("two_workers_released", 1, 2, "paused", 1, 2),
    ("tick2_recovered", 2, 3, "running", 2, 2),
    ("scale_in_released", 2, 3, "paused", 2, 2),
    ("all_player_only", 2, 3, "paused", 2, 2),
    ("tick3", 3, 4, "running", 3, 3),
    ("tick4_worker_killed", 3, 4, "draining", 3, 4),
    ("worker_loss_released", 3, 4, "paused", 3, 4),
    ("insufficient_after_loss_released", 3, 4, "paused", 4, 4),
    ("tick4_recovered", 4, 5, "running", 5, 4),
    ("tick5_partition", 4, 5, "draining", 5, 5),
    ("partition_released", 4, 5, "paused", 5, 5),
    ("all_workers_unavailable", 4, 5, "paused", 5, 5),
    ("tick5_recovered", 5, 6, "running", 6, 5),
    ("tick6", 6, 6, "running", 6, 6),
    ("recovered_released", 6, 6, "paused", 6, 6),
]
# Explicit fixture consent choices at each stage entry.
OFFERS = [(2,56,2,56), (3,96,3,0), (5,56,4,56), (5,56,4,56), (6,96,4,56), (6,96,5,96)]


def expected_offers(a, am, b, bm):
    return [Offer(0,a,am,1000 if am else 0), Offer(1,b,bm,1000 if bm else 0),
            Offer(2,1,0,0), Offer(3,1,0,0)]


def kernel(k, offer):
    cap = offer.process_memory_mib*MIB
    require(k["kernel_memory_max_bytes"] == cap
            and 0 < k["kernel_memory_high_bytes"] <= cap
            and 0 <= k["kernel_memory_current_bytes"] <= min(k["kernel_memory_high_bytes"],cap-RESERVE_BYTES)
            and 0 < k["effective_cpu_millicores"] <= offer.cpu_millicores
            and k["memory_oom_events"] == k["memory_oom_kills"] == 0
            and k["java_heap_mib"] is None and k["accounting"] == "linux_cgroup_v2_process_charges",
            "worker kernel accounting invalid")


def verify(data):
    fields(data, "schema", "classification", "stages", "observations", "clients", "faults",
           "pauses", "refusals", "events", "final", "journal", "all_scopes_released")
    require(data["schema"] == 1 and data["classification"] ==
            "LOCAL_DURABLE_CAPPED_RECOVERY_NOT_MINECRAFT_NOT_QUORUM"
            and data["all_scopes_released"] is True, "wrong classification")
    initial = fixture()
    states = [initial]
    for t in range(1,7):
        d = 1 if t%2 else -1
        states.append(serial_step(states[-1],t,controls={0:(d,d),1:(d,d),2:(0,0)}))
    roots = [world_root(t,region_roots(s)) for t,s in enumerate(states)]
    ids = sorted(a[0] for r in initial.values() for a in r.agents)
    resources = totals(region_roots(initial))[1]
    world = data["final"]["world"]
    require(digest(world), "invalid world")
    cuts = [cut(world,t,r) for t,r in enumerate(roots)]

    def view(t,p):
        return {**reference_view(world,t,cuts[t],states[t],PLAYERS,p), "ruleset":RULESET,"protocol":2}

    commands, pids, command_ids = {}, set(), set()
    def pid(p):
        require(integer(p,1,2**31-1) and p not in pids,"reused process ID")
        pids.add(p)

    def receipt(t,p):
        return {"op":"committed","id":commands[t,p]["id"],"view":view(t,p),
                "inventory_delta":view(t,p)["inventory"]-view(t-1,p)["inventory"]}

    require(len(data["clients"]) == len(PHASES),"missing clients")
    for row,(label,t,accepted) in zip(data["clients"],PHASES):
        require(row["label"] == label and len(row["sessions"])==2,"wrong client phase")
        for p,session in enumerate(row["sessions"]):
            pid(session["pid"])
            req,observed = session["request"],session["observed"]
            if (t,p) not in commands:
                d=1 if t%2 else -1
                require(digest(req["id"]) and req["id"] not in command_ids and
                        req=={"op":"act","world":world,"id":req["id"],"tick":t,"parent":cuts[t-1],
                              "direction":d,"action":d},"wrong command")
                commands[t,p]=req
                command_ids.add(req["id"])
            require(req==commands[t,p] and session["submission"]==
                    {"op":"queued","id":req["id"],"tick":t}
                    and observed=={"player":p,"joined":view(t-1,p),
                                   "reply":receipt(t,p) if accepted else None},"wrong client outcome")

    all_grants, sessions = {}, set()
    require(len(data["stages"])==len(STAGES),"missing stage")
    for row, spec, choice in zip(data["stages"],STAGES,OFFERS):
        label,scope,prior,t,gen,probes,selected,blocked,after_blocked,ticks=spec
        start,end,drain=row["start"],row["released"],row["before_cleanup"]
        opts=expected_offers(*choice)
        require(row["label"]==label and row["generation"]==gen
                and (start["phase"],start["scope"],start["generation"],start["tick"],start["root"])==
                ("probing",scope,prior,t,roots[t])
                and start["probes"]==probes and start["blocked"]==blocked and not start["assignment"]
                and [Offer(**o) for o in start["offers"]]==opts,"incorrect stage start")
        require(set(row["probes"])=={str(s) for s in probes},"wrong probe slots")
        samples={}
        for s in probes:
            observation=row["probes"][str(s)]
            k=observation["kernel"]
            kernel(k,opts[s])
            sample=Sample(**observation["sample"])
            rtt=observation["echo_rtt_ms"]
            require(len(rtt)==6 and all(type(v) in (int,float) and v>0 for v in rtt)
                    and sample.rtt_us==int(max(rtt)*1000+0.999)
                    and sample.verified_enforcement is True and sample.effective_cpu_millicores==k["effective_cpu_millicores"]
                    and sample.usable_memory_bytes==min(k["kernel_memory_high_bytes"],
                                                        k["kernel_memory_max_bytes"]-RESERVE_BYTES)
                    and observation["pre_admission_phase"]=="new"
                    and observation["veth_rx_bytes"]>0 and observation["veth_tx_bytes"]>0,"invalid probe")
            samples[s]=sample
        decision=choose(opts,samples,region_load(states[t]),row["decision_ns"])
        require(row["decision"]==decision,"planner replay differs")
        plan=decision["plan"]
        require((sorted(set(plan["owner_by_region"])) if plan else [])==selected,"wrong admission")
        require([v["slot"] for v in row["workers"]]==probes
                and len({v["namespace"] for v in row["workers"]} | {row["coordinator_namespace"]})==len(probes)+1,
                "wrong worker isolation")
        for v in row["workers"]:
            pid(v["pid"])
        require(len(row["cleanup"])==len(probes) and row["leaves_removed"] is True
                and row["network_clean"] is True,"incomplete budget release")
        for v,c in zip(row["workers"],row["cleanup"]):
            s=v["slot"]
            allowed=(-9,) if label=="worker_loss" and s==1 else ((0,-9) if label=="partition" else (0,))
            require(c["slot"]==s and c["pid"]==v["pid"] and c["populated"]==0
                    and c["exitcode"] in allowed and c["memory_mib"]==opts[s].process_memory_mib
                    and c["cpu_millicores"]==opts[s].cpu_millicores,"cleanup not verified")
        require([x["tick"] for x in row["ticks"]]==ticks,"incorrect accepted ticks")
        for x in row["ticks"]:
            require(x["root"]==roots[x["tick"]],"incorrect accepted root")
            for key in ("kernel_before","kernel_after"):
                require(len(x[key])==len(selected),"missing kernel guards")
                for k,s in zip(x[key],selected):
                    kernel(k,opts[s])
        end_tick=ticks[-1] if ticks else t
        require((drain["phase"],drain["scope"],drain["generation"],drain["tick"],drain["root"])==
                ("draining",scope,gen,end_tick,roots[end_tick])
                and drain["probes"]==probes and drain["blocked"]==after_blocked
                and drain["assignment"]==[[s,opts[s].revision] for s in selected],"drain mismatch")
        require(end=={**drain,"phase":"paused","probes":[],"assignment":[]},"scope not released")
        if selected:
            grants=row["initial_grants"]
            require(len(grants)==len(selected),"missing grants")
            for grant,s in zip(grants,selected):
                require(grant=={"world":world,"generation":gen,"base_tick":t,"base_root":roots[t],
                                "regions":[r for r,owner in enumerate(plan["owner_by_region"]) if owner==s],
                                "session":grant["session"]}
                        and digest(grant["session"]) and grant["session"] not in sessions,"incorrect grant")
                sessions.add(grant["session"])
            all_grants[gen]=grants
        else:
            require("initial_grants" not in row and not row["ticks"],"infeasible plan ran work")
    require(data["stages"][0]["withdrawal_workers_alive"] is True,"withdrawal overlap not tested")

    require(len(data["observations"])==len(OBSERVATIONS),"missing checkpoint")
    observed={}
    for row,(label,t,gen,phase,scope,jt) in zip(data["observations"],OBSERVATIONS):
        save,status,journal=row["save"],row["status"],row["journal"]
        accepted_gen={0:0,1:2,2:3,3:4,4:5,5:6,6:6}[t]
        require(row["label"]==label and save["world"]==world and save["ruleset"]==RULESET
                and save["players"]==PLAYERS and save["ids"]==ids and save["resources"]==resources
                and (save["generation"],save["accepted_generation"],save["tick"])==(gen,accepted_gen,t)
                and save["runtime_root"]==roots[t] and save["cut_root"]==cuts[t]
                and canonical(save["regions"])==canonical([states[t][r].wire() for r in range(8)]),
                "durable checkpoint differs from replay")
        require(save["claimed"] is (gen!=1),"wrong durable generation claim")
        if gen!=1:
            require(save["grants"]==all_grants[gen],"grant changed")
        require((status["phase"],status["scope"],status["generation"],status["tick"],status["root"])==
                (phase,scope,gen,t,roots[t]),"wrong checkpoint status")
        if phase=="paused":
            require(not status["probes"] and not status["assignment"],"paused work")
        require(canonical(journal["genesis"])==canonical([initial[r].wire() for r in range(8)]),"bad genesis")
        expected_batches,expected_commands=[],[]
        for i in range(1,jt+1):
            batch=[[p,commands[i,p]["direction"],commands[i,p]["action"],commands[i,p]["id"]]
                   for p in (0,1)]+[[2,0,0,None]]
            expected_batches.append({"tick":i,"parent":cuts[i-1],"batch":batch,"result":cuts[i] if i<=t else None})
            expected_commands.extend({"player":p,"request":commands[i,p],"response":receipt(i,p) if i<=t else None}
                                     for p in (0,1))
        require(journal["batches"]==expected_batches and journal["commands"]==expected_commands,
                "incorrect durable input journal")
        observed[label]=row
    require(data["final"]==data["observations"][-1]["save"] and data["journal"]==data["observations"][-1]["journal"],
            "final cut mismatch")
    for row,label in zip(data["pauses"],("all_player_only","all_workers_unavailable")):
        require(row=={"label":label,"status":observed[label]["status"],"spawned_slots":[]}
                and not any(o["process_memory_mib"] and o["slot"] not in row["status"]["blocked"]
                            for o in row["status"]["offers"]),"invalid pause")
    require(len(data["pauses"])==2,"missing pauses")
    expected_faults=[("tick4_worker_killed","worker_loss","prepared",1,3,[1],2),
                     ("tick5_partition","partition","sealed",0,4,[0,1],4)]
    require(len(data["faults"])==2,"missing fault")
    for row,(label,kind,phase,s,t,blocked,index) in zip(data["faults"],expected_faults):
        stage=data["stages"][index]
        require(row=={"label":label,"kind":kind,"phase":phase,"slot":s,
                      "pid":next(v["pid"] for v in stage["workers"] if v["slot"]==s),
                      "last_tick":t,"last_root":roots[t],"frozen":True,"blocked":blocked}
                and stage["fault_error"] in ("EOFError","TimeoutError","ConnectionResetError","BrokenPipeError","OSError"),
                "wrong fault boundary")
    reasons=[("withdrawn_generation_candidate","contribution scope fenced"),
             ("withdrawn_generation_seal","retired runtime cannot seal input"),
             ("start_before_cleanup","previous scope not released"),
             ("incomplete_cleanup","scope cleanup incomplete or stale")]
    require(data["refusals"]==[{"label":l,"reason":r,"tick":1,"generation":2,"root":roots[1]} for l,r in reasons],
            "missing refusal")

    events=data["events"]
    expected_events=["consent_changed","begin_probe","admitted","claimed","consent_changed","released",
                     "begin_probe","admitted","claimed","consent_changed","released",
                     "consent_changed","begin_probe","admitted","claimed","worker_loss","released",
                     "begin_probe","no_feasible_placement","released","consent_changed",
                     "begin_probe","admitted","claimed","partition","released","consent_changed",
                     "begin_probe","admitted","claimed","stop","released"]
    require([e["event"] for e in events]==expected_events,"wrong lifecycle ordering")
    for row in data["stages"]:
        scope=row["start"]["scope"]
        scoped=[e for e in events if e["scope"]==scope]
        def event_state(e):
            return {k:v for k,v in e.items() if k!="event"}
        for selected, observed_state in (
            ([e for e in scoped if e["event"]=="begin_probe"], row["start"]),
            ([e for e in scoped if e["phase"]=="draining"], row["before_cleanup"]),
            ([e for e in scoped if e["event"]=="released"], row["released"]),
        ):
            require(len(selected)==1 and event_state(selected[0])==observed_state,
                    "lifecycle history differs from observed state")
    for e in events:
        require(integer(e["tick"],0,6) and e["root"]==roots[e["tick"]],"invalid lifecycle cut")
    return {"accepted_ticks":6,"unique_commands":12,"client_sessions":18,"worker_processes":8,
            "scopes_released":6,"faults":2,"paused_without_capacity":3,"final_root":roots[6]}


if __name__=="__main__":
    parser=argparse.ArgumentParser()
    parser.add_argument("receipt",type=Path)
    args=parser.parse_args()
    print(json.dumps(verify(json.loads(args.receipt.read_text())),indent=2))
