#!/usr/bin/env python3
"""Explicit opt-in placement experiment, not a service or Minecraft runtime.

Run ONLY in the disposable delegated systemd scope plus fresh mapped
user/network namespace documented in WORKER_PLACEMENT_DESIGN.md.
"""
from __future__ import annotations

import argparse
from dataclasses import asdict
import json
import os
from pathlib import Path
import re
import socket
import time

from network_lab import Lab, cpu_plan, duration_summary, percentile, safe_namespace
from region_runtime import Coordinator, Rejected, initial_world, region_roots, require, serial_step, world_root
from worker_placement import (Offer, Sample, MIB, RESERVE_BYTES, MAX_SAMPLE_AGE_NS,
                              ConsentLedger, choose, evaluate, region_load)

MOUNT = Path("/sys/fs/cgroup")
MACHINE_RESERVE = 1024*MIB
PERIOD = 100_000
PAGE_SIZE = os.sysconf("SC_PAGE_SIZE")
CLASSIFICATION = "DELEGATED_KERNEL_PLACEMENT_CUSTOM_REFERENCE_NOT_MINECRAFT_NOT_MULTI_HOST"


def values(path):
    return {k: int(v) for k,v in (line.split() for line in path.read_text().splitlines())}


def group_of(pid):
    rows = Path(f"/proc/{pid}/cgroup").read_text().splitlines()
    require(len(rows) == 1 and rows[0].startswith("0::/"), "unverified unified cgroup")
    path = MOUNT / rows[0][4:]
    require(path.resolve() == path and path.is_dir(), "invalid cgroup membership")
    return path


