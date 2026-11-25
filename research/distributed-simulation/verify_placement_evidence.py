#!/usr/bin/env python3
"""Recompute #58 lab roots, decisions and receipt invariants, read-only.

This verifies retained data consistency with the lab model. It is not
cryptographic attestation of historical execution or production authority.
"""
from __future__ import annotations

import argparse
import copy
import json
import math
from pathlib import Path

from network_lab import duration_summary, percentile
from placement_lab import CLASSIFICATION, MACHINE_RESERVE
from region_runtime import Rejected, initial_world, region_roots, require, serial_step, world_root
from worker_placement import (ConsentLedger, DEADLINE_US, MIB, Offer, Sample,
                              RESERVE_BYTES, choose, evaluate, region_load)

ZERO = lambda slot,revision=1: Offer(slot,revision,0,0)


def choices(*active):
    by_slot = {o.slot:o for o in active}
    return [by_slot.get(s,ZERO(s)) for s in range(4)]


EXPECTED = {
    "one_worker": (0,12,choices(Offer(0,1,96,1000)),{0}),
    "one_worker_comparison": (12,32,choices(Offer(0,1,96,1000)),{0}),
    "scale_out": (12,32,choices(Offer(0,1,96,1000),Offer(1,2,96,1000),
                               Offer(2,2,32,250),Offer(3,2,96,1000)),{0,1}),
    "scale_in": (44,12,choices(Offer(0,2,80,1000),ZERO(1,3),ZERO(2,3),ZERO(3,3)),{0}),
    "memory_blocked": (56,8,choices(Offer(0,3,32,1000),ZERO(1,3),ZERO(2,3),ZERO(3,3)),set()),
    "cpu_blocked": (56,8,choices(Offer(0,4,96,250),ZERO(1,3),ZERO(2,3),ZERO(3,3)),set()),
    "all_player_only": (56,8,choices(ZERO(0,5),ZERO(1,3),ZERO(2,3),ZERO(3,3)),set()),
    "clustered": (0,16,choices(Offer(0,1,96,1000),Offer(1,1,96,1000)),None),
}


def verify_kernel(row, offer):
    cap = offer.process_memory_mib*MIB
    high = row["kernel_memory_high_bytes"]
    current = row["kernel_memory_current_bytes"]
    peak = row["kernel_memory_peak_bytes"]
    require(row["accounting"]=="linux_cgroup_v2_process_charges"
            and row["java_heap_mib"] is None and row["kernel_memory_max_bytes"]==cap
            and 89*cap//100 <= high <= 9*cap//10
            and 0 < current <= min(high,cap-RESERVE_BYTES)
            and (peak is None or current <= peak <= cap)
            and 0 < row["effective_cpu_millicores"] <= offer.cpu_millicores
            and row["cpu_usage_usec"]>0 and row["throttled_periods"]>=0
            and row["memory_oom_events"]==row["memory_oom_kills"]==0,
            "kernel observation inconsistent with offer")


def verify_decision(row, state, tick, offers, slow_slots=()):
    require(row["start_tick"]==tick and row["start_root"]==world_root(tick,region_roots(state))
            and row["load"]==region_load(state)
            and [Offer(**o) for o in row["offers"]]==offers,
            "incorrect starting cut, load or consent")
    positive = {o.slot:o for o in offers if o.opted_in}
    require(row["spawned_slots"]==sorted(positive)
            and set(row["probes"])=={str(s) for s in positive},
            "player-only or missing probe")
    samples = {}
    if positive:
        machine = row["machine"]
        require(machine["coordinator_cpu_reserve_millicores"]==1000
                and machine["machine_memory_reserve_bytes"]==MACHINE_RESERVE
                and machine["effective_cpu_millicores"]-1000>=sum(o.cpu_millicores for o in positive.values())
                and machine["available_memory_bytes"]-MACHINE_RESERVE>=sum(o.process_memory_mib*MIB for o in positive.values()),
                "unsafe shared capacity receipt")
    for slot,o in positive.items():
        probe = row["probes"][str(slot)]
        verify_kernel(probe["kernel"],o)
        sample = Sample(**probe["sample"])
        sample.validate()
        ping = probe["echo_rtt_ms"]
        expected_delay = 25 if slot in slow_slots else 0
        require(len(ping)==6 and all(math.isfinite(t) and t>0 for t in ping)
                and sample.slot==slot and sample.revision==o.revision and sample.verified_enforcement
                and sample.effective_cpu_millicores==probe["kernel"]["effective_cpu_millicores"]
                and sample.usable_memory_bytes==min(probe["kernel"]["kernel_memory_high_bytes"],
                                                     o.process_memory_mib*MIB-RESERVE_BYTES)
                and sample.rtt_us==int(percentile(ping,95)*1000+0.999)
                and probe["pre_admission_phase"]=="new" and probe["veth_rx_bytes"]>0
                and probe["veth_tx_bytes"]>0 and probe["one_way_netem_delay_ms"]==expected_delay
                and all((probe[d+"_netem_packets"]>0)==bool(expected_delay) for d in ("coordinator","worker")),
                "invalid probe or kernel path evidence")
        samples[slot] = sample
    decision = choose(offers,samples,region_load(state),row["decision_ns"])
    require(row["decision"]==decision,"recorded decision differs from bounded planner replay")
    plan = decision["plan"]
    selected = set(plan["owner_by_region"]) if plan else set()
    require(row.get("probe_only_slots_exited_before_init",[])==sorted(set(positive)-selected),
            "unselected probe not retired before work")
    return samples,selected


