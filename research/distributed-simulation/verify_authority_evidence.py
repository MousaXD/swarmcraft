#!/usr/bin/env python3
"""Independent serial replay of public #60 saves and recorded race outcomes.

The report deliberately omits ephemeral MAC keys and raw credential-bearing
messages. This verifier checks structural and deterministic state evidence;
it cannot independently attest live process execution or authenticate MACs.
"""
import argparse
import json
from pathlib import Path

from player_runtime import RULESET, cut_digest, fixture
from region_runtime import canonical, digest, fields, integer, region_roots, require, serial_step, totals, world_root

LAYOUTS = {1: [[0, 1, 2, 3], [4, 5, 6, 7]],
           2: [[4, 5, 6, 7], [0, 1, 2, 3]],
           3: [[4, 5, 6, 7], [0, 1, 2, 3]]}
EXPECTED = [
    ("genesis", 1, 0, 0, False),
    ("initial_claim", 1, 0, 0, True),
    ("initial_commit", 1, 1, 1, True),
    ("initial_commit", 1, 1, 2, True),
    ("initial_commit", 1, 1, 3, True),
    ("reassignment", 2, 1, 3, False),
    ("reassignment_claim", 2, 1, 3, True),
    ("raced_commit", 2, 2, 4, True),
    ("new_writer_commit", 2, 2, 5, True),
    ("before_commit_crash", 2, 2, 5, True),
    ("after_commit_crash", 2, 2, 6, True),
    ("ledger_reopen", 2, 2, 6, True),
    ("restart_generation", 3, 2, 6, False),
    ("restart_claim", 3, 2, 6, True),
    ("restarted_commit", 3, 3, 7, True),
    ("restarted_commit", 3, 3, 8, True),
    ("restarted_commit", 3, 3, 9, True),
]
DENIALS = [
    ("restart_before_first_tick", 0, 1, "generation already claimed; rotate before worker restart"),
    ("overlapping_grant", 3, 1, "overlapping or incomplete region placement"),
    ("retired_worker_receipts", 3, 2, "stale, unknown or duplicate session"),
    ("mixed_generation_receipts", 3, 2, "stale, unknown or duplicate session"),
    ("replayed_current_receipts", 4, 2, "invalid or stale regional receipt"),
    ("retired_adapter", 4, 2, "runtime grant or cut retired"),
    ("lost_ack_replay", 6, 2, "invalid or stale regional receipt"),
    ("restart_without_new_generation", 6, 2, "generation already claimed; rotate before worker restart"),
]


