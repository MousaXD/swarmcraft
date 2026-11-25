#!/usr/bin/env python3
"""Real private TCP reference clients and independently networked region workers."""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import secrets
import select
import socket
import subprocess
import sys
import tempfile
import time

from network_lab import Lab, command, cpu_plan, duration_summary, safe_namespace
from player_runtime import PlayerWorld, PROTOCOL, RULESET, fixture, scripted_direction
from region_runtime import (Channel, Coordinator, KEY_BYTES, Rejected, canonical,
                            integer, require)

ACTIVE_TICKS = 144
TOTAL_TICKS = 148
PERIOD = 0.05
INPUT_WINDOW = 0.04


def client_main():
    config = json.loads(sys.stdin.buffer.readline(4097))
    require(set(config) == {"port", "key", "world", "player", "ticks", "mode"},
            "bad private client config")
    require(os.getuid() == 0 and Path("/proc/self/uid_map").read_text().split()[1] != "0",
            "client requires mapped-root namespace")
    player, ticks = config["player"], config["ticks"]
    require(integer(player, 0, 2) and integer(ticks, 1, ACTIVE_TICKS), "bad client fixture")
    mode = config["mode"]
    require(mode in ("valid", "spoof", "fault"), "unknown client mode")
    with socket.create_connection(("127.0.0.1", config["port"]), timeout=5) as sock:
        channel = Channel(sock, bytes.fromhex(config["key"]), coordinator=False)
        channel.send({"op": "join", "world": config["world"], "protocol": PROTOCOL},
                     time.monotonic() + 5)
        first = channel.recv(time.monotonic() + 5)
        require(first["player"] == player and first["world"] == config["world"]
                and first["ruleset"] == RULESET and first["tick"] == 0, "wrong initial view")
        views = [first]
        for tick in range(1, ticks + 1):
            request = {"op": "move", "world": config["world"], "tick": tick,
                       "direction": scripted_direction(player, tick) if mode != "spoof" else 1}
            if mode == "spoof":
                request["player"] = 0
            channel.send(request, time.monotonic() + 5)
            if mode == "spoof" or (mode == "fault" and tick == ticks):
                try:
                    channel.recv(time.monotonic() + 5)
                except EOFError:
                    print(json.dumps({"player": player, "views": views, "refused": True}))
                    return
                raise Rejected("refused input received a view")
            view = channel.recv(time.monotonic() + 5)
            require(view["tick"] == tick and view["world"] == config["world"]
                    and view["player"] == player and view["ruleset"] == RULESET,
                    "client view lost continuity")
            views.append(view)
        print(json.dumps({"player": player, "views": views, "refused": False}))


