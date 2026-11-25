#!/usr/bin/env python3
"""Real TCP region lab in independent Linux network namespaces.

Run inside a *new* mapped-root user/network namespace. No host interface,
production runtime, private world, external network, or hosted CI is used.
All worker processes are scoped to this invocation.
"""
from __future__ import annotations

import argparse
import json
import math
import os
from pathlib import Path
import platform
import resource
import secrets
import select
import socket
import statistics
import subprocess
import sys
import time

from region_runtime import (Channel, Coordinator, KEY_BYTES, MAX_AGENTS, MAX_TICK, REGIONS,
                            Rejected, Worker, canonical, initial_world, integer, region_roots,
                            require, serial_step, totals, world_root)

PORT = 28560
PROFILES = {"local": 0, "near": 5, "wan": 25}  # one-way kernel netem delay in ms


def command(*args, capture=False):
    return subprocess.run(args, check=True, text=True, timeout=10,
                          stdout=subprocess.PIPE if capture else subprocess.DEVNULL,
                          stderr=subprocess.PIPE).stdout


def safe_namespace():
    require(platform.system() == "Linux" and os.getuid() == 0, "mapped-root Linux namespace required")
    uid_map = Path("/proc/self/uid_map").read_text().split()
    require(len(uid_map) == 3 and uid_map[0] == "0" and uid_map[1] != "0" and uid_map[2] == "1",
            "refusing initial user namespace")
    require({name for _, name in socket.if_nameindex()} == {"lo"}, "fresh loopback-only namespace required")
    require(not json.loads(command("ip", "-j", "route", "show", "default", capture=True)),
            "external route present")
    command("ip", "link", "set", "lo", "up")


def cpu_plan():
    available = sorted(os.sched_getaffinity(0))
    by_core = {}
    for cpu in available:
        path = Path(f"/sys/devices/system/cpu/cpu{cpu}/topology")
        core = (int((path / "physical_package_id").read_text()), int((path / "core_id").read_text()))
        by_core.setdefault(core, cpu)
    cpus = list(by_core.values())
    require(len(cpus) >= 5, "five distinct physical cores required for the fixed comparison")
    return cpus[0], cpus[1:5], available


def worker_main():
    # This entrypoint is deliberately usable only in a private mapped
    # namespace built by this harness. The session secret travels on stdin,
    # never in process arguments or retained evidence.
    require(os.getuid() == 0 and Path("/proc/self/uid_map").read_text().split()[1] != "0",
            "worker requires mapped-root namespace")
    print("namespace-ready", flush=True)
    line = sys.stdin.buffer.readline(4097)
    require(0 < len(line) <= 4096 and line.endswith(b"\n"), "missing bounded worker config")
    config = json.loads(line)
    require(isinstance(config, dict) and set(config) == {"bind", "key", "cpu"}, "invalid config")
    bind = config["bind"]
    require(isinstance(bind, str) and bind in [f"10.56.{n}.2" for n in range(4)],
            "worker bind must be private lab endpoint")
    require({name for _, name in socket.if_nameindex()} == {"lo", "sc56w"},
            "worker must own only a lab veth")
    require(integer(config["cpu"], 0, 4095), "invalid CPU")
    os.sched_setaffinity(0, {config["cpu"]})
    key = bytes.fromhex(config["key"])
    require(len(key) == KEY_BYTES, "invalid session key")
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as listener:
        listener.settimeout(10)
        listener.bind((bind, PORT))
        listener.listen(1)
        print("listening", flush=True)
        sock, peer = listener.accept()
        require(peer[0] == bind[:-1] + "1", "unexpected lab coordinator")
    with sock:
        channel = Channel(sock, key, coordinator=False)
        worker = Worker()
        while True:
            try:
                request = channel.recv(time.monotonic() + 12)
            except EOFError:
                return
            response = worker.handle(request)
            channel.send(response, time.monotonic() + 5)


def ready(process, expected):
    readable, _, _ = select.select([process.stdout], [], [], 10)
    require(readable and process.stdout.readline() == expected + b"\n", "worker startup failed")


