#!/usr/bin/env python3
"""Offline replay of #63 paced durable scale evidence.

Historical process, clock and kernel measurements are reported observations,
not cryptographic attestations or evidence of independent physical hosts.
"""
import argparse
import hashlib
import json
import math
from pathlib import Path
import statistics

from region_runtime import canonical, digest, fields, integer, region_roots, require, serial_step, totals, world_root
from scale_lab import (CLASSIFICATION, CAP_MIB, CPU_MILLICORES, PERIOD_NS, WINDOW_NS,
                       REPEATS, SCENARIOS, PLAYERS, fixture)
from verify_player_evidence import reference_view
from verify_placement_evidence import verify_kernel
from worker_placement import MIB, Offer
from placement_lab import MACHINE_RESERVE
from scale_profile import PHASES

RULESET = "swarmcraft-reference-ring/transaction-v2"


def cut(world,tick,root):
    return hashlib.sha256(b"sc61-reference-cut-v2\0"+
                          canonical([RULESET,world,tick,root,PLAYERS])).hexdigest()


def expected_stats(ns):
    samples=[v/1e6 for v in ns]
    ordered=sorted(samples)
    def percentile(p):
        return ordered[math.ceil(len(ordered)*p/100)-1]
    return {"p50_ms":percentile(50),"p95_ms":percentile(95),"p99_ms":percentile(99),
            "mean_ms":statistics.mean(samples),"max_ms":max(samples),
            "deadline_misses_50ms":sum(v>50 for v in samples),
            "unpaced_service_ticks_per_second":len(samples)*1000/sum(samples)}


def verify_timing(row,profile=False,input_mode=None):
    ticks=row["ticks"]
    target=WINDOW_NS//PERIOD_NS
    require(1<=len(ticks)<=target,"invalid tick sample count")
    last=0
    service,latency=[],[]
    for t,r in enumerate(ticks,1):
        fields(r,"tick","scheduled_ns","begin_ns","finish_ns","root","cut_root",
               *("phase_ns",) if profile else (),
               *("transactions",) if input_mode is not None else ())
        scheduled=(t-1)*PERIOD_NS
        require(r["tick"]==t and r["scheduled_ns"]==scheduled
                and integer(r["begin_ns"],max(last,scheduled),WINDOW_NS-1)
                and integer(r["finish_ns"],r["begin_ns"]+1,WINDOW_NS+10_000_000_000),
                "invalid paced timing sequence")
        last=r["finish_ns"]
        if profile:
            phases=r["phase_ns"]
            fields(phases,*PHASES)
            elapsed=last-r["begin_ns"]
            require(all(integer(v,0,elapsed) for v in phases.values())
                    and sum(phases.values())==elapsed
                    and all(phases[k]>0 for k in ("state_decode_roots","worker_tick",
                        "worker_receipts","sqlite_durable_commit","independent_serial_replay",
                        "client_acknowledgment","resource_guard")), "invalid exclusive phase timing")
        if input_mode is not None:
            counts=r["transactions"]
            fields(counts,"write","read","commit","rollback")
            expected=(2,2,4,0) if input_mode=="batched" else (6,1,7,0)
            require(input_mode in ("batched","individual")
                    and all(type(v) is int for v in counts.values())
                    and tuple(counts[k] for k in ("write","read","commit","rollback"))==expected,
                    "unexpected transaction boundaries")
        service.append(last-r["begin_ns"])
        latency.append(last-scheduled)
    # Stopping early is permissible only if the fixed observation window
    # was exhausted (a final operation can straddle its boundary).
    require(len(ticks)==target or last>=WINDOW_NS,"premature benchmark termination")
    within=sum(r["finish_ns"]<=WINDOW_NS for r in ticks)
    on_time=sum(v<=PERIOD_NS for v in latency)
    expected={"accepted_ticks":len(ticks),"within_window":within,"target_ticks":target,
              "on_time_ticks":on_time,"schedule_misses_completed":len(ticks)-on_time,
              "unserved_slots":target-len(ticks),"target_slot_failures":target-on_time,
              "window_tps":within/(WINDOW_NS/1e9),
              "elapsed_tps":len(ticks)*1e9/max(WINDOW_NS,last),
              "service":expected_stats(service),"scheduled_latency":expected_stats(latency),
              "qualifies_20hz":within==target and on_time==target}
    require(row["summary"]==expected,"inconsistent timing summary")
    require(integer(row["coordinator_cpu_ns"],1,last),"invalid coordinator CPU time")


