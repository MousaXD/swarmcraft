#!/usr/bin/env python3
"""Real private worker overlap, local authority process races and crash injection."""
import argparse
import json
import multiprocessing as mp
import os
from pathlib import Path
import secrets
import signal
import socket
import sys
import tempfile
import time

from network_lab import Lab, cpu_plan, safe_namespace
from player_runtime import fixture
from region_runtime import (Rejected, canonical, region_roots, require, serial_step,
                            world_root)
from regional_authority import Authority, FencedRuntime, candidate_receipts, wire

GROUPS = [list(range(4)), list(range(4, 8))]
SWAPPED = list(reversed(GROUPS))


class FencedLab(Lab):
    def worker_command(self, slot):
        return ["unshare", "--net", sys.executable,
                str(Path(__file__).with_name("network_lab.py").resolve()), "--fenced-worker"]


def contender(path, mode, payload, barrier, connection):
    ledger = Authority(path)
    try:
        if barrier is not None:
            barrier.wait(timeout=5)
        if mode == "rotate":
            generation, tick, root, groups = payload
            result = ledger.rotate(generation, tick, root, groups)
            connection.send({"accepted": True, "generation": result[0]["generation"]})
        elif mode == "commit":
            result = ledger.commit(payload)
            connection.send({"accepted": True, "result": result})
        elif mode == "kill_before":
            ledger.commit(payload, before_commit=lambda: os.kill(os.getpid(), signal.SIGKILL))
        elif mode == "kill_after":
            ledger.commit(payload)
            os.kill(os.getpid(), signal.SIGKILL)
        else:
            raise RuntimeError("unknown contention fixture")
    except Rejected as error:
        connection.send({"accepted": False, "reason": str(error)})
    finally:
        ledger.close()
        connection.close()


def execute_contenders(path, mode, payloads):
    ctx = mp.get_context("spawn")
    barrier = ctx.Barrier(len(payloads) + 1) if len(payloads) > 1 else None
    children = []
    try:
        for payload in payloads:
            read, write = ctx.Pipe(duplex=False)
            process = ctx.Process(target=contender, args=(path, mode, payload, barrier, write))
            process.start()
            write.close()
            children.append((process, read))
        if barrier is not None:
            barrier.wait(timeout=5)
        output = []
        for process, connection in children:
            process.join(timeout=10)
            require(not process.is_alive(), "authority contender hung")
            result = connection.recv() if process.exitcode == 0 and connection.poll() else None
            output.append({"pid": process.pid, "exitcode": process.exitcode, "result": result})
        return output
    finally:
        for process, connection in children:
            if process.is_alive():
                process.kill()
                process.join(timeout=3)
            connection.close()
            process.close()


def refusal(ledger, label, action):
    before, history = ledger.save(), ledger.history()
    try:
        action()
    except Rejected as error:
        result = {"label": label, "reason": str(error), "tick": before["tick"],
                  "generation": before["generation"], "root": before["runtime_root"]}
    else:
        raise Rejected("unexpected canonical acceptance: " + label)
    require(ledger.save() == before and ledger.history() == history, "refusal changed durable cut")
    return result