class Lab:
    def __init__(self, count, delay_ms, cpus):
        self.count, self.delay_ms, self.cpus = count, delay_ms, cpus
        self.processes, self.channels, self.interfaces = [], [], []
        self.namespaces = []
        self.expected_fault = False
        self.parent_ns = os.readlink("/proc/self/ns/net")

    def __enter__(self):
        try:
            for slot in range(self.count):
                self.spawn(slot)
            require(len(set(self.namespaces + [self.parent_ns])) == self.count + 1,
                    "worker namespace isolation not established")
            return self
        except BaseException:
            self.close()
            raise

    def worker_command(self, slot):
        return ["unshare", "--net", sys.executable, str(Path(__file__).resolve()), "--worker"]

    def delay_for(self, slot):
        return self.delay_ms

    def spawn(self, slot):
        process = subprocess.Popen(self.worker_command(slot),
                                   stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                   env={"PATH": os.environ.get("PATH", "/usr/bin:/bin"),
                                        "PYTHONUNBUFFERED": "1", "PYTHONDONTWRITEBYTECODE": "1"})
        self.processes.append(process)
        ready(process, b"namespace-ready")
        namespace = os.readlink(f"/proc/{process.pid}/ns/net")
        require(namespace != self.parent_ns and namespace not in self.namespaces, "network namespace reused")
        self.namespaces.append(namespace)
        interface = f"sc56c{slot}"
        self.interfaces.append(interface)
        command("ip", "link", "add", interface, "type", "veth", "peer", "name", "sc56w")
        command("ip", "link", "set", "sc56w", "netns", str(process.pid))
        command("ip", "addr", "add", f"10.56.{slot}.1/30", "dev", interface)
        command("ip", "link", "set", interface, "up")
        prefix = ("nsenter", "-t", str(process.pid), "-n")
        command(*prefix, "ip", "addr", "add", f"10.56.{slot}.2/30", "dev", "sc56w")
        command(*prefix, "ip", "link", "set", "sc56w", "up")
        command(*prefix, "ip", "link", "set", "lo", "up")
        delay_ms = self.delay_for(slot)
        if delay_ms:
            command("tc", "qdisc", "add", "dev", interface, "root", "netem",
                    "limit", "1000", "delay", f"{delay_ms}ms")
            command(*prefix, "tc", "qdisc", "add", "dev", "sc56w", "root", "netem",
                    "limit", "1000", "delay", f"{delay_ms}ms")
        key = secrets.token_bytes(KEY_BYTES)
        process.stdin.write(canonical({"bind": f"10.56.{slot}.2", "key": key.hex(),
                                       "cpu": self.cpus[slot]}) + b"\n")
        process.stdin.close()
        ready(process, b"listening")
        sock = socket.create_connection((f"10.56.{slot}.2", PORT), timeout=5)
        self.channels.append(Channel(sock, key, coordinator=True))

    def ping(self, samples=6):
        result = []
        for channel in self.channels:
            times = []
            for _ in range(samples + 1):
                start = time.monotonic_ns()
                deadline = time.monotonic() + 3
                channel.send({"op": "ping"}, deadline)
                require(channel.recv(deadline) == {"op": "pong"}, "invalid echo")
                times.append((time.monotonic_ns() - start) / 1e6)
            result.append(times[1:])  # exclude first ARP/TCP warmup
        return result

    def net_stats(self):
        output = []
        for slot, interface in enumerate(self.interfaces):
            row, = json.loads(command("ip", "-s", "-j", "link", "show", "dev", interface, capture=True))
            stats = row["stats64"]
            qdiscs = json.loads(command("tc", "-s", "-j", "qdisc", "show", "dev", interface, capture=True))
            peer_qdiscs = json.loads(command("nsenter", "-t", str(self.processes[slot].pid), "-n",
                                            "tc", "-s", "-j", "qdisc", "show", "dev", "sc56w", capture=True))
            output.append({
                "rx_bytes": stats["rx"]["bytes"], "tx_bytes": stats["tx"]["bytes"],
                "rx_packets": stats["rx"]["packets"], "tx_packets": stats["tx"]["packets"],
                "coordinator_netem": [d for d in qdiscs if d.get("kind") == "netem"],
                "worker_netem": [d for d in peer_qdiscs if d.get("kind") == "netem"],
            })
        return output

    def close(self):
        errors = []
        for channel in self.channels:
            channel.close()
        for process in self.processes:
            try:
                process.wait(timeout=2)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)
            stderr = process.stderr.read(2048).decode(errors="replace")
            if process.returncode not in (0, -9) and stderr:
                errors.append(stderr)
            process.stdout.close()
            process.stderr.close()
            if not process.stdin.closed:
                process.stdin.close()
        # Worker namespace exit removes its veth pair. Remove only our
        # explicitly created endpoints still present in the lab namespace.
        for interface in self.interfaces:
            if interface in {name for _, name in socket.if_nameindex()}:
                try:
                    command("ip", "link", "del", interface)
                except subprocess.CalledProcessError:
                    # The peer namespace can disappear asynchronously
                    # after waitpid. Only tolerate actual disappearance.
                    require(interface not in {name for _, name in socket.if_nameindex()},
                            "lab veth cleanup failed")
        self.errors = errors
        require(all(p.poll() is not None for p in self.processes), "worker cleanup incomplete")

    def __exit__(self, kind, value, traceback):
        self.close()
        if kind is None and not self.expected_fault:
            require(not self.errors, "unexpected worker exit: " + "; ".join(self.errors))


