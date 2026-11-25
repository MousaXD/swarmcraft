#!/usr/bin/env python3
"""Replay every published reference player cut against the monolithic oracle."""
import argparse
import json
import math
from pathlib import Path

from player_runtime import PROTOCOL, RULESET, cut_digest, fixture, scripted_direction
from region_runtime import (CELLS, WIDTH, canonical, region_roots, require, serial_step,
                            world_root)
from network_lab import duration_summary


def reference_positions(state, players):
    rows = [(a[0], a[1], a[2]) for r in state.values() for a in r.agents if a[0] in players]
    require(len(rows) == len(players) and {r[0] for r in rows} == set(players),
            "reference roster mismatch")
    return {p: (pos, inventory) for p, pos, inventory in rows}


def reference_view(world, tick, root, state, players, player):
    pos, inventory = reference_positions(state, players)[player]
    signals = [v for rid in sorted(state) for v in state[rid].signals]
    banks = [v for rid in sorted(state) for v in state[rid].banks]
    cells = [(pos + offset) % CELLS for offset in range(-2, 3)]
    return {"op": "view", "protocol": PROTOCOL, "ruleset": RULESET,
            "world": world, "tick": tick, "root": root, "player": player,
            "position": pos, "region": pos // WIDTH, "inventory": inventory,
            "cells": [[c, signals[c], banks[c]] for c in cells]}


def verify(data):
    if "faults" in data:
        return verify_faults(data)
    require(data["schema"] == 1 and data["ruleset"] == RULESET, "wrong schema")
    world, players = data["world"], data["players"]
    require(players == [0, 1, 2] and data["active_ticks"] == 144
            and data["total_ticks"] == 148 and data["period_ms"] == 50,
            "wrong fixture")
    require(len(data["worker_pids"]) == 2 and len(data["client_pids"]) == 3
            and len(set(data["worker_pids"] + data["client_pids"])) == 5,
            "process identity overlap")
    require(len(data["worker_network_namespaces"]) == 2 and
            len(set(data["worker_network_namespaces"] +
                    [data["coordinator_network_namespace"]])) == 3, "namespace overlap")
    require(data["placement"] == [[r, r // 4] for r in range(8)], "wrong placement")
    state = fixture()
    root = cut_digest(world, 0, world_root(0, region_roots(state)), players)
    require(root == data["initial_root"], "wrong initial cut")
    views = {p: [reference_view(world, 0, root, state, players, p)] for p in players}
    require(data["initial_views"] == [views[p][0] for p in players], "wrong initial views")
    require(len(data["ticks"]) == 148, "missing ticks")
    crossings = {p: 0 for p in players}
    region_crossings = {p: 0 for p in players}
    for tick, row in enumerate(data["ticks"], 1):
        timings = [row[k] for k in ("elapsed_ms", "loop_ms", "schedule_ms")]
        require(all(type(t) in (int, float) and math.isfinite(t) and t > 0 for t in timings)
                and timings[0] <= timings[1] <= timings[2], "invalid timing observation")
        directions = {p: scripted_direction(p, tick) if p != 2 and tick <= 144 else 0
                      for p in players}
        require(row["tick"] == tick and row["directions"] == [[p, directions[p]] for p in players],
                "wrong input or tick")
        before = reference_positions(state, players)
        counter = {}
        state = serial_step(state, tick, counter, controls=directions)
        runtime_root = world_root(tick, region_roots(state))
        root = cut_digest(world, tick, runtime_root, players)
        require(row["runtime_root"] == runtime_root and row["root"] == root,
                "cut differs from independent serial reference")
        require(row["region_agents"] == counter["region_agents"] and
                row["region_resources"] == counter["region_resources"], "wrong crossing counters")
        after = reference_positions(state, players)
        for p in players:
            src, dst = before[p][0] // WIDTH, after[p][0] // WIDTH
            crossings[p] += src // 4 != dst // 4
            region_crossings[p] += src != dst
            views[p].append(reference_view(world, tick, root, state, players, p))
        require(row["views"] == [views[p][-1] for p in players], "wrong published view")
    require(canonical(data["final_regions"]) == canonical([state[r].wire() for r in sorted(state)]),
            "wrong final state")
    require(crossings == {0: 3, 1: 3, 2: 0} and region_crossings == {0: 4, 1: 4, 2: 0},
            "incomplete traversal")
    require(data["client_receipts"] == [
        {"player": p, "views": views[p][:145] if p != 2 else views[p][:1], "refused": p == 2}
        for p in players], "client-observed views differ")
    failures = sorted(data["client_failures"], key=lambda r: r["player"])
    require(failures == [
        {"player": 0, "tick": 145, "reason": "disconnect"},
        {"player": 1, "tick": 145, "reason": "disconnect"},
        {"player": 2, "tick": 1, "reason": "invalid_or_late_input"}], "wrong refusal evidence")
    require(data["loop_summary"] == duration_summary([r["loop_ms"] for r in data["ticks"]])
            and data["schedule_misses_50ms"] == sum(r["schedule_ms"] > 50 for r in data["ticks"]),
            "incorrect timing summary")
    return {"ticks": 148, "client_observed_views": 291, "worker_crossings": crossings,
            "region_crossings": region_crossings, "final_root": root}


def verify_faults(data):
    require(data["schema"] == 1 and data["ruleset"] == RULESET, "wrong fault schema")
    require([row["kind"] for row in data["faults"]] ==
            ["kill_after_prepare", "partition_after_seal"], "wrong fault cases")
    require(len({row["world"] for row in data["faults"]}) == 2, "reused fault world")
    players = [0, 1, 2]
    initial = fixture()
    next_state = serial_step(initial, 1, controls={0: 1, 1: 0, 2: 0})
    for row in data["faults"]:
        world = row["world"]
        r0 = cut_digest(world, 0, world_root(0, region_roots(initial)), players)
        r1 = cut_digest(world, 1, world_root(1, region_roots(next_state)), players)
        views = [reference_view(world, 0, r0, initial, players, 0),
                 reference_view(world, 1, r1, next_state, players, 0)]
        require(row["last_tick"] == 1 and row["last_root"] == r1
                and row["client_receipt"] == {"player": 0, "views": views, "refused": True},
                "fault advanced or altered published cut")
        require(row["failure"] in ("BrokenPipeError", "ConnectionResetError", "EOFError",
                                  "TimeoutError", "OSError", "Rejected")
                and row["refused"] == ["view", "advance", "input", "join"]
                and row["coordinator_poisoned"] is True and row["player_frozen"] is True
                and row["all_workers_exited"] is True
                and row["client_failures"] == [{"player": 0, "tick": 2, "reason": "world_frozen"}],
                "fault did not fail closed")
    return {"fault_cases": 2, "last_published_ticks": [1, 1], "client_views_after_fault": 0}


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("receipt", type=Path)
    args = parser.parse_args()
    print(json.dumps(verify(json.loads(args.receipt.read_text())), indent=2))
