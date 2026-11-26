#!/usr/bin/env python3
"""Real reference TCP clients, durable transitions, and pre/post-commit SIGKILL."""
import argparse
import json
import multiprocessing as mp
import os
from pathlib import Path
import secrets
import signal
import socket
import subprocess
import sys
import tempfile
import time

from authority_lab import FencedLab, GROUPS, SWAPPED
from network_lab import cpu_plan, safe_namespace
from player_runtime import fixture
from region_runtime import Channel, Rejected, canonical, fields, require
from regional_authority import candidate_receipts
from transition_runtime import DurableRuntime, PROTOCOL, RULESET, TransitionAuthority


def client_main():
    require(os.getuid() == 0 and Path("/proc/self/uid_map").read_text().split()[1] != "0",
            "client requires mapped-root namespace")
    line = sys.stdin.buffer.readline(4097)
    require(0 < len(line) <= 4096 and line.endswith(b"\n"), "invalid client config")
    config = json.loads(line)
    fields(config, "port", "key", "world", "player", "request")
    with socket.create_connection(("127.0.0.1", config["port"]), timeout=5) as sock:
        channel = Channel(sock, bytes.fromhex(config["key"]), coordinator=False)
        channel.send({"op": "join", "world": config["world"], "protocol": PROTOCOL}, time.monotonic() + 5)
        joined = channel.recv(time.monotonic() + 5)
        require(joined["world"] == config["world"] and joined["player"] == config["player"]
                and joined["ruleset"] == RULESET and joined["protocol"] == PROTOCOL, "wrong client view")
        channel.send(config["request"], time.monotonic() + 5)
        try:
            reply = channel.recv(time.monotonic() + 10)
        except EOFError:
            reply = None
        print(json.dumps({"player": config["player"], "joined": joined, "reply": reply}))


class Clients:
    def __init__(self, authority):
        self.authority = authority
        self.listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self.listener.bind(("127.0.0.1", 0))
        self.listener.listen(3)
        self.listener.settimeout(5)
        self.sessions = []

    def __enter__(self):
        return self

    def spawn(self, player, request):
        key = secrets.token_bytes(32)
        process = subprocess.Popen([sys.executable, str(Path(__file__).resolve()), "--client"],
                                   stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                   env={"PATH": os.environ.get("PATH", "/usr/bin:/bin"),
                                        "PYTHONDONTWRITEBYTECODE": "1"})
        item = {"process": process, "channel": None, "player": player, "request": request}
        self.sessions.append(item)
        current = self.authority.save()
        process.stdin.write(canonical({"port": self.listener.getsockname()[1], "key": key.hex(),
                                       "world": current["world"], "player": player,
                                       "request": request}) + b"\n")
        process.stdin.close()
        sock, peer = self.listener.accept()
        require(peer[0] == "127.0.0.1", "unexpected client peer")
        channel = Channel(sock, key, coordinator=True)
        item["channel"] = channel
        deadline = time.monotonic() + 5
        join = channel.recv(deadline)
        require(join == {"op": "join", "world": current["world"], "protocol": PROTOCOL},
                "invalid join")
        channel.send(self.authority.view(player), deadline)
        received = channel.recv(deadline)
        require(received == request, "client payload differs from fixture")
        try:
            item["submission"] = self.authority.submit(player, received)
        except Rejected as error:
            item["submission"] = {"op": "refused", "reason": str(error)}

    def finish(self, lose_ack=False):
        rows = []
        for item in self.sessions:
            channel = item["channel"]
            result = item["submission"]
            if not lose_ack:
                if result["op"] in ("queued", "committed"):
                    # Recheck even an already-committed replay at the
                    # actual send boundary. A guarded authority may have
                    # lost eligibility or its certified pin since spawn.
                    result, = self.authority.committed_receipts([(item["player"], item["request"])])
                    require(result["op"] == "committed", "attempt to send uncommitted response")
                channel.send(result, time.monotonic() + 5)
            channel.close()
            item["channel"] = None
            process = item["process"]
            require(process.wait(timeout=5) == 0, "client failed")
            observed = json.loads(process.stdout.read())
            require(observed["reply"] == (None if lose_ack else result), "client receipt mismatch")
            rows.append({"pid": process.pid, "request": item["request"],
                         "submission": item["submission"], "observed": observed})
        return rows

    def __exit__(self, *args):
        self.listener.close()
        for item in self.sessions:
            if item["channel"] is not None:
                item["channel"].close()
            process = item["process"]
            if process.poll() is None:
                process.kill()
                process.wait(timeout=5)
            process.stdout.close()
            process.stderr.close()
            if not process.stdin.closed:
                process.stdin.close()


def crash_writer(path, receipts, kind):
    ledger = TransitionAuthority(path)
    if kind == "before":
        ledger.commit(receipts, before_commit=lambda: os.kill(os.getpid(), signal.SIGKILL))
    elif kind == "after":
        ledger.commit(receipts)
        os.kill(os.getpid(), signal.SIGKILL)
    else:
        raise RuntimeError("bad fault fixture")


def fault_commit(path, runtime, kind):
    controls = runtime.seal()
    coordinator = runtime.runtime.coordinator
    candidate = coordinator.step(controls=controls)
    receipts = candidate_receipts(coordinator)
    require(all((r["body"]["tick"], r["body"]["runtime_root"]) ==
                (candidate["tick"], candidate["root"]) for r in receipts), "bad staged candidate")
    process = mp.get_context("spawn").Process(target=crash_writer, args=(path, receipts, kind))
    process.start()
    try:
        process.join(timeout=10)
        require(process.exitcode == -signal.SIGKILL, "writer crash not injected")
        return {"kind": kind, "pid": process.pid, "exitcode": process.exitcode,
                "candidate_tick": candidate["tick"], "candidate_root": candidate["root"]}
    finally:
        if process.is_alive():
            process.kill()
            process.join(timeout=3)
        process.close()