def percentile(samples, p):
    ordered = sorted(samples)
    return ordered[max(0, math.ceil(len(ordered) * p / 100) - 1)]


def duration_summary(samples):
    return {"p50_ms": percentile(samples, 50), "p95_ms": percentile(samples, 95),
            "p99_ms": percentile(samples, 99), "mean_ms": statistics.mean(samples),
            "max_ms": max(samples), "deadline_misses_50ms": sum(x > 50 for x in samples),
            "unpaced_service_ticks_per_second": len(samples) * 1000 / sum(samples)}


def serial_run(initial, ticks):
    state = initial
    rows = region_roots(state)
    invariant = totals(rows)
    roots = []
    samples, crosses = [], []
    started = time.process_time_ns()
    for tick in range(1, ticks + 1):
        counter = {}
        begin = time.monotonic_ns()
        state = serial_step(state, tick, counter)
        rows = region_roots(state)
        require(totals(rows) == invariant, "serial conservation failed")
        roots.append(world_root(tick, rows))
        samples.append((time.monotonic_ns() - begin) / 1e6)
        crosses.append(counter)
    return state, roots, crosses, {"samples_ms": samples, **duration_summary(samples),
                                   "cpu_ms": (time.process_time_ns() - started) / 1e6,
                                   "peak_rss_kib": resource.getrusage(resource.RUSAGE_SELF).ru_maxrss}


def network_run(initial, expected, roots, crosses, count, delay, cpus):
    with Lab(count, delay, cpus) as lab:
        ping = lab.ping()
        coordinator = Coordinator(lab.channels, initial)
        before_stats = coordinator.stats()
        before_net = lab.net_stats()
        before_frames = [(c.sent, c.received) for c in lab.channels]
        cpu_start = time.process_time_ns()
        ticks = [coordinator.step() for _ in roots]
        coordinator_cpu = (time.process_time_ns() - cpu_start) / 1e6
        after_frames = [(c.sent, c.received) for c in lab.channels]
        after_net = lab.net_stats()
        after_stats = coordinator.stats()
        # Check every accepted cut, not only the final counts or digest.
        require([t["root"] for t in ticks] == roots, "per-tick serial root mismatch")
        require([{k: t[k] for k in ("region_agents", "region_resources")} for t in ticks] == crosses,
                "boundary event count mismatch")
        state, _ = coordinator.snapshot()
        require(state == expected, "final full-state mismatch")
        wire = [{"tx_bytes": b[0] - a[0], "rx_bytes": b[1] - a[1]}
                for a, b in zip(before_frames, after_frames)]
        network = []
        for before, after in zip(before_net, after_net):
            item = {k: after[k] - before[k] for k in ("rx_bytes", "tx_bytes", "rx_packets", "tx_packets")}
            require(item["rx_bytes"] > 0 and item["tx_bytes"] > 0, "no real network traffic")
            if delay:
                for key in ("coordinator_netem", "worker_netem"):
                    require(len(before[key]) == len(after[key]) == 1 and
                            after[key][0]["packets"] > before[key][0]["packets"],
                            "kernel impairment not exercised")
            item["coordinator_qdisc_before"] = before["coordinator_netem"]
            item["coordinator_qdisc_after"] = after["coordinator_netem"]
            item["worker_qdisc_before"] = before["worker_netem"]
            item["worker_qdisc_after"] = after["worker_netem"]
            network.append(item)
        worker_metrics = [
            {"compute_cpu_ms": (b["compute_cpu_ns"] - a["compute_cpu_ns"]) / 1e6,
             "process_cpu_ms": (b["process_cpu_ns"] - a["process_cpu_ns"]) / 1e6,
             "peak_rss_kib": b["peak_rss_kib"], "region_ids": sorted(coordinator.owned[slot]),
             "cpu": cpus[slot]}
            for slot, (a, b) in enumerate(zip(before_stats, after_stats))]
        samples = [t["elapsed_ms"] for t in ticks]
        result = {
            "accepted_ticks": coordinator.tick, "all_tick_roots_match_serial": True,
            "final_full_state_matches": True, "final_root": coordinator.root,
            "distinct_network_namespaces": len(set(lab.namespaces + [lab.parent_ns])),
            "worker_count": count, "one_way_netem_delay_ms": delay, "echo_rtt_ms": ping,
            "samples_ms": samples, **duration_summary(samples),
            "cross_region_agents": sum(t["region_agents"] for t in ticks),
            "cross_region_resources": sum(t["region_resources"] for t in ticks),
            "cross_worker_agents": sum(t["cross_worker_agents"] for t in ticks),
            "cross_worker_resources": sum(t["cross_worker_resources"] for t in ticks),
            "agent_count": coordinator.invariants[0], "total_resources": coordinator.invariants[1],
            "coordinator_cpu_ms": coordinator_cpu,
            "coordinator_peak_rss_kib": resource.getrusage(resource.RUSAGE_SELF).ru_maxrss,
            "worker_metrics": worker_metrics, "application_frame_bytes": wire,
            "veth_interface_counters": network,
            "per_tick_roots": [t["root"] for t in ticks],
        }
    result["all_workers_exited"] = True
    return result


