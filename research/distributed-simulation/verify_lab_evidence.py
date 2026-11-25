#!/usr/bin/env python3
"""Recompute reference roots and summarize retained network-lab receipts.

This checks the recorded experiment's consistency. It does not attest to
the honesty of a worker, authenticate historical evidence, or certify a
Minecraft runtime. No sockets, interfaces or production state are opened.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import statistics

from network_lab import PROFILES, duration_summary
from region_runtime import initial_world, region_roots, require, serial_step, totals, world_root


def verify(report):
    require(report["classification"] == "REAL_NETWORKED_CUSTOM_REGION_MODEL_NOT_MINECRAFT_NOT_PHYSICAL_MULTI_HOST"
            and report["completed"] is True and report["hardware"]["physical_machine_count"] == 1,
            "incomplete or incorrectly classified experiment")
    params = report["parameters"]
    ticks, repeats = params["ticks"], params["repeats"]
    expected_modes = {("monolithic_serial", None)} | {
        (profile, count) for profile in params["profiles"] for count in params["worker_counts"]}
    for scenario in report["scenarios"].values():
        by_run = {}
        for row in scenario["runs"]:
            key = (row["repetition"], row["mode"], row.get("worker_count"))
            require(key not in by_run, "duplicate measurement")
            by_run[key] = row
        require(set(by_run) == {(rep, mode, count) for rep in range(1, repeats + 1)
                                for mode, count in expected_modes}, "missing measurement")
        for rep in range(1, repeats + 1):
            seed = by_run[(rep, "monolithic_serial", None)]["seed"]
            state = initial_world(scenario["shape"], scenario["agents"], seed)
            invariant = totals(region_roots(state))
            roots = []
            count_agents = count_resources = 0
            for tick in range(1, ticks + 1):
                counters = {}
                state = serial_step(state, tick, counters)
                rows = region_roots(state)
                require(totals(rows) == invariant, "reference conservation failure")
                roots.append(world_root(tick, rows))
                count_agents += counters["region_agents"]
                count_resources += counters["region_resources"]
            for mode, count in expected_modes:
                row = by_run[(rep, mode, count)]
                require(row["seed"] == seed and row["accepted_ticks"] == ticks
                        and row["per_tick_roots"] == roots and row["final_root"] == roots[-1]
                        and row["cross_region_agents"] == count_agents
                        and row["cross_region_resources"] == count_resources,
                        "recorded state or boundary count differs from reference")
                samples = row["samples_ms"]
                require(len(samples) == ticks and all(isinstance(x, (float, int)) and x > 0 for x in samples),
                        "invalid timing samples")
                require(all(row[k] == v for k, v in duration_summary(samples).items()), "invalid timing summary")
                if count is None:
                    continue
                require(row["all_tick_roots_match_serial"] and row["final_full_state_matches"]
                        and row["all_workers_exited"] and row["distinct_network_namespaces"] == count + 1
                        and row["agent_count"] == invariant[0] and row["total_resources"] == invariant[1]
                        and row["cross_worker_agents"] <= count_agents
                        and row["cross_worker_resources"] <= count_resources,
                        "invalid networked run receipt")
                require(len(row["worker_metrics"]) == count and len(row["application_frame_bytes"]) == count
                        and len(row["veth_interface_counters"]) == count and len(row["echo_rtt_ms"]) == count,
                        "missing per-worker metric")
                require(sorted(r for worker in row["worker_metrics"] for r in worker["region_ids"]) == list(range(8)),
                        "worker ownership overlap or omission")
                require(row["one_way_netem_delay_ms"] == PROFILES[mode], "wrong impairment profile")
                if count == 1:
                    require(row["cross_worker_agents"] == row["cross_worker_resources"] == 0,
                            "single worker has inter-worker messages")
                for frames, interface in zip(row["application_frame_bytes"], row["veth_interface_counters"]):
                    require(interface["rx_bytes"] >= frames["rx_bytes"] > 0
                            and interface["tx_bytes"] >= frames["tx_bytes"] > 0,
                            "missing actual link bytes")
                    for direction in ("coordinator", "worker"):
                        before = interface[direction + "_qdisc_before"]
                        after = interface[direction + "_qdisc_after"]
                        if PROFILES[mode]:
                            require(len(before) == len(after) == 1
                                    and after[0]["packets"] > before[0]["packets"]
                                    and after[0]["options"]["delay"]["delay"] == PROFILES[mode] / 1000,
                                    "no evidence of configured kernel impairment")
                        else:
                            require(not before and not after, "unexpected netem")
    require({f["fault"] for f in report["faults"]} == {"killed_after_prepare", "partition_after_seal"}
            and len(report["faults"]) == 2
            and all(f["accepted_cut_unchanged"] and f["subsequent_tick_and_snapshot_refused"]
                    and f["all_workers_exited"] and f["last_accepted_tick"] == 1
                    and f["cross_worker_agents_before_fault"] > 0 for f in report["faults"]),
            "fault evidence missing")
    initial = initial_world("boundary", 128)
    expected_fault_root = world_root(1, region_roots(serial_step(initial, 1)))
    require(all(f["last_accepted_root"] == expected_fault_root for f in report["faults"]), "fault prior root mismatch")


def summarize(report):
    print("| Workload | Mode | Workers | p50 ms | p95 ms | p99 ms | >50 ms | Echo RTT ms | Veth bytes/tick |")
    print("| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |")
    for label, scenario in report["scenarios"].items():
        groups = {}
        for row in scenario["runs"]:
            groups.setdefault((row["mode"], row.get("worker_count")), []).append(row)
        for (mode, count), rows in groups.items():
            median = lambda name: statistics.median(row[name] for row in rows)
            misses = sum(row["deadline_misses_50ms"] for row in rows)
            total = sum(row["accepted_ticks"] for row in rows)
            echo = ("-" if count is None else
                    f"{statistics.median(x for row in rows for worker in row['echo_rtt_ms'] for x in worker):.2f}")
            veth = ("-" if count is None else
                    f"{sum(link['rx_bytes'] + link['tx_bytes'] for row in rows for link in row['veth_interface_counters']) / total:.0f}")
            print(f"| {label} | {mode} | {count or 0} | {median('p50_ms'):.2f} | "
                  f"{median('p95_ms'):.2f} | {median('p99_ms'):.2f} | {misses}/{total} | {echo} | {veth} |")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("evidence", type=Path)
    args = parser.parse_args()
    report = json.loads(args.evidence.read_text())
    verify(report)
    summarize(report)