class Clients:
    """One private ephemeral key per connection. No production player auth."""
    def __init__(self, world):
        self.world = world
        self.active = {}
        self.processes = {}
        self.failures = []
        self.tmp = tempfile.TemporaryDirectory(prefix="sc59-client-")
        self.listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self.listener.bind(("127.0.0.1", 0))
        self.listener.listen(3)
        self.listener.settimeout(5)

    def __enter__(self):
        return self

    def spawn(self, player, ticks, mode):
        require(player not in self.processes, "duplicate lab client")
        key = secrets.token_bytes(KEY_BYTES)
        output = open(Path(self.tmp.name) / f"{player}.json", "w+")
        try:
            process = subprocess.Popen([sys.executable, str(Path(__file__).resolve()), "--client"],
                                       stdin=subprocess.PIPE, stdout=output, stderr=subprocess.PIPE,
                                       env={"PATH": os.environ.get("PATH", "/usr/bin:/bin"),
                                            "PYTHONDONTWRITEBYTECODE": "1"})
        except BaseException:
            output.close()
            raise
        self.processes[player] = (process, output)
        process.stdin.write(canonical({"port": self.listener.getsockname()[1], "key": key.hex(),
                                       "world": self.world.world, "player": player, "ticks": ticks,
                                       "mode": mode}) + b"\n")
        process.stdin.close()
        sock, peer = self.listener.accept()
        channel = Channel(sock, key, coordinator=True)
        self.active[player] = channel
        require(peer[0] == "127.0.0.1", "unexpected client peer")
        deadline = time.monotonic() + 5
        view = self.world.join(player, channel.recv(deadline))
        channel.send(view, deadline)

    def drop(self, player, tick, reason):
        self.active.pop(player).close()
        self.failures.append({"player": player, "tick": tick, "reason": reason})

    def collect(self, deadline, tick):
        waiting = set(self.active)
        while waiting:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                break
            sockets = {self.active[p].sock: p for p in waiting}
            ready, _, _ = select.select(list(sockets), [], [], remaining)
            if not ready:
                break
            for sock in ready:
                player = sockets[sock]
                waiting.remove(player)
                try:
                    request = self.active[player].recv(deadline)
                    self.world.submit(player, request)
                except EOFError:
                    self.drop(player, tick, "disconnect")
                except (Rejected, ValueError, OSError):
                    self.drop(player, tick, "invalid_or_late_input")
        for player in sorted(waiting):
            self.drop(player, tick, "input_deadline")

    def publish(self, tick):
        deadline = time.monotonic() + 0.25
        for player in list(self.active):
            try:
                self.active[player].send(self.world.view(player), deadline)
            except (EOFError, OSError):
                self.drop(player, tick, "send_failure")

    def receipts(self):
        results = []
        for player, (process, output) in sorted(self.processes.items()):
            code = process.wait(timeout=5)
            err = process.stderr.read(2048).decode(errors="replace")
            require(code == 0, f"client {player} failed: {err}")
            output.seek(0)
            results.append(json.load(output))
        return results

    def __exit__(self, kind, value, traceback):
        for channel in self.active.values():
            channel.close()
        self.listener.close()
        for process, output in self.processes.values():
            try:
                process.wait(timeout=1)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)
            process.stderr.close()
            output.close()
        self.tmp.cleanup()


def run():
    safe_namespace()
    control_cpu, worker_cpus, _ = cpu_plan()
    os.sched_setaffinity(0, {control_cpu})
    with Lab(2, 0, worker_cpus) as lab:
        initial = fixture()
        coordinator = Coordinator(lab.channels, initial)
        world = PlayerWorld(coordinator, secrets.token_hex(KEY_BYTES), (0, 1, 2))
        initial_root = world.root
        initial_views = [world.view(p) for p in sorted(world.players)]
        with Clients(world) as clients:
            clients.spawn(0, ACTIVE_TICKS, "valid")
            clients.spawn(1, ACTIVE_TICKS, "valid")
            clients.spawn(2, 1, "spoof")
            rows = []
            start = time.monotonic()
            for tick in range(1, TOTAL_TICKS + 1):
                target = start + (tick - 1) * PERIOD
                time.sleep(max(0, target - time.monotonic()))
                begin = time.monotonic()
                clients.collect(begin + INPUT_WINDOW, tick)
                result = world.advance()
                clients.publish(tick)
                rows.append({**result, "views": [world.view(p) for p in sorted(world.players)],
                             "loop_ms": (time.monotonic() - begin) * 1000,
                             "schedule_ms": (time.monotonic() - target) * 1000})
            receipts = clients.receipts()
            require(clients.failures == [
                {"player": 2, "tick": 1, "reason": "invalid_or_late_input"},
                *[{"player": p, "tick": ACTIVE_TICKS + 1, "reason": "disconnect"} for p in
                  [row["player"] for row in clients.failures[1:]]]
            ] and sorted(row["player"] for row in clients.failures[1:]) == [0, 1],
                    f"unexpected client loss: {clients.failures}")
            require(not clients.active, "client sessions still active")
            evidence = {"schema": 1, "ruleset": RULESET, "world": world.world,
                        "players": sorted(world.players), "active_ticks": ACTIVE_TICKS,
                        "total_ticks": TOTAL_TICKS, "period_ms": PERIOD * 1000,
                        "initial_root": initial_root, "initial_views": initial_views,
                        "placement": [[r, coordinator.owner[r]] for r in sorted(coordinator.owner)],
                        "worker_pids": [p.pid for p in lab.processes],
                        "worker_network_namespaces": lab.namespaces,
                        "coordinator_network_namespace": lab.parent_ns,
                        "client_pids": [p.pid for p, _ in clients.processes.values()],
                        "client_failures": clients.failures, "client_receipts": receipts,
                        "ticks": rows,
                        "loop_summary": duration_summary([r["loop_ms"] for r in rows]),
                        "schedule_misses_50ms": sum(r["schedule_ms"] > 50 for r in rows),
                        "final_regions": [world.state[r].wire() for r in sorted(world.state)]}
    return evidence


