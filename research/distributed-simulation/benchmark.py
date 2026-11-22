#!/usr/bin/env python3
"""Reproducible synthetic CPU-process benchmark. NOT a Minecraft benchmark.

Measurements include real local CPU work and JSON encode/decode overhead. Network
RTT numbers are explicitly analytical lower bounds, NOT observed network behavior.
"""
import argparse
import hashlib
import json
import os
import platform
import statistics
import sys
import time
from concurrent.futures import ProcessPoolExecutor
from pathlib import Path


def distribution(shape, regions, count):
    if shape == "clustered":
        busy = count * 85 // 100
        result = [busy] + [0] * (regions - 1)
        rest = count - busy
        for i in range(rest):
            result[1 + i % (regions - 1)] += 1
        return result
    return [count // regions + (i < count % regions) for i in range(regions)]


def cpu_job(job):
    region, amount, rounds = job
    x = (region + 1) * 2654435761 & 0xFFFFFFFF
    # Python arithmetic is a stand-in for computation, NOT game AI/tick logic.
    for i in range(amount * rounds):
        x = (1664525 * (x ^ i) + 1013904223) & 0xFFFFFFFF
    return x


def run_jobs(jobs, executor=None):
    if executor is None:
        return [cpu_job(job) for job in jobs]
    return list(executor.map(cpu_job, jobs))


def measure(jobs, workers, repeats):
    samples = []
    if workers == 1:
        for _ in range(repeats):
            start = time.perf_counter_ns()
            values = run_jobs(jobs)
            samples.append((time.perf_counter_ns() - start) / 1e6)
    else:
        with ProcessPoolExecutor(max_workers=workers) as pool:
            values = run_jobs(jobs, pool)  # warm workers; excludes process start
            for _ in range(repeats):
                start = time.perf_counter_ns()
                new_values = run_jobs(jobs, pool)
                samples.append((time.perf_counter_ns() - start) / 1e6)
                if new_values != values:
                    raise RuntimeError("nondeterministic CPU proxy")
    return {"median_ms": round(statistics.median(samples), 3),
            "samples_ms": [round(v, 3) for v in samples],
            "checksum": hashlib.sha256(repr(values).encode()).hexdigest()[:16]}


def message_proxy(transfer_events, repeats):
    # Example payload is NOT a measured Minecraft protocol packet.
    record = {"entity": "entity-000000", "source": [0, 0], "destination": [1, 0],
              "epoch": 7, "tick": 1000, "state_digest": "ab" * 32}
    messages = [dict(record, entity=f"entity-{i:08d}") for i in range(transfer_events)]
    sizes = []
    milliseconds = []
    for _ in range(repeats):
        start = time.perf_counter_ns()
        encoded = [json.dumps(msg, sort_keys=True, separators=(",", ":")).encode() for msg in messages]
        decoded = [json.loads(msg) for msg in encoded]
        milliseconds.append((time.perf_counter_ns() - start) / 1e6)
        assert len(decoded) == transfer_events
        sizes.append(sum(map(len, encoded)))
    return {"events_per_tick_assumed": transfer_events,
            "bytes_per_tick_proxy": sizes[0],
            "median_local_encode_decode_ms": round(statistics.median(milliseconds), 3)}


def cpu_limits():
    quota = None
    try:
        p = Path('/sys/fs/cgroup/cpu.max')
        if p.exists():
            q, period = p.read_text().split()[:2]
            if q != 'max':
                quota = round(int(q) / int(period), 3)
    except (OSError, ValueError, ZeroDivisionError):
        pass
    return {"logical_cpus": os.cpu_count(),
            "affinity_cpus": len(os.sched_getaffinity(0)) if hasattr(os, 'sched_getaffinity') else None,
            "cgroup_cpu_quota": quota}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--entities', type=int, default=12000)
    parser.add_argument('--regions', type=int, default=12)
    parser.add_argument('--rounds', type=int, default=64)
    parser.add_argument('--repeats', type=int, default=5)
    parser.add_argument('--workers', default='1,2,4')
    parser.add_argument('--output', default='')
    args = parser.parse_args()
    workers = [int(i) for i in args.workers.split(',')]
    if args.regions < 2 or args.entities < 1 or args.rounds < 1 or args.repeats < 1 or any(i < 1 for i in workers):
        parser.error('regions>=2 and all other numeric parameters>=1')
    results = {"classification": "SYNTHETIC_LOCAL_PROCESS_MODEL_NOT_MINECRAFT_NOT_NETWORK",
               "python": platform.python_version(), "platform": platform.platform(),
               "hardware": cpu_limits(), "parameters": vars(args).copy(), "scenarios": {}}
    for shape in ['spread', 'clustered', 'boundary']:
        counts = distribution(shape, args.regions, args.entities)
        jobs = [(i, count, args.rounds) for i, count in enumerate(counts)]
        modes = {str(n): measure(jobs, n, args.repeats) for n in workers}
        checksums = {v['checksum'] for v in modes.values()}
        if len(checksums) != 1:
            raise RuntimeError('inconsistent region compute result across worker counts')
        # Assumed synthetic crossing rate, not observed Minecraft behavior.
        crossings = 0 if shape == 'spread' else (args.entities // 10 if shape == 'boundary' else args.entities // 100)
        results['scenarios'][shape] = {"region_entity_counts": counts, "workers": modes,
                                       "synthetic_transfer_proxy": message_proxy(crossings, args.repeats)}
    results['network_rtt_budget_sensitivity_NOT_MEASURED'] = [
        {"rtt_ms_assumption": ms, "single_barrier_lower_bound_ms": ms,
         "two_serial_barriers_lower_bound_ms": 2 * ms,
         "within_50ms_tick_if_one_barrier": ms < 50,
         "within_50ms_tick_if_two_barriers": 2 * ms < 50}
        for ms in (5, 25, 60)]
    results['limitations'] = ('Single-machine Python process benchmark; neither real game ticks nor '
                              'inter-machine network, consensus, durable writes or mod hooks.')
    output = json.dumps(results, indent=2, sort_keys=True) + '\n'
    if args.output:
        Path(args.output).write_text(output, encoding='utf-8')
    print(output)


if __name__ == '__main__':
    main()