def capacity(path, affinity):
    """Read all visible ancestor CPU and memory boundaries; not idle CPU."""
    cpu = len(affinity)*1000
    meminfo = dict(line.split(":", 1) for line in Path("/proc/meminfo").read_text().splitlines())
    memory = int(meminfo["MemAvailable"].split()[0])*1024
    require(memory > 0 and cpu > 0, "unmeasurable machine")
    while True:
        if (path/"cpu.max").exists():
            quota, period = (path/"cpu.max").read_text().split()
            period = int(period)
            require(period > 0, "invalid CPU period")
            if quota != "max":
                cpu = min(cpu, int(quota)*1000//period)
        for name in ("memory.max","memory.high"):
            if (path/name).exists():
                ceiling = (path/name).read_text().strip()
                if ceiling != "max":
                    memory = min(memory, max(0, int(ceiling)-int((path/"memory.current").read_text())))
        if path == MOUNT:
            break
        path = path.parent
        require(path == MOUNT or MOUNT in path.parents, "escaped cgroup hierarchy")
    return cpu, memory


class Scope:
    def __init__(self, affinity):
        raw = os.environ.get("SWARMCRAFT_HOST_CGROUP_ROOT", "")
        self.root = Path(raw)
        require(raw and self.root.resolve() == self.root and MOUNT in self.root.parents
                and re.fullmatch(r"swarmcraft-memory-cap-test-placement-58-[A-Za-z0-9_-]+\.scope", self.root.name)
                and group_of(os.getpid()) == self.root/"controller", "refusing non-disposable placement scope")
        require(not (self.root/"cgroup.procs").read_text().strip()
                and {"memory","cpu"} <= set((self.root/"cgroup.subtree_control").read_text().split()),
                "CPU and memory must be delegated to an empty parent")
        # A truncated cgroup mount can conceal tighter ancestors.
        mount = [line.split() for line in Path("/proc/self/mountinfo").read_text().splitlines()
                 if " - cgroup2 " in line and line.split()[4] == str(MOUNT)]
        require(len(mount) == 1 and mount[0][3] == "/", "opaque cgroup mount")
        require({p.name for p in self.root.iterdir() if p.is_dir()} == {"controller"},
                "disposable scope already contains worker leaves")
        self.affinity = affinity
        self.generation = 0
        self.active = False
        self.poisoned = False

    def begin(self, offers):
        require(not self.active and not self.poisoned, "overlapping or poisoned experiment generations")
        cpu, memory = capacity(self.root, self.affinity)
        require(sum(o.cpu_millicores for o in offers) <= max(0,cpu-1000)
                and sum(o.process_memory_mib*MIB for o in offers) <= max(0,memory-MACHINE_RESERVE),
                "insufficient shared physical/ancestor headroom")
        self.generation += 1
        self.active = True
        return {"effective_cpu_millicores":cpu,"available_memory_bytes":memory,
                "coordinator_cpu_reserve_millicores":1000,"machine_memory_reserve_bytes":MACHINE_RESERVE}

    def create(self, offer):
        require(self.active and offer.opted_in, "worker has no explicit consent")
        leaf = self.root/f"sc58-g{self.generation}-w{offer.slot}"
        leaf.mkdir()
        cap = offer.process_memory_mib*MIB
        try:
            for name, value in (("memory.max",cap),("memory.high",cap*9//10//PAGE_SIZE*PAGE_SIZE),
                                ("memory.swap.max",0),("memory.oom.group",1),
                                ("cpu.max",f"{offer.cpu_millicores*100} {PERIOD}")):
                (leaf/name).write_text(str(value)+"\n")
            if (leaf/"cpu.max.burst").exists():
                (leaf/"cpu.max.burst").write_text("0\n")
            return leaf
        except BaseException:
            leaf.rmdir()
            raise

    def recheck(self, observations):
        cpu, memory = capacity(self.root, self.affinity)
        remaining = sum(row["kernel_memory_max_bytes"]-row["kernel_memory_current_bytes"]
                        for row in observations)
        require(sum(row["effective_cpu_millicores"] for row in observations) <= max(0,cpu-1000)
                and memory >= MACHINE_RESERVE+remaining, "shared capacity no longer safe")
        return memory-remaining

    def end(self, leaves):
        for leaf in leaves:
            require(values(leaf/"cgroup.events")["populated"] == 0
                    and not (leaf/"cgroup.procs").read_text().strip(), "previous worker still active")
            leaf.rmdir()
        require({p.name for p in self.root.iterdir() if p.is_dir()} == {"controller"},
                "unexpected residual worker leaf")
        self.active = False


class CappedLab(Lab):
    """Uses the #56 isolated real TCP transport with pre-exec kernel caps."""
    def __init__(self, scope, offers, delays, cpus):
        self.scope = scope
        self.offers = sorted(offers, key=lambda o:o.slot)
        self.delays = delays
        self.leaves = []
        self.machine = scope.begin(self.offers)
        super().__init__(len(self.offers), 0, [cpus[o.slot] for o in self.offers])

    def __enter__(self):
        try:
            return super().__enter__()
        except BaseException:
            self.scope.poisoned = True
            raise

    def __exit__(self, kind, value, traceback):
        if kind is not None:
            self.scope.poisoned = True
        try:
            return super().__exit__(kind, value, traceback)
        except BaseException:
            self.scope.poisoned = True
            raise

    def worker_command(self, index):
        offer = self.offers[index]
        leaf = self.scope.create(offer)
        self.leaves.append(leaf)
        return ["sh", "-ec", 'printf "%s\\n" "$$" > "$1/cgroup.procs"; shift; exec "$@"',
                "sh", str(leaf), *super().worker_command(index)]

    def delay_for(self, index):
        return self.delays.get(self.offers[index].slot, 0)

    def observe(self, index):
        leaf, offer, process = self.leaves[index], self.offers[index], self.processes[index]
        require(process.poll() is None and group_of(process.pid) == leaf
                and set((leaf/"cgroup.procs").read_text().split()) == {str(process.pid)}
                and os.sched_getaffinity(process.pid) == {self.cpus[index]}, "worker ownership/affinity changed")
        cap = offer.process_memory_mib*MIB
        require((leaf/"cpu.max").read_text().split() == [str(offer.cpu_millicores*100),str(PERIOD)]
                and int((leaf/"memory.max").read_text()) == cap
                and int((leaf/"memory.high").read_text()) == cap*9//10//PAGE_SIZE*PAGE_SIZE
                and int((leaf/"memory.swap.max").read_text()) == 0
                and int((leaf/"memory.oom.group").read_text()) == 1
                and (not (leaf/"cpu.max.burst").exists() or int((leaf/"cpu.max.burst").read_text()) == 0),
                "kernel resource policy changed")
        events = values(leaf/"memory.events")
        high = int((leaf/"memory.high").read_text())
        current = int((leaf/"memory.current").read_text())
        peak = int((leaf/"memory.peak").read_text()) if (leaf/"memory.peak").exists() else None
        require(events["oom"] == events["oom_kill"] == 0 and current <= min(high,cap-RESERVE_BYTES),
                "worker exhausted memory headroom")
        cpu, _ = capacity(leaf, os.sched_getaffinity(process.pid))
        require(0 < cpu <= offer.cpu_millicores, "invalid effective CPU capacity")
        stat = values(leaf/"cpu.stat")
        return {"effective_cpu_millicores":cpu,"kernel_memory_max_bytes":cap,
                "kernel_memory_high_bytes":high,"kernel_memory_current_bytes":current,"kernel_memory_peak_bytes":peak,
                "cpu_usage_usec":stat["usage_usec"],"throttled_periods":stat["nr_throttled"],
                "memory_oom_events":events["oom"],"memory_oom_kills":events["oom_kill"],
                "java_heap_mib":None,"accounting":"linux_cgroup_v2_process_charges"}

    def probe(self):
        samples, receipts = {}, {}
        for index, offer in enumerate(self.offers):
            channel = self.channels[index]
            ping = []
            for _ in range(7):
                start = time.monotonic_ns()
                deadline = time.monotonic()+3
                channel.send({"op":"ping"},deadline)
                require(channel.recv(deadline) == {"op":"pong"}, "invalid authenticated echo")
                observed_ns = time.monotonic_ns()
                ping.append((observed_ns-start)/1e6)
            ping = ping[1:]  # discard first connection warm-up
            deadline = time.monotonic()+3
            channel.send({"op":"status"},deadline)
            require(channel.recv(deadline) == {"op":"status","tick":0,"phase":"new"},
                    "probe received simulation work before admission")
            observed = self.observe(index)
            rtt = int(percentile(ping,95)*1000+0.999)
            # Total usable worker charge budget, not remaining bytes or
            # Java heap. BASE_BYTES includes existing process overhead.
            usable = min(observed["kernel_memory_high_bytes"],
                         observed["kernel_memory_max_bytes"]-RESERVE_BYTES)
            sample = Sample(offer.slot,offer.revision,observed["effective_cpu_millicores"],
                            usable,rtt,observed_ns,True)
            samples[offer.slot] = sample
            receipts[str(offer.slot)] = {"sample":asdict(sample),"kernel":observed,
                    "echo_rtt_ms":ping,"one_way_netem_delay_ms":self.delay_for(index),
                    "pre_admission_phase":"new"}
        links = self.net_stats()
        for index, offer in enumerate(self.offers):
            row = receipts[str(offer.slot)]
            row.update(veth_rx_bytes=links[index]["rx_bytes"],veth_tx_bytes=links[index]["tx_bytes"])
            for direction in ("coordinator","worker"):
                netem = links[index][direction+"_netem"]
                require(len(netem) == bool(self.delay_for(index)), "unexpected qdisc")
                if netem:
                    require(netem[0]["packets"] > 0
                            and netem[0]["options"]["delay"]["delay"] == self.delay_for(index)/1000,
                            "kernel impairment not exercised")
                row[direction+"_netem_packets"] = netem[0]["packets"] if netem else 0
        return samples, receipts

    def retire(self, index):
        self.channels[index].close()
        process = self.processes[index]
        require(process.wait(timeout=5) == 0, "worker did not exit cleanly before placement")
        require(values(self.leaves[index]/"cgroup.events")["populated"] == 0,
                "retired worker has descendants")

    def close(self):
        if not self.scope.active:
            return
        try:
            super().close()
            require(all(p.returncode == 0 for p in self.processes),
                    "worker did not cleanly drain; no new generation permitted")
            self.scope.end(self.leaves)
        except BaseException:
            # Do not turn failed cleanup into reusable capacity.
            raise


def run_stage(scope, cpus, state, tick, root, offers, delays, ticks, receipt, *, fault=None):
    """One ephemeral generation; no restart from a failed/unverified cut."""
    require(not scope.active and not scope.poisoned, "previous generation not retired or poisoned")
    require(world_root(tick,region_roots(state)) == root, "unverified starting cut")
    for offer in offers:
        offer.validate()
    require(len({o.slot for o in offers}) == len(offers), "duplicate offers")
    positive = [o for o in offers if o.opted_in]
    load = region_load(state)
    receipt.update(start_tick=tick,start_root=root,offers=[asdict(o) for o in offers],
                   load=load,probes={},requested_ticks=ticks,
                   last_verified_lab_tick=tick,last_verified_lab_root=root)
    if not positive:
        decision_ns = time.monotonic_ns()
        receipt.update(decision_ns=decision_ns,decision=choose(offers,{},load,decision_ns),
                       accepted_ticks=0,end_tick=tick,end_root=root,
                       all_workers_exited=True,spawned_slots=[],worker_metrics={})
        return state,tick,root
    with CappedLab(scope, positive, delays, cpus) as lab:
        samples, probes = lab.probe()
        decision_ns = time.monotonic_ns()
        decision = choose(offers,samples,load,decision_ns)
        plan = decision["plan"]
        receipt.update(machine=lab.machine,probes=probes,decision_ns=decision_ns,decision=decision,
                       spawned_slots=[o.slot for o in positive])
        selected = sorted(set(plan["owner_by_region"])) if plan else []
        indices = [i for i,o in enumerate(lab.offers) if o.slot in selected]
        # Excluded candidates received only the probe; retire them before
        # any initialization message is sent to selected workers.
        for i,o in enumerate(lab.offers):
            if o.slot not in selected:
                lab.retire(i)
        receipt["probe_only_slots_exited_before_init"] = [o.slot for o in lab.offers if o.slot not in selected]
        if plan is None:
            receipt.update(accepted_ticks=0,end_tick=tick,end_root=root,worker_metrics={})
        else:
            placement = {r:selected.index(s) for r,s in enumerate(plan["owner_by_region"])}
            # A prior verified cut was drained and all former worker
            # processes exited. New authenticated sessions start exactly
            # at that cut, without resetting the global tick.
            # The lab's shadow reference supplies current measured region load.
            # These safety checks are outside the measured tick service duration.
            min_headroom = scope.recheck([lab.observe(i) for i in indices])
            max_score = plan["score_us"]
            control = Coordinator([lab.channels[i] for i in indices],state,placement=placement,start_tick=tick)
            require(control.root == root, "resumed root mismatch")
            before = control.stats()
            kernel_before = [lab.observe(i) for i in indices]
            before_frames = [(lab.channels[i].sent,lab.channels[i].received) for i in indices]
            results = []
            reference = state
            started_ns = time.monotonic_ns()
            def recheck(state):
                nonlocal min_headroom,max_score
                require(all(0 <= time.monotonic_ns()-samples[s].observed_ns <= MAX_SAMPLE_AGE_NS for s in selected),
                        "worker measurements expired; freeze generation")
                observations = [lab.observe(i) for i in indices]
                require(all(row["effective_cpu_millicores"] >= samples[s].effective_cpu_millicores
                            for s,row in zip(selected,observations)), "admitted CPU capacity reduced")
                min_headroom = min(min_headroom,scope.recheck(observations))
                estimate = evaluate(plan["owner_by_region"],region_load(state),samples)
                require(estimate is not None,"region load no longer fits admitted limits; freeze generation")
                max_score = max(max_score,estimate["score_us"])
            for _ in range(ticks):
                recheck(reference)
                result = control.step()
                reference = serial_step(reference,control.tick)
                require(result["root"] == world_root(control.tick,region_roots(reference)),
                        "accepted tick differs from independent serial rules")
                # Fail the experiment if the post-tick kernel state is
                # not verifiable; never export a new cut on such a fault.
                recheck(reference)
                results.append(result)
                receipt.update(last_verified_lab_tick=control.tick,last_verified_lab_root=control.root)
                if fault is not None:
                    fault(lab, indices, control.tick)
            state,_ = control.snapshot()
            require(state == reference, "full state differs from serial reference")
            after = control.stats()
            kernel_after = [lab.observe(i) for i in indices]
            metrics = {}
            for j, (slot,i) in enumerate(zip(selected,indices)):
                ch = lab.channels[i]
                metrics[str(slot)] = {"regions":sorted(control.owned[j]),
                    "compute_cpu_us":(after[j]["compute_cpu_ns"]-before[j]["compute_cpu_ns"])/1000,
                    "process_cpu_us":(after[j]["process_cpu_ns"]-before[j]["process_cpu_ns"])/1000,
                    "kernel_before":kernel_before[j],"kernel_after":kernel_after[j],
                    "peak_rss_kib":after[j]["peak_rss_kib"],
                    "frame_tx_bytes":ch.sent-before_frames[j][0],"frame_rx_bytes":ch.received-before_frames[j][1]}
                require(metrics[str(slot)]["compute_cpu_us"]>0, "admitted worker executed no simulation")
            tick,root = control.tick,control.root
            control.poisoned = True  # retire this in-memory coordinator before the next generation
            timing = [r["elapsed_ms"] for r in results]
            receipt.update(accepted_ticks=len(results),end_tick=tick,end_root=root,
                           per_tick_roots=[r["root"] for r in results],samples_ms=timing,
                           **duration_summary(timing),worker_metrics=metrics,full_state_matches_serial=True,
                           checked_tick_loop_wall_ms=(time.monotonic_ns()-started_ns)/1e6,
                           minimum_shared_headroom_bytes=min_headroom,
                           max_estimated_tick_us=max_score,
                           cross_worker_agents=sum(r["cross_worker_agents"] for r in results))
    require(not scope.active and {n for _,n in socket.if_nameindex()} == {"lo"},
            "generation cleanup not verified")
    receipt["all_workers_exited"] = True
    return state,tick,root


def run(report):
    safe_namespace()
    control_cpu,cpus,affinity = cpu_plan()
    scope = Scope(affinity)
    os.sched_setaffinity(0,{control_cpu})
    try:
        report["hardware"] = {"physical_machine_count":1,"worker_cpus":cpus,"coordinator_cpu":control_cpu,
                              "platform":"linux","actual_kernel_caps":True}
        stages = report["stages"]
        zero = lambda slot,revision=1: Offer(slot,revision,0,0)
        def offers(*active):
            result = {o.slot:o for o in active}
            return [result.get(slot,zero(slot)) for slot in range(4)]
        ledger = ConsentLedger()
        def stage(name,cut,choices,ticks,delays=None):
            require(not scope.active and not scope.poisoned,"previous generation not retired")
            ledger.accept(choices)
            row = {}
            stages[name] = row
            result = run_stage(scope,cpus,*cut,choices,delays or {},ticks,row)
            print(name+": "+str(row["decision"]["reasons"])+
                  f"; accepted={row['accepted_ticks']} p95={row.get('p95_ms','n/a')}",flush=True)
            return result
        state = initial_world("spread",16384,seed=58)
        cut = state,0,world_root(0,region_roots(state))
        first = stage("one_worker",cut,offers(Offer(0,1,96,1000)),12)
        comparator = stage("one_worker_comparison",first,offers(Offer(0,1,96,1000)),32)
        scaled = stage("scale_out",first,offers(Offer(0,1,96,1000),Offer(1,2,96,1000),
                       Offer(2,2,32,250),Offer(3,2,96,1000)),32,{3:25})
        require(comparator == scaled,"scale-out produced different accepted cut")
        require(set(stages["scale_out"]["worker_metrics"]) == {"0","1"}
                and stages["scale_out"]["decision"]["reasons"]["2"] == "insufficient_memory"
                and stages["scale_out"]["decision"]["reasons"]["3"] == "path_too_slow",
                "scale-out did not exercise intended placement")
        down = stage("scale_in",scaled,offers(Offer(0,2,80,1000),zero(1,3),zero(2,3),zero(3,3)),12)
        require(set(stages["scale_in"]["worker_metrics"]) == {"0"}, "withdrawal retained work")
        for name,choice in (("memory_blocked",offers(Offer(0,3,32,1000),zero(1,3),zero(2,3),zero(3,3))),
                            ("cpu_blocked",offers(Offer(0,4,96,250),zero(1,3),zero(2,3),zero(3,3))),
                            ("all_player_only",offers(zero(0,5),zero(1,3),zero(2,3),zero(3,3)))):
            require(stage(name,down,choice,8) == down and stages[name]["accepted_ticks"] == 0,
                    "inadequate capacity advanced the cut")
        cluster = initial_world("clustered",16384,seed=59)
        ledger = ConsentLedger()  # separate synthetic world and choices
        stage("clustered", (cluster,0,world_root(0,region_roots(cluster))),
              offers(Offer(0,1,96,1000),Offer(1,1,96,1000)),16)
        require(len(stages["clustered"]["worker_metrics"])==1,"unhelpful additional worker admitted")
        baseline,parallel = stages["one_worker_comparison"],stages["scale_out"]
        report["comparison"] = {"identical_start_and_end_roots":True,
            "one_worker_p95_ms":baseline["p95_ms"],"two_worker_p95_ms":parallel["p95_ms"],
            "one_worker_p50_ms":baseline["p50_ms"],"two_worker_p50_ms":parallel["p50_ms"],
            "observed_p95_gain_percent":100*(1-parallel["p95_ms"]/baseline["p95_ms"]),
            "observed_p50_gain_percent":100*(1-parallel["p50_ms"]/baseline["p50_ms"]),
            "not_a_sustained_gameplay_benchmark":True}
        # A separate deliberately tampered leaf (not the parent scope or
        # another process) must deny the next tick and all future stages.
        fault_state = initial_world("spread",256,seed=60)
        fault_cut = fault_state,0,world_root(0,region_roots(fault_state))
        fault = report["policy_fault"] = {}
        injected = False
        def tamper(lab,indices,tick):
            nonlocal injected
            if tick == 1:
                require(len(indices)==1,"wrong fault fixture")
                (lab.leaves[indices[0]]/"cpu.max").write_text("50000 100000\n")
                injected = True
        try:
            run_stage(scope,cpus,*fault_cut,offers(Offer(0,1,96,1000)),{},4,fault,fault=tamper)
        except Rejected as error:
            require(injected and str(error)=="kernel resource policy changed" and scope.poisoned
                    and not scope.active and "accepted_ticks" not in fault,
                    "fault not refused at the intended resource guard")
        else:
            raise Rejected("policy fault unexpectedly exported a stage")
        require(fault["last_verified_lab_tick"]==1 and fault["last_verified_lab_root"]==
                world_root(1,region_roots(serial_step(fault_state,1))),
                "fault changed last verified in-memory cut")
        try:
            run_stage(scope,cpus,*fault_cut,offers(Offer(0,2,96,1000)),{},1,{})
        except Rejected as error:
            require(str(error)=="previous generation not retired or poisoned","wrong refusal")
        else:
            raise Rejected("poisoned generation reused")
        fault.update(injected_leaf_cpu_change=True,stage_export_refused=True,
                     future_generation_refused=True,all_workers_exited=True)
        print("policy_fault: changed quota refused; verified cut retained, future generation denied",flush=True)
        report["completed"] = True
    finally:
        os.sched_setaffinity(0,affinity)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output",type=Path,required=True)
    args = parser.parse_args()
    os.umask(0o077)
    report = {"classification":CLASSIFICATION,"completed":False,"stages":{}}
    with args.output.open("x",encoding="utf-8") as output:
        try:
            run(report)
        finally:
            json.dump(report,output,indent=2,sort_keys=True,allow_nan=False)
            output.write("\n")
