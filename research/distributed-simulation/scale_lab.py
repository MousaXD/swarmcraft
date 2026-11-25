#!/usr/bin/env python3
"""#63 fixed-placement, capped, paced durable reference benchmark.

Not Minecraft or independent physical machines. Compare two explicitly
consented lab layouts sequentially, not production admission or governance.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import secrets
import socket
import subprocess
import sys
import tempfile
import time

from network_lab import cpu_plan, duration_summary, safe_namespace
from placement_lab import CappedLab, Scope
from region_runtime import (CELLS, REGIONS, WIDTH, Channel, Region, canonical, fields,
                            initial_world, integer, require)
from transition_runtime import DurableRuntime, PROTOCOL, RULESET, TransitionAuthority
from worker_placement import Offer

CLASSIFICATION = "LOCAL_PACED_DURABLE_REFERENCE_SCALE_NOT_MINECRAFT"
PERIOD_NS = 50_000_000
WINDOW_NS = 8_000_000_000
REPEATS = 2
PLAYERS = [0, 1]
SCENARIOS = (
    ("spread_256_local", "spread", 256, 0),
    ("spread_2048_local", "spread", 2048, 0),
    ("spread_8192_local", "spread", 8192, 0),
    ("clustered_8192_local", "clustered", 8192, 0),
    ("spread_256_near", "spread", 256, 5),
    ("spread_256_wan", "spread", 256, 25),
)
CAP_MIB = 128
CPU_MILLICORES = 1000
SEED = 63


def fixture(shape, count):
    state = initial_world(shape, count, seed=SEED)
    positions = {0: CELLS//2-1, 1: CELLS-1}
    for rid, region in state.items():
        agents = [a for a in region.agents if a[0] not in positions]
        agents.extend((p,pos,5) for p,pos in positions.items() if pos//WIDTH == rid)
        state[rid] = Region(rid,region.signals,region.banks,tuple(sorted(agents)))
    return state


def action(tick):
    # Repeated crossings of the worker midpoint and ring seam.
    return 1 if tick % 2 else -1


def request(view, player):
    tick = view["tick"]+1
    d = action(tick)
    ident = hashlib.sha256(canonical(["sc63-command",view["world"],player,tick])).hexdigest()
    return {"op":"act","world":view["world"],"id":ident,"tick":tick,"parent":view["root"],
            "direction":d,"action":d}


def receipt_hash(receipt):
    return hashlib.sha256(canonical(receipt)).hexdigest()


def summarize(rows):
    service=[r["finish_ns"]-r["begin_ns"] for r in rows]
    latency=[r["finish_ns"]-r["scheduled_ns"] for r in rows]
    within=sum(r["finish_ns"] <= WINDOW_NS for r in rows)
    target=WINDOW_NS//PERIOD_NS
    misses=sum(v>PERIOD_NS for v in latency)
    stats=duration_summary([v/1e6 for v in service])
    schedule=duration_summary([v/1e6 for v in latency])
    return {"accepted_ticks":len(rows),"within_window":within,"target_ticks":target,
            "on_time_ticks":len(rows)-misses,
            "schedule_misses_completed":misses,
            "unserved_slots":target-len(rows),
            "target_slot_failures":target-(len(rows)-misses),
            "window_tps":within/(WINDOW_NS/1e9),
            "elapsed_tps":len(rows)*1e9/max(WINDOW_NS,rows[-1]["finish_ns"]),
            "service":stats,"scheduled_latency":schedule,
            "qualifies_20hz":within==target and misses==0}


class FixedCappedLab(CappedLab):
    def worker_command(self,index):
        cmd=super().worker_command(index)
        require(cmd[-1]=="--worker","wrong worker entry")
        cmd[-1]="--fenced-worker"
        return cmd


def client_main():
    require(os.getuid()==0 and Path("/proc/self/uid_map").read_text().split()[1]!="0",
            "mapped-root client required")
    line=sys.stdin.buffer.readline(4097)
    require(0<len(line)<=4096 and line.endswith(b"\n"),"invalid client config")
    config=json.loads(line)
    fields(config,"port","key","world","player")
    p=config["player"]
    require(p in PLAYERS,"unknown player")
    with socket.create_connection(("127.0.0.1",config["port"]),timeout=5) as sock:
        channel=Channel(sock,bytes.fromhex(config["key"]),coordinator=False)
        channel.send({"op":"join","world":config["world"],"protocol":PROTOCOL},time.monotonic()+5)
        view=channel.recv(time.monotonic()+5)
        require(view["player"]==p and view["world"]==config["world"] and view["tick"]==0
                and view["protocol"]==PROTOCOL and view["ruleset"]==RULESET,"bad initial view")
        observed={"player":p,"joined":view,"commands":[],"receipts":[]}
        while True:
            signal=channel.recv(time.monotonic()+10)
            if signal=={"op":"stop"}:
                break
            require(signal=={"op":"next","tick":view["tick"]+1},"unexpected tick trigger")
            req=request(view,p)
            channel.send(req,time.monotonic()+3)
            response=channel.recv(time.monotonic()+5)
            require(response["op"]=="committed" and response["id"]==req["id"]
                    and response["view"]["tick"]==req["tick"] and response["view"]["player"]==p
                    and response["view"]["world"]==config["world"],"invalid committed reply")
            observed["commands"].append(req)
            observed["receipts"].append(response)
            view=response["view"]
            channel.send({"op":"observed","tick":view["tick"],"hash":receipt_hash(response)},
                         time.monotonic()+3)
        print(json.dumps(observed,separators=(",",":")))


class Peers:
    def __init__(self,ledger,tmp):
        self.ledger=ledger
        self.tmp=Path(tmp)
        self.listener=socket.socket(socket.AF_INET,socket.SOCK_STREAM)
        self.listener.bind(("127.0.0.1",0))
        self.listener.listen(len(PLAYERS))
        self.listener.settimeout(5)
        self.entries=[]

    def __enter__(self):
        return self

    def spawn(self,p):
        key=secrets.token_bytes(32)
        output=(self.tmp/f"client-{p}.json").open("w+")
        try:
            process=subprocess.Popen([sys.executable,str(Path(__file__).resolve()),"--client"],
                                     stdin=subprocess.PIPE,stdout=output,stderr=subprocess.PIPE,
                                     env={"PATH":os.environ.get("PATH","/usr/bin:/bin"),
                                          "PYTHONDONTWRITEBYTECODE":"1"})
        except BaseException:
            output.close()
            raise
        row={"player":p,"process":process,"output":output,"channel":None}
        self.entries.append(row)
        view=self.ledger.view(p)
        process.stdin.write(canonical({"port":self.listener.getsockname()[1],"key":key.hex(),
                                       "world":view["world"],"player":p})+b"\n")
        process.stdin.close()
        sock,peer=self.listener.accept()
        channel=Channel(sock,key,coordinator=True)
        row["channel"]=channel
        require(peer[0]=="127.0.0.1","unexpected client peer")
        deadline=time.monotonic()+5
        require(channel.recv(deadline)=={"op":"join","world":view["world"],"protocol":PROTOCOL},
                "invalid join")
        channel.send(view,deadline)
        row["view"]=view

    def tick(self,runtime,tick,guard):
        deadline=time.monotonic()+3
        for row in self.entries:
            row["channel"].send({"op":"next","tick":tick},deadline)
        inputs=[]
        for row in self.entries:
            req=row["channel"].recv(deadline)
            require(req==request(row["view"],row["player"]),"fixture command differs")
            require(self.ledger.submit(row["player"],req)=={"op":"queued","id":req["id"],"tick":tick},
                    "input not queued")
            inputs.append(req)
        commit=runtime.advance(before_publish=guard)
        deadline=time.monotonic()+3
        receipts=[]
        for row,req in zip(self.entries,inputs):
            response=self.ledger.submit(row["player"],req)
            require(response["op"]=="committed" and response["view"]["tick"]==tick
                    and response["view"]["root"]==commit["cut_root"],"uncommitted reply")
            row["channel"].send(response,deadline)
            row["view"]=response["view"]
            receipts.append(response)
        for row,response in zip(self.entries,receipts):
            require(row["channel"].recv(deadline)=={"op":"observed","tick":tick,"hash":receipt_hash(response)},
                    "client acknowledgment mismatch")
        return commit

    def stop(self):
        rows=[]
        for row in self.entries:
            row["channel"].send({"op":"stop"},time.monotonic()+3)
        for row in self.entries:
            process=row["process"]
            require(process.wait(timeout=5)==0,"client failed: "+process.stderr.read(2048).decode(errors="replace"))
            row["output"].seek(0)
            rows.append({"pid":process.pid,**json.load(row["output"])})
        return rows

    def __exit__(self,*args):
        self.listener.close()
        for row in self.entries:
            if row["channel"] is not None:
                row["channel"].close()
            process=row["process"]
            if process.poll() is None:
                process.kill()
                process.wait(timeout=5)
            process.stderr.close()
            if not process.stdin.closed:
                process.stdin.close()
            row["output"].close()


def run_case(scope,cpus,scenario,repeat,count):
    label,shape,agents,delay=scenario
    offers=[Offer(s,1,CAP_MIB,CPU_MILLICORES) for s in range(count)]
    groups=[[r for r in range(REGIONS) if r*count//REGIONS==s] for s in range(count)]
    with tempfile.TemporaryDirectory(prefix="sc63-") as tmp:
        ledger=TransitionAuthority.create(Path(tmp)/"world.db",secrets.token_hex(32),
                                          fixture(shape,agents),PLAYERS,groups)
        try:
            with FixedCappedLab(scope,offers,{s:delay for s in range(count)},cpus) as lab:
                ping=lab.ping()
                runtime=DurableRuntime(ledger,lab.channels)
                control=runtime.runtime.coordinator
                before_stats=control.stats()
                before_net=lab.net_stats()
                before_kernel=[lab.observe(i) for i in range(count)]
                scope.recheck(before_kernel)
                before_frames=[(c.sent,c.received) for c in lab.channels]
                def guard():
                    scope.recheck([lab.observe(i) for i in range(count)])
                with Peers(ledger,tmp) as peers:
                    for p in PLAYERS:
                        peers.spawn(p)
                    rows=[]
                    start=time.monotonic_ns()
                    cpu_start=time.process_time_ns()
                    for tick in range(1,WINDOW_NS//PERIOD_NS+1):
                        target=(tick-1)*PERIOD_NS
                        now=time.monotonic_ns()-start
                        if now<target:
                            time.sleep((target-now)/1e9)
                        begin=time.monotonic_ns()-start
                        if begin>=WINDOW_NS:
                            break
                        guard()
                        accepted=peers.tick(runtime,tick,guard)
                        finish=time.monotonic_ns()-start
                        rows.append({"tick":tick,"scheduled_ns":target,"begin_ns":begin,"finish_ns":finish,
                                     "root":accepted["runtime_root"],"cut_root":accepted["cut_root"]})
                    cpu_ns=time.process_time_ns()-cpu_start
                    require(rows,"no accepted ticks")
                    clients=peers.stop()
                after_frames=[(c.sent,c.received) for c in lab.channels]
                after_kernel=[lab.observe(i) for i in range(count)]
                scope.recheck(after_kernel)
                after_stats=control.stats()
                after_net=lab.net_stats()
                saved=ledger.save()
                journal=ledger.audit()
                require(saved["tick"]==len(rows) and saved["runtime_root"]==rows[-1]["root"],"wrong final head")
                result={"scenario":label,"shape":shape,"agents":agents,"delay_ms":delay,
                        "repeat":repeat,"workers":count,"groups":groups,"worker_cpus":lab.cpus,
                        "coordinator_namespace":lab.parent_ns,
                        "worker_namespaces":lab.namespaces,"worker_pids":[p.pid for p in lab.processes],
                        "echo_rtt_ms":ping,"machine":lab.machine,
                        "kernel_before":before_kernel,"kernel_after":after_kernel,
                        "stats_before":before_stats,"stats_after":after_stats,
                        "net_before":before_net,"net_after":after_net,
                        "worker_frame_bytes":[[b[0]-a[0],b[1]-a[1]] for a,b in zip(before_frames,after_frames)],
                        "coordinator_cpu_ns":cpu_ns,"ticks":rows,"summary":summarize(rows),
                        "clients":clients,"final":saved,"journal":journal}
            require(not scope.active and not scope.poisoned and all(not leaf.exists() for leaf in lab.leaves)
                    and {n for _,n in socket.if_nameindex()}=={"lo"},"scope cleanup incomplete")
            result["all_workers_exited"]=all(p.returncode==0 for p in lab.processes)
            result["scope_released"]=True
            return result
        finally:
            ledger.close()


def run(report):
    safe_namespace()
    coordinator,cpus,affinity=cpu_plan()
    scope=Scope(affinity)
    os.sched_setaffinity(0,{coordinator})
    report["hardware"]={"physical_machines":1,"coordinator_cpu":coordinator,"worker_cpus":cpus,
                        "cap_mib_per_worker":CAP_MIB,"cpu_millicores_per_worker":CPU_MILLICORES}
    for scenario in SCENARIOS:
        for repeat in range(1,REPEATS+1):
            # Alternate order to reduce systematic warm/cache/host-load bias.
            for count in ((1,2) if repeat%2 else (2,1)):
                result=run_case(scope,cpus,scenario,repeat,count)
                report["runs"].append(result)
                s=result["summary"]
                print(f"{scenario[0]} rep={repeat} workers={count}: {s['within_window']}/{s['target_ticks']} "
                      f"p95={s['service']['p95_ms']:.2f}ms ontime={s['on_time_ticks']} "
                      f"tps={s['window_tps']:.2f}",flush=True)
    report["completed"]=True


def main():
    parser=argparse.ArgumentParser()
    parser.add_argument("--client",action="store_true")
    parser.add_argument("--output",type=Path)
    args=parser.parse_args()
    if args.client:
        client_main()
        return
    require(args.output is not None and not args.output.exists(),"new receipt path required")
    os.umask(0o077)
    report={"schema":1,"classification":CLASSIFICATION,"completed":False,
            "period_ns":PERIOD_NS,"window_ns":WINDOW_NS,"repeats":REPEATS,"runs":[]}
    with args.output.open("x") as out:
        try:
            run(report)
        finally:
            json.dump(report,out,separators=(",",":"),allow_nan=False)
            out.write("\n")


if __name__=="__main__":
    main()