def run(path, cpus):
    state = fixture()
    ledger = Authority.create(path, secrets.token_hex(32), state, [0, 1, 2], GROUPS)
    saves = [{"label": "genesis", "save": ledger.save()}]
    refused = []
    races = {}
    checkpoints = []
    def capture(label):
        saves.append({"label": label, "save": ledger.save()})

    def check_tick():
        nonlocal state
        state = serial_step(state, ledger.save()["tick"])
        current = ledger.read()
        require(current["state"] == state and
                current["runtime_root"] == world_root(current["tick"], region_roots(state)),
                "durable cut differs from monolithic reference")

    try:
        with FencedLab(4, 0, cpus) as lab:
            old = FencedRuntime(ledger, lab.channels[:2])
            capture("initial_claim")
            refused.append(refusal(ledger, "restart_before_first_tick",
                                   lambda: FencedRuntime(ledger, [])))
            for _ in range(3):
                old.advance()
                check_tick()
                capture("initial_commit")
            prior = ledger.save()
            old.coordinator.step()
            stale = candidate_receipts(old.coordinator)
            require(all(p.poll() is None for p in lab.processes[:2]), "old workers exited before overlap")
            refused.append(refusal(ledger, "overlapping_grant", lambda: ledger.rotate(
                prior["generation"], prior["tick"], prior["runtime_root"], [[0, 1, 2, 3], [3, 4, 5, 6, 7]])))
            # Same requested epoch from two independent authority processes.
            payload = (prior["generation"], prior["tick"], prior["runtime_root"], SWAPPED)
            races["rotate"] = execute_contenders(path, "rotate", [payload, payload])
            require(all(row["exitcode"] == 0 and row["result"] is not None for row in races["rotate"])
                    and sum(row["result"]["accepted"] for row in races["rotate"]) == 1,
                    "rotation race produced multiple writers")
            capture("reassignment")
            require(ledger.save()["generation"] == 2 and ledger.save()["tick"] == 3,
                    "rotation altered accepted state")
            new = FencedRuntime(ledger, lab.channels[2:])
            capture("reassignment_claim")
            new.coordinator.step()
            fresh = candidate_receipts(new.coordinator)
            require(all(p.poll() is None for p in lab.processes), "overlap not established")
            checkpoints.append({"label": "live_overlap", "worker_pids": [p.pid for p in lab.processes],
                                "namespaces": list(lab.namespaces), "coordinator_namespace": lab.parent_ns,
                                "retired_candidate_tick": stale[0]["body"]["tick"],
                                "current_candidate_tick": fresh[0]["body"]["tick"],
                                "same_candidate_root": stale[0]["body"]["runtime_root"] ==
                                fresh[0]["body"]["runtime_root"]})
            refused.append(refusal(ledger, "retired_worker_receipts", lambda: ledger.commit(stale)))
            refused.append(refusal(ledger, "mixed_generation_receipts", lambda: ledger.commit([stale[0], fresh[1]])))
            # Concurrent exact current candidate publication. Exactly one wins.
            races["commit"] = execute_contenders(path, "commit", [fresh, fresh])
            require(all(row["exitcode"] == 0 and row["result"] is not None for row in races["commit"])
                    and sum(row["result"]["accepted"] for row in races["commit"]) == 1,
                    "commit race accepted more than one cut")
            check_tick()
            capture("raced_commit")
            refused.append(refusal(ledger, "replayed_current_receipts", lambda: ledger.commit(fresh)))
            refused.append(refusal(ledger, "retired_adapter", old.advance))
            new.advance()
            check_tick()
            capture("new_writer_commit")
            new.coordinator.step()
            before_kill = candidate_receipts(new.coordinator)
            prior = ledger.save()
            races["kill_before"] = execute_contenders(path, "kill_before", [before_kill])
            require(races["kill_before"][0]["exitcode"] == -signal.SIGKILL
                    and ledger.save() == prior, "interrupted commit changed accepted cut")
            capture("before_commit_crash")
            races["kill_after"] = execute_contenders(path, "kill_after", [before_kill])
            require(races["kill_after"][0]["exitcode"] == -signal.SIGKILL
                    and ledger.save()["tick"] == prior["tick"] + 1, "lost-ack commit missing")
            check_tick()
            capture("after_commit_crash")
            refused.append(refusal(ledger, "lost_ack_replay", lambda: ledger.commit(before_kill)))
        require({name for _, name in socket.if_nameindex()} == {"lo"}, "lab interface leak")
        ledger.close()
        ledger = Authority(path)
        capture("ledger_reopen")
        prior = ledger.save()
        refused.append(refusal(ledger, "restart_without_new_generation",
                               lambda: FencedRuntime(ledger, [])))
        ledger.rotate(prior["generation"], prior["tick"], prior["runtime_root"], SWAPPED)
        capture("restart_generation")
        with FencedLab(2, 0, cpus) as lab:
            restarted = FencedRuntime(ledger, lab.channels)
            capture("restart_claim")
            for _ in range(3):
                restarted.advance()
                check_tick()
                capture("restarted_commit")
        require({name for _, name in socket.if_nameindex()} == {"lo"}, "restart interface leak")
        return {"schema": 1, "classification": "LOCAL_DURABLE_REFERENCE_AUTHORITY_NOT_CONSENSUS",
                "saves": saves, "refusals": refused, "races": races,
                "checkpoints": checkpoints, "history": ledger.history(),
                "all_workers_exited": True}
    finally:
        ledger.close()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    require(not args.output.exists(), "refusing to overwrite evidence")
    safe_namespace()
    os.umask(0o077)
    control_cpu, cpus, _ = cpu_plan()
    os.sched_setaffinity(0, {control_cpu})
    with tempfile.TemporaryDirectory(prefix="sc60-ledger-") as tmp:
        result = run(Path(tmp) / "ledger.db", cpus)
    with args.output.open("x") as output:
        json.dump(result, output, separators=(",", ":"), allow_nan=False)
        output.write("\n")
    print(json.dumps({"ticks": result["saves"][-1]["save"]["tick"],
                      "generation": result["saves"][-1]["save"]["generation"],
                      "refusals": result["refusals"], "races": result["races"]}, indent=2))


if __name__ == "__main__":
    main()