def fault_run(kind, cpus):
    initial = initial_world("boundary", 128)
    with Lab(2, 0, cpus) as lab:
        coordinator = Coordinator(lab.channels, initial, timeout=0.75)
        first = coordinator.step()
        reference = serial_step(initial, 1)
        require(first["root"] == world_root(1, region_roots(reference))
                and first["cross_worker_agents"] > 0, "fault fixture has no crossing")
        prior = (coordinator.tick, coordinator.root)
        invoked = False
        def fault(phase):
            nonlocal invoked
            if kind == "killed_after_prepare" and phase == "prepared":
                invoked = True
                lab.expected_fault = True
                lab.processes[1].kill()
                lab.processes[1].wait(timeout=3)
            elif kind == "partition_after_seal" and phase == "sealed":
                invoked = True
                lab.expected_fault = True
                command("ip", "link", "set", lab.interfaces[1], "down")
        start = time.monotonic()
        try:
            coordinator.step(fault)
        except (OSError, EOFError, Rejected, TimeoutError) as error:
            failure = type(error).__name__
        else:
            raise Rejected("fault unexpectedly accepted a cut")
        elapsed = (time.monotonic() - start) * 1000
        require(invoked and coordinator.poisoned and (coordinator.tick, coordinator.root) == prior,
                "failed tick changed accepted cut")
        try:
            coordinator.step()
        except Rejected:
            pass
        else:
            raise Rejected("poisoned run continued")
        try:
            coordinator.snapshot()
        except Rejected:
            pass
        else:
            raise Rejected("poisoned run exported state")
        # A partitioned worker might have committed tentatively. Never
        # treat any local phase as a new accepted cut or resume a lab.
        local = None
        try:
            ch = lab.channels[0]
            deadline = time.monotonic() + 1
            ch.send({"op": "status"}, deadline)
            local = ch.recv(deadline)
        except (OSError, EOFError, Rejected, TimeoutError):
            pass
        result = {"fault": kind, "failed_barrier": failure, "elapsed_until_refusal_ms": elapsed,
                  "last_accepted_tick": prior[0], "last_accepted_root": prior[1],
                  "accepted_cut_unchanged": True, "subsequent_tick_and_snapshot_refused": True,
                  "surviving_worker_observation_not_authority": local,
                  "cross_worker_agents_before_fault": first["cross_worker_agents"]}
        # Closing the connection after link-down may leave an honest peer
        # waiting for its deadline. Lab.close kills only the scoped worker.
    result["all_workers_exited"] = True
    return result