def verify(report):
    require(report["classification"]==CLASSIFICATION and report["completed"] is True
            and report["hardware"]["physical_machine_count"]==1
            and report["hardware"]["actual_kernel_caps"] is True
            and len(set(report["hardware"]["worker_cpus"]+[report["hardware"]["coordinator_cpu"]]))==5
            and set(report["stages"])==set(EXPECTED),"incomplete or misclassified lab")
    # Independently traverse the same world from genesis; no trust in a
    # worker's self-reported checkpoint or a prior stage's output root.
    initial = initial_world("spread",16384,58)
    states = {0:initial}
    for tick in range(1,57):
        states[tick] = serial_step(states[tick-1],tick)
    cluster_states = {0:initial_world("clustered",16384,59)}
    for tick in range(1,17):
        cluster_states[tick] = serial_step(cluster_states[tick-1],tick)
    ledger=ConsentLedger()
    for name,(start,count,opts,expected_workers) in EXPECTED.items():
        row=report["stages"][name]
        ref=cluster_states if name=="clustered" else states
        if name!="clustered":
            ledger.accept(opts)
        samples,selected=verify_decision(row,ref[start],start,opts,(3,) if name=="scale_out" else ())
        require(selected==expected_workers if expected_workers is not None else len(selected)==1,
                "wrong number or identity of admitted workers")
        require(row["requested_ticks"]==count and row["all_workers_exited"] is True
                and set(row["worker_metrics"])=={str(s) for s in selected},"incomplete stage lifecycle")
        if not selected:
            require(row["accepted_ticks"]==0 and row["end_tick"]==row["last_verified_lab_tick"]==start
                    and row["end_root"]==row["last_verified_lab_root"]==row["start_root"],
                    "denied stage advanced state")
            continue
        end=start+count
        roots=[world_root(t,region_roots(ref[t])) for t in range(start+1,end+1)]
        require(row["accepted_ticks"]==count and row["end_tick"]==row["last_verified_lab_tick"]==end
                and row["per_tick_roots"]==roots and row["end_root"]==row["last_verified_lab_root"]==roots[-1]
                and row["full_state_matches_serial"] is True,"accepted root or tick mismatch")
        timings=row["samples_ms"]
        require(len(timings)==count and all(math.isfinite(t) and t>0 for t in timings)
                and all(row[k]==v for k,v in duration_summary(timings).items())
                and row["checked_tick_loop_wall_ms"]>=sum(timings)
                and row["minimum_shared_headroom_bytes"]>=MACHINE_RESERVE,
                "timing or shared headroom evidence inconsistent")
        plan=row["decision"]["plan"]
        estimates=[evaluate(plan["owner_by_region"],region_load(ref[t]),samples) for t in range(start,end+1)]
        require(all(est is not None for est in estimates)
                and row["max_estimated_tick_us"]==max(est["score_us"] for est in estimates)<=DEADLINE_US,
                "load exceeded chosen allocation")
        for slot in selected:
            worker=row["worker_metrics"][str(slot)]
            o=opts[slot]
            for label in ("kernel_before","kernel_after"):
                verify_kernel(worker[label],o)
                require(worker[label]["effective_cpu_millicores"]>=samples[slot].effective_cpu_millicores,
                        "admitted CPU reduced")
            require(worker["regions"]==plan["allocations"][str(slot)]["regions"]
                    and worker["compute_cpu_us"]>0 and worker["process_cpu_us"]>=worker["compute_cpu_us"]
                    and worker["kernel_after"]["cpu_usage_usec"]>worker["kernel_before"]["cpu_usage_usec"]
                    and worker["frame_rx_bytes"]>0 and worker["frame_tx_bytes"]>0 and worker["peak_rss_kib"]>0,
                    "admitted worker did not execute observed work")
    stages=report["stages"]
    require(stages["scale_out"]["decision"]["reasons"]["2"]=="insufficient_memory"
            and stages["scale_out"]["decision"]["reasons"]["3"]=="path_too_slow"
            and stages["scale_out"]["cross_worker_agents"]>0
            and "no_useful_feasible_placement" in stages["clustered"]["decision"]["reasons"].values()
            and stages["cpu_blocked"]["decision"]["reasons"]["0"]=="no_complete_feasible_placement",
            "required admission scenarios not observed")
    a,b=stages["one_worker_comparison"],stages["scale_out"]
    c=report["comparison"]
    require(c["identical_start_and_end_roots"] is True and c["not_a_sustained_gameplay_benchmark"] is True
            and a["start_root"]==b["start_root"] and a["end_root"]==b["end_root"]
            and all(c[f"{prefix}_{p}_ms"]==row[f"{p}_ms"]
                    for prefix,row in (("one_worker",a),("two_worker",b)) for p in ("p50","p95"))
            and all(c[f"observed_{p}_gain_percent"]==100*(1-b[f"{p}_ms"]/a[f"{p}_ms"]) for p in ("p50","p95")),
            "invalid comparison")
    fault=report["policy_fault"]
    state=initial_world("spread",256,60)
    _,selected=verify_decision(fault,state,0,choices(Offer(0,1,96,1000)))
    require(selected=={0} and fault["requested_ticks"]==4
            and fault["last_verified_lab_tick"]==1
            and fault["last_verified_lab_root"]==world_root(1,region_roots(serial_step(state,1)))
            and "accepted_ticks" not in fault
            and all(fault.get(k) is True for k in ("injected_leaf_cpu_change","stage_export_refused",
                                                   "future_generation_refused","all_workers_exited")),
            "tampered resource policy did not freeze the lab")