def verify(data):
    fields(data, "schema", "classification", "saves", "refusals", "races", "checkpoints",
           "history", "all_workers_exited")
    require(data["schema"] == 1 and data["classification"] ==
            "LOCAL_DURABLE_REFERENCE_AUTHORITY_NOT_CONSENSUS"
            and data["all_workers_exited"] is True, "wrong report classification")
    players = [0, 1, 2]
    states = [fixture()]
    for tick in range(1, 10):
        states.append(serial_step(states[-1], tick))
    roots = [world_root(t, region_roots(state)) for t, state in enumerate(states)]
    ids = sorted(a[0] for region in states[0].values() for a in region.agents)
    resources = totals(region_roots(states[0]))[1]
    require(len(data["saves"]) == len(EXPECTED), "missing cut observations")
    world = data["saves"][0]["save"]["world"]
    require(digest(world), "wrong world identity")
    grant_sets, sessions = {}, set()
    for row, (label, generation, accepted, tick, claimed) in zip(data["saves"], EXPECTED):
        fields(row, "label", "save")
        save = row["save"]
        fields(save, "world", "ruleset", "players", "ids", "resources", "generation",
               "accepted_generation", "claimed", "tick", "runtime_root", "cut_root", "regions", "grants")
        require(row["label"] == label and save["world"] == world and save["ruleset"] == RULESET
                and save["players"] == players and save["ids"] == ids and save["resources"] == resources
                and integer(save["generation"], generation, generation)
                and integer(save["accepted_generation"], accepted, accepted)
                and integer(save["tick"], tick, tick) and save["claimed"] is claimed
                and save["runtime_root"] == roots[tick]
                and save["cut_root"] == cut_digest(world, tick, roots[tick], players)
                and canonical(save["regions"]) == canonical([states[tick][r].wire() for r in range(8)]),
                "world save differs from reference cut")
        base_tick = (0, 3, 6)[generation - 1]
        grants = save["grants"]
        require(len(grants) == 2 and [g["regions"] for g in grants] == LAYOUTS[generation],
                "overlapping or incorrect placement")
        for grant in grants:
            fields(grant, "world", "generation", "session", "regions", "base_tick", "base_root")
            require(grant["world"] == world and grant["generation"] == generation
                    and grant["base_tick"] == base_tick and grant["base_root"] == roots[base_tick]
                    and digest(grant["session"]), "bad grant scope")
        if generation not in grant_sets:
            new_ids = {g["session"] for g in grants}
            require(len(new_ids) == 2 and not sessions.intersection(new_ids), "session reused")
            sessions.update(new_ids)
            grant_sets[generation] = grants
        require(grants == grant_sets[generation], "grant changed without rotation")
    commits = data["history"]["commits"]
    epochs = data["history"]["epochs"]
    require(len(commits) == 10 and len(epochs) == 3, "duplicate or missing history")
    for tick, commit in enumerate(commits):
        expected_gen = (tick + 2) // 3 if tick else 0
        require(commit[:5] == [tick, expected_gen, roots[tick],
                               cut_digest(world, tick, roots[tick], players), roots[tick - 1] if tick else None]
                and (digest(commit[5]) if tick else commit[5] is None), "invalid commit history")
    require(epochs == [[g, t, roots[t], grant_sets[g], True] for g, t in ((1, 0), (2, 3), (3, 6))],
            "invalid generation history")
    require(data["refusals"] == [{"label": label, "tick": tick, "generation": g, "root": roots[tick],
                                  "reason": reason} for label, tick, g, reason in DENIALS],
            "invalid refusal observations")
    races = data["races"]
    fields(races, "rotate", "commit", "kill_before", "kill_after")
    pids = []
    for mode in ("rotate", "commit"):
        rows = races[mode]
        require(len(rows) == 2 and all(r["exitcode"] == 0 for r in rows), "invalid process race")
        pids.extend(r["pid"] for r in rows)
        winners = [r["result"] for r in rows if r["result"]["accepted"] is True]
        losers = [r["result"] for r in rows if r["result"]["accepted"] is False]
        expected = ({"accepted": True, "generation": 2} if mode == "rotate" else
                    {"accepted": True, "result": {"tick": 4, "generation": 2, "runtime_root": roots[4],
                                                  "cut_root": commits[4][3], "receipt_hash": commits[4][5]}})
        require(winners == [expected] and losers == [{"accepted": False,
                "reason": "stale generation or base cut" if mode == "rotate" else "invalid or stale regional receipt"}],
                "race did not produce exactly one accepted decision")
    for mode in ("kill_before", "kill_after"):
        row, = races[mode]
        require(row["exitcode"] == -9 and row["result"] is None, "missing process-kill evidence")
        pids.append(row["pid"])
    overlap, = data["checkpoints"]
    require(overlap["label"] == "live_overlap" and overlap["retired_candidate_tick"] == 4
            and overlap["current_candidate_tick"] == 4 and overlap["same_candidate_root"] is True
            and len(overlap["worker_pids"]) == 4 and len(overlap["namespaces"]) == 4
            and len(set(overlap["namespaces"] + [overlap["coordinator_namespace"]])) == 5,
            "missing live overlapping worker evidence")
    pids.extend(overlap["worker_pids"])
    require(all(integer(p, 1, 2**31 - 1) for p in pids) and len(set(pids)) == len(pids),
            "invalid process identities")
    return {"accepted_ticks": 9, "generations": 3, "save_observations": len(EXPECTED),
            "explicit_refusals": len(DENIALS), "single_winner_races": 2, "process_crashes": 2,
            "final_root": roots[-1]}


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("receipt", type=Path)
    args = parser.parse_args()
    print(json.dumps(verify(json.loads(args.receipt.read_text())), indent=2))