def verify_run(row,scenario,repeat,count,hardware,states,roots,profile=False,input_mode=None):
    label,shape,agents,delay=scenario
    require((row["scenario"],row["shape"],row["agents"],row["delay_ms"],row["repeat"],row["workers"])==
            (label,shape,agents,delay,repeat,count),"wrong scenario/order")
    require(row["groups"]==[[r for r in range(8) if r*count//8==s] for s in range(count)]
            and row["worker_cpus"]==hardware["worker_cpus"][:count]
            and len(set(row["worker_namespaces"]+[row["coordinator_namespace"]]))==count+1
            and len(row["worker_namespaces"])==count and row["all_workers_exited"] is True
            and row["scope_released"] is True,"invalid worker placement or cleanup")
    require(len(row["worker_pids"])==count and len(row["clients"])==len(PLAYERS)
            and [c["player"] for c in row["clients"]]==PLAYERS,"invalid process roster")
    pids=row["worker_pids"]+[c["pid"] for c in row["clients"]]
    require(len(set(pids))==len(pids) and all(integer(p,1,2**31-1) for p in pids),"invalid process identities")
    machine=row["machine"]
    require(machine["coordinator_cpu_reserve_millicores"]==1000
            and machine["machine_memory_reserve_bytes"]==MACHINE_RESERVE
            and machine["effective_cpu_millicores"]>=1000+count*CPU_MILLICORES
            and machine["available_memory_bytes"]>=MACHINE_RESERVE+count*CAP_MIB*MIB,
            "invalid shared resource receipt")
    for key in ("echo_rtt_ms","kernel_before","kernel_after","stats_before","stats_after",
                "net_before","net_after","worker_frame_bytes"):
        require(len(row[key])==count,"missing per-worker measurement")
    for s in range(count):
        offer=Offer(s,1,CAP_MIB,CPU_MILLICORES)
        a,b=row["kernel_before"][s],row["kernel_after"][s]
        verify_kernel(a,offer)
        verify_kernel(b,offer)
        require(b["cpu_usage_usec"]>a["cpu_usage_usec"],"no observed kernel work")
        ping=row["echo_rtt_ms"][s]
        require(len(ping)==6 and all(type(v) in (int,float) and math.isfinite(v) and v>0 for v in ping),
                "invalid echo observations")
        before,after=row["stats_before"][s],row["stats_after"][s]
        require(before["tick"]==0 and after["tick"]==len(row["ticks"])
                and 0<after["compute_cpu_ns"]-before["compute_cpu_ns"]<=
                after["process_cpu_ns"]-before["process_cpu_ns"]
                and after["peak_rss_kib"]>0,"worker did not compute ticks")
        before,after=row["net_before"][s],row["net_after"][s]
        tx,rx=row["worker_frame_bytes"][s]
        require(0<tx<=after["tx_bytes"]-before["tx_bytes"]
                and 0<rx<=after["rx_bytes"]-before["rx_bytes"],"invalid actual network traffic")
        for side in ("coordinator_netem","worker_netem"):
            a,b=before[side],after[side]
            if delay:
                require(len(a)==len(b)==1 and b[0]["packets"]>a[0]["packets"]
                        and a[0]["options"]["delay"]["delay"]==b[0]["options"]["delay"]["delay"]==delay/1000,
                        "kernel network impairment not exercised")
            else:
                require(not a and not b,"unexpected network impairment")
    verify_timing(row,profile,input_mode)
    n=len(row["ticks"])
    saved=row["final"]
    world=saved["world"]
    require(digest(world),"invalid world")
    cuts=[cut(world,t,roots[t]) for t in range(n+1)]
    for t,observed in enumerate(row["ticks"],1):
        require(observed["root"]==roots[t] and observed["cut_root"]==cuts[t],"wrong accepted world root")
    ids=list(range(agents))
    resources=totals(region_roots(states[0]))[1]
    require(saved["world"]==world and saved["ruleset"]==RULESET and saved["players"]==PLAYERS
            and saved["ids"]==ids and saved["resources"]==resources
            and saved["generation"]==saved["accepted_generation"]==1 and saved["claimed"] is True
            and saved["tick"]==n and saved["runtime_root"]==roots[n] and saved["cut_root"]==cuts[n]
            and canonical(saved["regions"])==canonical([states[n][r].wire() for r in range(8)]),
            "final save differs from serial reference")
    grants=saved["grants"]
    require(len(grants)==count and len({g["session"] for g in grants})==count,"incorrect grant set")
    for s,g in enumerate(grants):
        require(g=={"world":world,"generation":1,"session":g["session"],"regions":row["groups"][s],
                    "base_tick":0,"base_root":roots[0]} and digest(g["session"]),"wrong grant scope")

    def view(t,p):
        return {**reference_view(world,t,cuts[t],states[t],PLAYERS,p),"protocol":2,"ruleset":RULESET}

    def command(t,p):
        d=1 if t%2 else -1
        return {"op":"act","world":world,
                "id":hashlib.sha256(canonical(["sc63-command",world,p,t])).hexdigest(),
                "tick":t,"parent":cuts[t-1],"direction":d,"action":d}

    def receipt(t,p):
        return {"op":"committed","id":command(t,p)["id"],"view":view(t,p),
                "inventory_delta":view(t,p)["inventory"]-view(t-1,p)["inventory"]}

    for client in row["clients"]:
        p=client["player"]
        require(client["joined"]==view(0,p)
                and client["commands"]==[command(t,p) for t in range(1,n+1)]
                and client["receipts"]==[receipt(t,p) for t in range(1,n+1)],
                "client-observed state differs from reference")
    journal=row["journal"]
    require(canonical(journal["genesis"])==canonical([states[0][r].wire() for r in range(8)]),
            "incorrect journal genesis")
    batches=[]
    commands=[]
    for t in range(1,n+1):
        batch=[[p,command(t,p)["direction"],command(t,p)["action"],command(t,p)["id"]] for p in PLAYERS]
        batches.append({"tick":t,"parent":cuts[t-1],"batch":batch,"result":cuts[t]})
        commands.extend({"player":p,"request":command(t,p),"response":receipt(t,p)} for p in PLAYERS)
    require(journal["batches"]==batches and journal["commands"]==commands,"incorrect durable command journal")
    return world


def verify(data):
    require(isinstance(data,dict),"invalid benchmark receipt")
    profile=data.get("schema") in (2,3,4)
    batched_schema=data.get("schema") in (3,4)
    candidate_schema=data.get("schema")==4
    fields(data,"schema","classification","completed","period_ns","window_ns","repeats","hardware","runs",
           *("state_cache",) if profile else (),
           *("input_mode",) if batched_schema else (),
           *("candidate_mode","transaction_counts") if candidate_schema else ())
    require(type(data["schema"]) is int and data["schema"] in (1,2,3,4)
            and (not profile or data["state_cache"] in ("cached","uncached"))
            and (not batched_schema or data["input_mode"] in ("batched","individual"))
            and (not candidate_schema or (data["candidate_mode"] in ("reuse","reparse")
                 and data["transaction_counts"] in ("boundary","trace")))
            and data["classification"]==CLASSIFICATION and data["completed"] is True
            and data["period_ns"]==PERIOD_NS and data["window_ns"]==WINDOW_NS and data["repeats"]==REPEATS,
            "incomplete or misclassified benchmark")
    hw=data["hardware"]
    require(hw["physical_machines"]==1 and hw["cap_mib_per_worker"]==CAP_MIB
            and hw["cpu_millicores_per_worker"]==CPU_MILLICORES
            and len(set(hw["worker_cpus"]+[hw["coordinator_cpu"]]))==5,
            "invalid hardware classification")
    expected=[(s,r,c) for s in SCENARIOS for r in range(1,REPEATS+1)
              for c in ((1,2) if r%2 else (2,1))]
    require(len(data["runs"])==len(expected),"missing benchmark run")
    seen_worlds=set()
    for scenario in SCENARIOS:
        pairs=[(row,rep,count) for row,(s,rep,count) in zip(data["runs"],expected) if s==scenario]
        n=max(len(row["ticks"]) for row,_,_ in pairs)
        states=[fixture(scenario[1],scenario[2])]
        for t in range(1,n+1):
            d=1 if t%2 else -1
            states.append(serial_step(states[-1],t,controls={p:(d,d) for p in PLAYERS}))
        roots=[world_root(t,region_roots(state)) for t,state in enumerate(states)]
        for row,rep,count in pairs:
            world=verify_run(row,scenario,rep,count,hw,states,roots,profile,data.get("input_mode"))
            require(world not in seen_worlds,"comparison reused authority identity")
            seen_worlds.add(world)
    result=aggregate(data)
    if profile:
        result["state_cache"]=data["state_cache"]
        result["phase_profile"]=aggregate_profile(data)
    if batched_schema:
        result["input_mode"]=data["input_mode"]
        result["transactions"]={k:sum(t["transactions"][k] for r in data["runs"] for t in r["ticks"])
                                for k in ("write","read","commit","rollback")}
    if candidate_schema:
        result["candidate_mode"]=data["candidate_mode"]
        result["transaction_counts"]=data["transaction_counts"]
    return result


def aggregate_profile(data):
    """Tick-weighted exclusive wall times, not overlapping stage durations."""
    results=[]
    for scenario in SCENARIOS:
        for count in (1,2):
            ticks=[tick for row in data["runs"] if row["scenario"]==scenario[0]
                   and row["workers"]==count for tick in row["ticks"]]
            elapsed=sum(t["finish_ns"]-t["begin_ns"] for t in ticks)
            phases={k:sum(t["phase_ns"][k] for t in ticks) for k in PHASES}
            require(ticks and sum(phases.values())==elapsed,"phase aggregate does not reconcile")
            results.append({"scenario":scenario[0],"workers":count,"ticks":len(ticks),
                            "mean_service_ms":round(elapsed/len(ticks)/1e6,3),
                            "mean_phase_ms":{k:round(v/len(ticks)/1e6,3) for k,v in phases.items()},
                            "phase_share_percent":{k:round(100*v/elapsed,3) for k,v in phases.items()}})
    return results


def aggregate(data):
    results=[]
    for scenario in SCENARIOS:
        for count in (1,2):
            rows=[r["summary"] for r in data["runs"] if r["scenario"]==scenario[0] and r["workers"]==count]
            results.append({"scenario":scenario[0],"workers":count,
                            "within_window":[r["within_window"] for r in rows],
                            "p95_ms":[round(r["service"]["p95_ms"],2) for r in rows],
                            "on_time":[r["on_time_ticks"] for r in rows],
                            "window_tps":[r["window_tps"] for r in rows],
                            "qualifies_both":all(r["qualifies_20hz"] for r in rows)})
    return {"runs":len(data["runs"]),"accepted_ticks":sum(len(r["ticks"]) for r in data["runs"]),
            "within_window_ticks":sum(r["summary"]["within_window"] for r in data["runs"]),
            "client_commands":len(PLAYERS)*sum(len(r["ticks"]) for r in data["runs"]),
            "worker_processes":sum(r["workers"] for r in data["runs"]),
            "client_processes":len(PLAYERS)*len(data["runs"]),
            "qualified_runs":sum(r["summary"]["qualifies_20hz"] for r in data["runs"]),
            "results":results}


if __name__=="__main__":
    parser=argparse.ArgumentParser()
    parser.add_argument("receipt",type=Path)
    args=parser.parse_args()
    print(json.dumps(verify(json.loads(args.receipt.read_text())),indent=2))