def fault_run(kind, cpus):
    with Lab(2, 0, cpus) as lab:
        coordinator = Coordinator(lab.channels, fixture(), timeout=0.75)
        world = PlayerWorld(coordinator, secrets.token_hex(KEY_BYTES), (0, 1, 2))
        with Clients(world) as clients:
            clients.spawn(0, 2, "fault")
            clients.collect(time.monotonic() + 1, 1)
            require(world.pending == {0: 1}, "missing initial client input")
            initial = world.view(0)
            world.advance()
            clients.publish(1)
            accepted = world.view(0)
            require(accepted["region"] != initial["region"], "missing initial crossing")
            prior = world.tick, world.root
            clients.collect(time.monotonic() + 1, 2)
            require(world.pending == {0: 1}, "missing fault tick input")
            triggered = False

            def inject(phase):
                nonlocal triggered
                if kind == "kill_after_prepare" and phase == "prepared":
                    triggered = lab.expected_fault = True
                    lab.processes[1].kill()
                    lab.processes[1].wait(timeout=3)
                elif kind == "partition_after_seal" and phase == "sealed":
                    triggered = lab.expected_fault = True
                    command("ip", "link", "set", lab.interfaces[1], "down")

            try:
                world.advance(inject)
            except (OSError, EOFError, Rejected, TimeoutError) as error:
                failure = type(error).__name__
            else:
                raise Rejected("fault advanced player world")
            require(triggered and world.frozen and coordinator.poisoned and
                    (world.tick, world.root) == prior, "fault changed published cut")
            refused = []
            for label, operation in (
                ("view", lambda: world.view(0)), ("advance", world.advance),
                ("input", lambda: world.submit(0, {"op": "move", "world": world.world,
                                                   "tick": 2, "direction": 1})),
                ("join", lambda: world.join(0, {"op": "join", "world": world.world,
                                               "protocol": PROTOCOL})),
            ):
                try:
                    operation()
                except Rejected:
                    refused.append(label)
                else:
                    raise Rejected("frozen world allowed " + label)
            clients.drop(0, 2, "world_frozen")
            receipt, = clients.receipts()
            require(receipt == {"player": 0, "views": [initial, accepted], "refused": True},
                    "client observed unaccepted tick")
            result = {"kind": kind, "world": world.world, "failure": failure,
                      "last_tick": prior[0], "last_root": prior[1], "refused": refused,
                      "client_receipt": receipt, "client_failures": clients.failures,
                      "coordinator_poisoned": coordinator.poisoned, "player_frozen": world.frozen}
    require({name for _, name in socket.if_nameindex()} == {"lo"}, "fault interface leak")
    return {**result, "all_workers_exited": True}


def run_faults():
    safe_namespace()
    control_cpu, worker_cpus, _ = cpu_plan()
    os.sched_setaffinity(0, {control_cpu})
    return {"schema": 1, "ruleset": RULESET, "faults": [
        fault_run(kind, worker_cpus) for kind in ("kill_after_prepare", "partition_after_seal")]}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--client", action="store_true")
    parser.add_argument("--output", type=Path)
    parser.add_argument("--faults", action="store_true")
    args = parser.parse_args()
    if args.client:
        client_main()
        return
    require(args.output is not None, "explicit receipt path required")
    os.umask(0o077)
    require(not args.output.exists(), "refusing to replace receipt")
    result = run_faults() if args.faults else run()
    with args.output.open("x") as handle:
        json.dump(result, handle, indent=2)
        handle.write("\n")
    if args.faults:
        print(json.dumps(result, indent=2))
        return
    print(json.dumps({"ticks": result["total_ticks"], "loop": result["loop_summary"],
                      "schedule_misses_50ms": result["schedule_misses_50ms"],
                      "client_failures": result["client_failures"]}, indent=2))


if __name__ == "__main__":
    main()