def negative_checks(report):
    """Ensure a valid-looking but altered receipt cannot pass the verifier."""
    def bad(mutator):
        v=copy.deepcopy(report)
        mutator(v)
        try:
            verify(v)
        except (Rejected,KeyError,ValueError,TypeError,IndexError):
            return
        raise Rejected("tampered evidence accepted")
    bad(lambda r:r["stages"]["one_worker"]["per_tick_roots"].__setitem__(0,"0"*64))
    bad(lambda r:r["stages"]["scale_out"]["worker_metrics"]["1"]["kernel_after"].__setitem__("cpu_usage_usec",0))
    bad(lambda r:r["stages"]["scale_out"]["probes"]["3"]["sample"].__setitem__("rtt_us",1))
    bad(lambda r:r["stages"]["all_player_only"].__setitem__("spawned_slots",[0]))
    bad(lambda r:r["stages"]["scale_in"].__setitem__("all_workers_exited",False))
    bad(lambda r:r["policy_fault"].__setitem__("future_generation_refused",False))


if __name__=="__main__":
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("evidence",type=Path,nargs="+")
    parser.add_argument("--negative-checks",action="store_true")
    args=parser.parse_args()
    for path in args.evidence:
        report=json.loads(path.read_text())
        verify(report)
        if args.negative_checks:
            negative_checks(report)
        c=report["comparison"]
        print(f"PASS {path.name}: 8 stages, 104 accepted measured ticks, fault refusal; "
              f"p95 {c['one_worker_p95_ms']:.3f} -> {c['two_worker_p95_ms']:.3f} ms "
              f"({c['observed_p95_gain_percent']:.1f}% reduction)")