def commands(ledger, direction, action):
    cur = ledger.save()
    return {p: {"op": "act", "world": cur["world"], "id": secrets.token_hex(32),
                "tick": cur["tick"] + 1, "parent": cur["cut_root"],
                "direction": direction, "action": action} for p in (0, 1)}


def run(path, cpus):
    authority = TransitionAuthority.create(path, secrets.token_hex(32), fixture(), [0, 1, 2], GROUPS)
    checkpoints, phases, workers, faults = [], [], [], []

    def capture(label):
        checkpoints.append({"label": label, "save": authority.save(), "journal": authority.audit()})

    def record_workers(lab):
        workers.append({"generation": authority.save()["generation"], "pids": [p.pid for p in lab.processes],
                        "namespaces": lab.namespaces, "coordinator_namespace": lab.parent_ns})

    def normal(runtime, label, requests):
        with Clients(authority) as clients:
            for player, request in requests.items():
                clients.spawn(player, request)
            require(all(s["submission"]["op"] == "queued" for s in clients.sessions), "missing pending input")
            runtime.advance()
            phases.append({"label": label, "clients": clients.finish()})
        capture(label)

    def fault(runtime, label, requests, kind):
        with Clients(authority) as clients:
            for player, request in requests.items():
                clients.spawn(player, request)
            before = authority.save()
            faults.append(fault_commit(path, runtime, kind))
            after = authority.save()
            require(after["tick"] == before["tick"] + (kind == "after"), "wrong crash boundary")
            if kind == "before":
                require(before == after, "pre-commit crash changed canonical cut")
            phases.append({"label": label, "clients": clients.finish(lose_ack=True)})
        capture(label)

    def reopen(groups):
        nonlocal authority
        previous, journal = authority.save(), authority.audit()
        authority.close()
        authority = TransitionAuthority(path)
        require(authority.save() == previous and authority.audit() == journal, "restart changed world or journal")
        authority.rotate(previous["generation"], previous["tick"], previous["runtime_root"], groups)

    try:
        capture("genesis")
        with FencedLab(2, 0, cpus) as lab:
            runtime = DurableRuntime(authority, lab.channels)
            record_workers(lab)
            normal(runtime, "tick1", commands(authority, 1, 1))
            second = commands(authority, -1, -1)
            fault(runtime, "tick2_before_commit_crash", second, "before")
        require({name for _, name in socket.if_nameindex()} == {"lo"}, "first generation leaked interface")
        reopen(SWAPPED)
        with FencedLab(2, 0, cpus) as lab:
            runtime = DurableRuntime(authority, lab.channels)
            record_workers(lab)
            normal(runtime, "tick2_retry", second)
            third = commands(authority, 1, 1)
            fault(runtime, "tick3_after_commit_crash", third, "after")
        require({name for _, name in socket.if_nameindex()} == {"lo"}, "second generation leaked interface")
        reopen(GROUPS)
        with FencedLab(2, 0, cpus) as lab:
            runtime = DurableRuntime(authority, lab.channels)
            record_workers(lab)
            prior = authority.save()
            with Clients(authority) as clients:
                for player, request in third.items():
                    clients.spawn(player, request)
                require(all(s["submission"]["op"] == "committed" for s in clients.sessions),
                        "lost-ack replay did not recover original receipt")
                phases.append({"label": "tick3_replay", "clients": clients.finish()})
            require(authority.save() == prior, "replay advanced cut")
            capture("tick3_replay")
            with Clients(authority) as clients:
                clients.spawn(0, {**third[0], "action": -1})
                clients.spawn(1, third[0])
                require(all(s["submission"]["op"] == "refused" for s in clients.sessions),
                        "conflicting or spoofed replay accepted")
                phases.append({"label": "invalid_replays", "clients": clients.finish()})
            require(authority.save() == prior, "refusal changed cut")
            capture("invalid_replays")
            normal(runtime, "tick4", commands(authority, -1, -1))
        require({name for _, name in socket.if_nameindex()} == {"lo"}, "final generation leaked interface")
        return {"schema": 1, "classification": "LOCAL_DURABLE_REFERENCE_PLAYER_TRANSACTIONS_NOT_MINECRAFT",
                "checkpoints": checkpoints, "phases": phases, "workers": workers, "faults": faults,
                "all_workers_exited": True}
    finally:
        authority.close()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--client", action="store_true")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    if args.client:
        client_main()
        return
    require(args.output is not None and not args.output.exists(), "new receipt path required")
    safe_namespace()
    os.umask(0o077)
    control_cpu, cpus, _ = cpu_plan()
    os.sched_setaffinity(0, {control_cpu})
    with tempfile.TemporaryDirectory(prefix="sc61-") as tmp:
        result = run(Path(tmp) / "world.db", cpus)
    with args.output.open("x") as output:
        json.dump(result, output, separators=(",", ":"), allow_nan=False)
        output.write("\n")
    print(json.dumps({"ticks": result["checkpoints"][-1]["save"]["tick"],
                      "client_sessions": sum(len(p["clients"]) for p in result["phases"]),
                      "faults": result["faults"]}, indent=2))


if __name__ == "__main__":
    main()
