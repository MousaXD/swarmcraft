#!/usr/bin/env python3
"""Verify retained public #61 observations by independent whole-world replay."""
import argparse
import hashlib
import json
from pathlib import Path

from player_runtime import fixture
from region_runtime import (canonical, digest, fields, integer, region_roots,
                            require, serial_step, totals, world_root)
from verify_player_evidence import reference_view

RULESET = "swarmcraft-reference-ring/transaction-v2"
PLAYERS = [0, 1, 2]
PHASES = [
    ("tick1", 0, 1, "normal"),
    ("tick2_before_commit_crash", 1, 2, "lost"),
    ("tick2_retry", 1, 2, "normal"),
    ("tick3_after_commit_crash", 2, 3, "lost"),
    ("tick3_replay", 3, 3, "replay"),
    ("invalid_replays", 3, 3, "invalid"),
    ("tick4", 3, 4, "normal"),
]
CHECKPOINTS = [
    ("genesis", 0, 1, 0, False, 0),
    ("tick1", 1, 1, 1, True, 1),
    ("tick2_before_commit_crash", 1, 1, 1, True, 2),
    ("tick2_retry", 2, 2, 2, True, 2),
    ("tick3_after_commit_crash", 3, 2, 2, True, 3),
    ("tick3_replay", 3, 3, 2, True, 3),
    ("invalid_replays", 3, 3, 2, True, 3),
    ("tick4", 4, 3, 3, True, 4),
]


def cut(world, tick, root):
    return hashlib.sha256(b"sc61-reference-cut-v2\0" +
                          canonical([RULESET, world, tick, root, PLAYERS])).hexdigest()