def run(args, report):
    safe_namespace()
    coordinator_cpu, cpus, original = cpu_plan()
    report["hardware"] = {"platform": platform.platform(), "python": platform.python_version(),
                          "available_logical_cpus": len(original), "coordinator_cpu": coordinator_cpu,
                          "worker_cpus_on_distinct_physical_cores": cpus,
                          "physical_machine_count": 1,
                          "cpu_affinity_not_hard_quota": True}
    os.sched_setaffinity(0, {coordinator_cpu})
    try:
        for label, shape, count in args.scenarios:
            scenario = {"shape": shape, "agents": count, "runs": []}
            report["scenarios"][label] = scenario
            for repetition in range(args.repeats):
                initial = initial_world(shape, count, seed=56 + repetition)
                expected, roots, crosses, baseline = serial_run(initial, args.ticks)
                scenario["runs"].append({"repetition": repetition + 1, "seed": 56 + repetition,
                                         "mode": "monolithic_serial", "accepted_ticks": args.ticks,
                                         "final_root": roots[-1], "per_tick_roots": roots,
                                         "cross_region_agents": sum(c["region_agents"] for c in crosses),
                                         "cross_region_resources": sum(c["region_resources"] for c in crosses),
                                         **baseline})
                # Repeat the same workload and exact rules on all layouts.
                # No paced server sleeps, network profile is actual netem.
                for profile in args.profiles:
                    for n in args.workers:
                        result = network_run(initial, expected, roots, crosses, n, PROFILES[profile], cpus)
                        scenario["runs"].append({"repetition": repetition + 1, "seed": 56 + repetition,
                                                 "mode": profile, **result})
                        print(f"PASS {label} rep={repetition + 1} {profile} workers={n} "
                              f"p95={result['p95_ms']:.2f}ms misses={result['deadline_misses_50ms']}/"
                              f"{args.ticks}", flush=True)
                        require({name for _, name in socket.if_nameindex()} == {"lo"},
                                "lab interface cleanup incomplete")
        for kind in ("killed_after_prepare", "partition_after_seal"):
            report["faults"].append(fault_run(kind, cpus))
            require({name for _, name in socket.if_nameindex()} == {"lo"}, "fault cleanup incomplete")
            print(f"PASS {kind}: prior cut preserved, run frozen", flush=True)
        report["completed"] = True
    finally:
        os.sched_setaffinity(0, original)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--worker", action="store_true", help=argparse.SUPPRESS)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--ticks", type=int, default=32)
    parser.add_argument("--repeats", type=int, default=2)
    parser.add_argument("--workers", default="1,2,4")
    parser.add_argument("--profiles", default="local,near,wan")
    parser.add_argument("--scenarios", default="spread:8192,spread:16384,clustered:8192,boundary:8192")
    args = parser.parse_args()
    if args.worker:
        worker_main()
        return
    require(args.output is not None and integer(args.ticks, 1, 512) and integer(args.repeats, 1, 5),
            "bounded ticks/repeats and a new output file required")
    args.workers = [int(n) for n in args.workers.split(",")]
    args.profiles = args.profiles.split(",")
    require(args.workers and len(set(args.workers)) == len(args.workers)
            and all(n in (1, 2, 4) for n in args.workers), "invalid worker counts")
    require(args.profiles and len(set(args.profiles)) == len(args.profiles)
            and all(p in PROFILES for p in args.profiles), "invalid profiles")
    args.scenarios = [(f"{shape}-{count}", shape, int(count))
                      for shape, count in (row.split(":") for row in args.scenarios.split(","))]
    require(1 <= len(args.scenarios) <= 8 and len({row[0] for row in args.scenarios}) == len(args.scenarios)
            and all(shape in ("spread", "clustered", "boundary") and 1 <= count <= MAX_AGENTS
                    for _, shape, count in args.scenarios), "invalid scenario")
    os.umask(0o077)
    report = {"classification": "REAL_NETWORKED_CUSTOM_REGION_MODEL_NOT_MINECRAFT_NOT_PHYSICAL_MULTI_HOST",
              "parameters": {"ticks": args.ticks, "repeats": args.repeats,
                             "worker_counts": args.workers, "profiles": args.profiles},
              "scenarios": {}, "faults": [], "completed": False}
    # Exclusive creation: never replace previous evidence, even after failure.
    with args.output.open("x", encoding="utf-8") as output:
        try:
            run(args, report)
        finally:
            json.dump(report, output, indent=2, sort_keys=True, allow_nan=False)
            output.write("\n")


if __name__ == "__main__":
    main()