def verify(data):
    fields(data, "schema", "classification", "checkpoints", "phases", "workers", "faults", "all_workers_exited")
    require(data["schema"] == 1 and data["classification"] ==
            "LOCAL_DURABLE_REFERENCE_PLAYER_TRANSACTIONS_NOT_MINECRAFT"
            and data["all_workers_exited"] is True, "wrong classification")
    require(len(data["phases"]) == len(PHASES) and len(data["checkpoints"]) == len(CHECKPOINTS),
            "missing phase or checkpoint")
    initial = fixture()
    states = [initial]
    for tick in range(1, 5):
        d = 1 if tick % 2 else -1
        states.append(serial_step(states[-1], tick, controls={0: (d, d), 1: (d, d), 2: (0, 0)}))
    roots = [world_root(t, region_roots(state)) for t, state in enumerate(states)]
    ids = sorted(a[0] for r in initial.values() for a in r.agents)
    resources = totals(region_roots(initial))[1]
    world = data["checkpoints"][0]["save"]["world"]
    require(digest(world), "invalid world")
    cuts = [cut(world, tick, root) for tick, root in enumerate(roots)]

    def view(tick, player):
        return {**reference_view(world, tick, cuts[tick], states[tick], PLAYERS, player),
                "protocol": 2, "ruleset": RULESET}

    commands = {}
    request_ids, pids = set(), set()
    def pid(p):
        require(integer(p, 1, 2**31 - 1) and p not in pids, "duplicate process identity")
        pids.add(p)

    def committed(t, p):
        return {"op": "committed", "id": commands[t, p]["id"], "view": view(t, p),
                "inventory_delta": view(t, p)["inventory"] - view(t - 1, p)["inventory"]}

    for phase, (label, joined_tick, tick, mode) in zip(data["phases"], PHASES):
        require(phase["label"] == label and len(phase["clients"]) == 2, "wrong client phase")
        for p, row in enumerate(phase["clients"]):
            pid(row["pid"])
            request, obs = row["request"], row["observed"]
            require(obs["player"] == p and obs["joined"] == view(joined_tick, p), "client join differs")
            if mode == "invalid":
                expected = ({**commands[3, 0], "action": -1} if p == 0 else commands[3, 0])
                require(request == expected and row["submission"] == obs["reply"] ==
                        {"op": "refused", "reason": "conflicting command replay"}, "invalid replay accepted")
                continue
            if (tick, p) not in commands:
                d = 1 if tick % 2 else -1
                require(digest(request["id"]) and request["id"] not in request_ids and
                        request == {"op": "act", "world": world, "id": request["id"], "tick": tick,
                                    "parent": cuts[tick - 1], "direction": d, "action": d},
                        "wrong client request")
                commands[tick, p] = request
                request_ids.add(request["id"])
            require(request == commands[tick, p], "retry changed exact request")
            expected = committed(tick, p)
            queued = {"op": "queued", "id": request["id"], "tick": tick}
            require(row["submission"] == (expected if mode == "replay" else queued)
                    and obs["reply"] == (None if mode == "lost" else expected),
                    "client observed premature or incorrect committed receipt")
    require(len(request_ids) == 8, "wrong unique command count")

    grants_by_generation, seen_sessions = {}, set()
    for row, (label, tick, gen, accepted, claimed, journal_tick) in zip(data["checkpoints"], CHECKPOINTS):
        save, journal = row["save"], row["journal"]
        require(row["label"] == label and save["world"] == world and save["ruleset"] == RULESET
                and save["players"] == PLAYERS and save["ids"] == ids and save["resources"] == resources
                and save["generation"] == gen and save["accepted_generation"] == accepted
                and save["claimed"] is claimed and save["tick"] == tick and save["runtime_root"] == roots[tick]
                and save["cut_root"] == cuts[tick]
                and canonical(save["regions"]) == canonical([states[tick][r].wire() for r in range(8)]),
                "inconsistent durable world save")
        base_tick = {1: 0, 2: 1, 3: 3}[gen]
        groups = [[4, 5, 6, 7], [0, 1, 2, 3]] if gen == 2 else [[0, 1, 2, 3], [4, 5, 6, 7]]
        grants = save["grants"]
        require(len(grants) == 2 and [g["regions"] for g in grants] == groups, "wrong region assignment")
        for grant in grants:
            require(grant == {"world": world, "generation": gen, "regions": grant["regions"],
                              "session": grant["session"], "base_tick": base_tick, "base_root": roots[base_tick]}
                    and digest(grant["session"]), "wrong public grant")
        if gen not in grants_by_generation:
            new = {g["session"] for g in grants}
            require(len(new) == 2 and not seen_sessions.intersection(new), "session reused")
            seen_sessions.update(new)
            grants_by_generation[gen] = grants
        require(grants_by_generation[gen] == grants, "grant changed within generation")
        require(canonical(journal["genesis"]) == canonical([initial[r].wire() for r in range(8)]),
                "bad journal genesis")
        expected_batches, expected_commands = [], []
        for t in range(1, journal_tick + 1):
            batch = [[p, commands[t, p]["direction"], commands[t, p]["action"], commands[t, p]["id"]]
                     for p in (0, 1)] + [[2, 0, 0, None]]
            expected_batches.append({"tick": t, "parent": cuts[t - 1], "batch": batch,
                                     "result": cuts[t] if t <= tick else None})
            expected_commands.extend({"player": p, "request": commands[t, p],
                                      "response": committed(t, p) if t <= tick else None} for p in (0, 1))
        require(journal["batches"] == expected_batches and journal["commands"] == expected_commands,
                "journal not atomically bound to accepted cut")

    require(len(data["workers"]) == 3 and len(data["faults"]) == 2, "missing live generations or faults")
    namespaces = set()
    for generation, worker in enumerate(data["workers"], 1):
        require(worker["generation"] == generation and len(worker["pids"]) == 2
                and len(worker["namespaces"]) == 2 and len(set(worker["namespaces"])) == 2
                and worker["coordinator_namespace"] not in worker["namespaces"], "bad worker scope")
        for p in worker["pids"]:
            pid(p)
        # Namespace inodes may be recycled after all previous workers exit.
        namespaces.update(worker["namespaces"])
    for t, (kind, fault) in enumerate(zip(("before", "after"), data["faults"]), 2):
        pid(fault["pid"])
        require(fault == {"kind": kind, "pid": fault["pid"], "exitcode": -9,
                          "candidate_tick": t, "candidate_root": roots[t]}, "bad fault boundary")
    crossings = sum(view(t, p)["region"] // 4 != view(t - 1, p)["region"] // 4
                    for t in range(1, 5) for p in (0, 1))
    require(crossings == 8 and all(view(t, p)["inventory"] == (6 if t % 2 else 5)
                                  for t in range(5) for p in (0, 1)), "wrong movement or inventory fixture")
    return {"accepted_ticks": 4, "unique_commands": 8, "client_sessions": 14,
            "worker_crossings": crossings, "faults": 2, "replay_refusals": 2,
            "recovered_receipts": 2, "final_runtime_root": roots[4]}


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("receipt", type=Path)
    args = parser.parse_args()
    print(json.dumps(verify(json.loads(args.receipt.read_text())), indent=2))
