"""Deterministic, networked region simulation lab, NOT a Minecraft runtime.

No production SwarmCraft storage/identity, runtime, quorum or game protocol.
Workers hold their own region state; the trusted ephemeral coordinator
accepts a tick only after all staged work and acknowledgments arrive.
"""
from __future__ import annotations

from dataclasses import dataclass
import hashlib
import hmac
import json
import random
import resource
import socket
import struct
import time

REGIONS = 8
WIDTH = 128
CELLS = REGIONS * WIDTH
MAX_AGENTS = 65_536
MAX_EVENTS = MAX_AGENTS + REGIONS
MAX_FRAME = 4 * 1024 * 1024
MAX_BANK = 10_000_000
MAX_TICK = 1_000_000
VERSION = 1
KEY_BYTES = 32


class Rejected(RuntimeError):
    pass


def require(condition, reason):
    if not condition:
        raise Rejected(reason)


def integer(value, low, high):
    return type(value) is int and low <= value <= high


def fields(value, *keys):
    require(isinstance(value, dict) and set(value) == set(keys), "unexpected message fields")


def sequence(value, n=None, maximum=MAX_EVENTS):
    require(isinstance(value, (list, tuple)) and len(value) <= maximum
            and (n is None or len(value) == n), "invalid bounded sequence")
    return value


def digest(value):
    return isinstance(value, str) and len(value) == 64 and all(c in "0123456789abcdef" for c in value)


def canonical(value):
    return json.dumps(value, separators=(",", ":"), sort_keys=True, allow_nan=False).encode()


@dataclass(frozen=True)
class Region:
    rid: int
    signals: tuple
    banks: tuple
    agents: tuple  # (unique ID, global cell position, integer inventory)

    def wire(self):
        return [self.rid, self.signals, self.banks, self.agents]


def validate_agent(agent, rid):
    i, pos, inventory = sequence(agent, 3)
    require(integer(i, 0, MAX_AGENTS - 1) and integer(pos, rid * WIDTH, (rid + 1) * WIDTH - 1)
            and integer(inventory, 0, MAX_BANK), "invalid agent or owner")
    return i, pos, inventory


def read_region(value):
    rid, signals, banks, agents = sequence(value, 4)
    require(integer(rid, 0, REGIONS - 1), "invalid region")
    require(all(integer(v, 0, 255) for v in sequence(signals, WIDTH)), "invalid signal")
    require(all(integer(v, 0, MAX_BANK) for v in sequence(banks, WIDTH)), "invalid resource bank")
    agents = tuple(validate_agent(a, rid) for a in sequence(agents, maximum=MAX_AGENTS))
    ids = [a[0] for a in agents]
    require(all(a < b for a, b in zip(ids, ids[1:])), "duplicate or unordered agent IDs")
    return Region(rid, tuple(signals), tuple(banks), agents)


def initial_world(shape, count, seed=56):
    require(shape in ("spread", "clustered", "boundary") and integer(count, 1, MAX_AGENTS), "invalid fixture")
    rng = random.Random(seed)
    agents = [[] for _ in range(REGIONS)]
    for i in range(count):
        if shape == "clustered":
            rid = 0 if i < count * 85 // 100 else 1 + i % (REGIONS - 1)
            pos = rng.randrange(1, WIDTH - 1)
        else:
            # At tick 1, even IDs move left and odd IDs move right.
            pos = (WIDTH - 1 if i % 2 else 0) if shape == "boundary" else rng.randrange(WIDTH)
            # Avoid parity confounding with worker/region assignment.
            rid = (i // 2) % REGIONS
        agents[rid].append((i, rid * WIDTH + pos, rng.randrange(1, 9)))
    return {
        rid: Region(rid, tuple(rng.randrange(256) for _ in range(WIDTH)),
                    tuple(rng.randrange(10, 60) for _ in range(WIDTH)), tuple(agents[rid]))
        for rid in range(REGIONS)
    }


def region_summary(region):
    # A stable binary encoding independent of JSON and process ordering.
    payload = bytearray(struct.pack(">II", region.rid, len(region.agents)))
    payload.extend(bytes(region.signals))
    payload.extend(struct.pack(">" + "I" * WIDTH, *region.banks))
    for agent in region.agents:
        payload.extend(struct.pack(">III", *agent))
    return [region.rid, hashlib.sha256(payload).hexdigest(), len(region.agents),
            sum(region.banks) + sum(a[2] for a in region.agents), region.signals[0], region.signals[-1]]


def checked_summaries(rows, owned):
    result = {}
    for row in sequence(rows, maximum=REGIONS):
        rid, state_hash, count, tokens, left, right = sequence(row, 6)
        require(integer(rid, 0, REGIONS - 1) and rid in owned and rid not in result
                and digest(state_hash) and integer(count, 0, MAX_AGENTS)
                and integer(tokens, 0, MAX_BANK * (CELLS + MAX_AGENTS))
                and integer(left, 0, 255) and integer(right, 0, 255), "invalid region receipt")
        result[rid] = list(row)
    require(set(result) == set(owned), "incomplete region receipt set")
    return result


def world_root(tick, rows):
    require(integer(tick, 0, MAX_TICK), "invalid tick")
    summaries = checked_summaries(rows, range(REGIONS))
    content = b"swarmcraft-region-lab-v1\0" + struct.pack(">Q", tick)
    return hashlib.sha256(content + b"".join(bytes.fromhex(summaries[r][1]) for r in range(REGIONS))).hexdigest()


def totals(rows):
    return sum(row[2] for row in rows), sum(row[3] for row in rows)


def region_roots(regions):
    return [region_summary(regions[rid]) for rid in sorted(regions)]


def movement(i, tick):
    return 1 if (i + tick) % 2 == 0 else -1


def checked_controls(rows, resident=None):
    """Bounded, unique one-cell inputs from the trusted lab coordinator."""
    result = {}
    for row in sequence(rows, maximum=MAX_AGENTS):
        sequence(row, maximum=3)
        require(len(row) in (2, 3), "invalid control width")
        agent, direction = row[:2]
        require(integer(agent, 0, MAX_AGENTS - 1) and integer(direction, -1, 1)
                and agent not in result and (resident is None or agent in resident),
                "invalid, duplicate or nonresident player control")
        if len(row) == 3:
            require(integer(row[2], -1, 1), "invalid inventory action")
            result[agent] = (direction, row[2])
        else:
            result[agent] = direction
    return result


def serial_step(regions, tick, counters=None, *, controls=None):
    """Monolithic reference; no event routing, worker state or halo protocol.

    Direct whole-world arrays and sorted entities implement the same
    discrete rules independently of partition staging and delivery.
    """
    controls = {} if controls is None else controls
    old_signals = [s for rid in range(REGIONS) for s in regions[rid].signals]
    banks = [v for rid in range(REGIONS) for v in regions[rid].banks]
    agents = sorted(a for region in regions.values() for a in region.agents)
    next_agents = [[] for _ in range(REGIONS)]
    occupancy = [0] * CELLS
    crossings = 0
    for i, pos, inventory in agents:
        occupancy[pos] += 1
        control = controls.get(i, movement(i, tick))
        if isinstance(control, tuple):
            direction, action = control
            if action == 1 and banks[pos] and inventory < MAX_BANK:
                banks[pos] -= 1
                inventory += 1
            elif action == -1 and inventory and banks[pos] < MAX_BANK:
                banks[pos] += 1
                inventory -= 1
        else:
            direction = control
            if (i + tick) % 5 == 0 and banks[pos]:
                banks[pos] -= 1
                inventory += 1
            if (i + tick) % 7 == 0 and inventory:
                banks[pos] += 1
                inventory -= 1
        destination = (pos + direction) % CELLS
        next_agents[destination // WIDTH].append((i, destination, inventory))
        crossings += destination // WIDTH != pos // WIDTH
    signals = [(2 * old_signals[pos] + old_signals[(pos - 1) % CELLS]
                + old_signals[(pos + 1) % CELLS] + occupancy[pos]) & 255 for pos in range(CELLS)]
    moves = [pos for pos in range(CELLS) if (pos + tick) % 4 == 0 and banks[pos]]
    for pos in moves:
        banks[pos] -= 1
        banks[(pos + 1) % CELLS] += 1
    if counters is not None:
        counters["region_agents"] = crossings
        counters["region_resources"] = sum(pos % WIDTH == WIDTH - 1 for pos in moves)
    return {
        rid: Region(rid, tuple(signals[rid * WIDTH:(rid + 1) * WIDTH]),
                    tuple(banks[rid * WIDTH:(rid + 1) * WIDTH]), tuple(next_agents[rid]))
        for rid in range(REGIONS)
    }


def propose(region, tick, left, right, controls=None):
    """Pure region step. Outgoing messages carry exactly the debited state."""
    controls = {} if controls is None else controls
    rid = region.rid
    base = rid * WIDTH
    banks = list(region.banks)
    occupancy = [0] * WIDTH
    agents = []
    events = []
    for i, pos, inventory in region.agents:
        index = pos - base
        occupancy[index] += 1
        control = controls.get(i, movement(i, tick))
        if isinstance(control, tuple):
            direction, action = control
            if action == 1 and banks[index] > 0 and inventory < MAX_BANK:
                banks[index] -= 1
                inventory += 1
            if action == -1 and inventory > 0 and banks[index] < MAX_BANK:
                banks[index] += 1
                inventory -= 1
        else:
            direction = control
            if (i + tick) % 5 == 0 and banks[index] > 0:
                banks[index] -= 1
                inventory += 1
            if (i + tick) % 7 == 0 and inventory > 0:
                banks[index] += 1
                inventory -= 1
        dst = (pos + direction) % CELLS
        if dst // WIDTH == rid:
            agents.append((i, dst, inventory))
        else:
            events.append(["e", rid, dst // WIDTH, i, dst, inventory])
    signals = tuple((2 * old + (region.signals[idx - 1] if idx else left)
                     + (region.signals[idx + 1] if idx + 1 < WIDTH else right)
                     + occupancy[idx]) & 255 for idx, old in enumerate(region.signals))
    # Select debits before applying any incoming credits.
    moves = [idx for idx, value in enumerate(banks) if value and (base + idx + tick) % 4 == 0]
    for idx in moves:
        banks[idx] -= 1
        if idx + 1 < WIDTH:
            banks[idx + 1] += 1
        else:
            events.append(["t", rid, (rid + 1) % REGIONS, 1])
    return Region(rid, signals, tuple(banks), tuple(agents)), events


def check_event(event):
    sequence(event)
    require(len(event) >= 3, "short boundary event")
    kind, src, dst = event[:3]
    require(integer(src, 0, REGIONS - 1) and integer(dst, 0, REGIONS - 1)
            and dst in ((src - 1) % REGIONS, (src + 1) % REGIONS), "invalid boundary endpoints")
    if kind == "e":
        require(len(event) == 6, "invalid move event")
        i, pos, inventory = validate_agent(event[3:], dst)
        require(pos == (dst * WIDTH if dst == (src + 1) % REGIONS else (dst + 1) * WIDTH - 1),
                "nonadjacent agent transfer")
        return ("e", i)
    require(kind == "t" and len(event) == 4 and dst == (src + 1) % REGIONS
            and type(event[3]) is int and event[3] == 1, "invalid resource transfer")
    return ("t", src)


def deliver(pending, events):
    # Apply on copies; malformed/duplicated input cannot change pending
    # or committed state. Sort once per region to keep order deterministic.
    banks = {rid: list(region.banks) for rid, region in pending.items()}
    agents = {rid: list(region.agents) for rid, region in pending.items()}
    resident = {a[0] for region in pending.values() for a in region.agents}
    require(len(resident) == sum(map(len, agents.values())), "duplicate resident ID")
    seen = set()
    for event in sequence(events):
        key = check_event(event)
        require(key not in seen, "duplicate boundary event")
        seen.add(key)
        dst = event[2]
        require(dst in pending, "boundary event not assigned here")
        if event[0] == "e":
            require(event[3] not in resident, "duplicate incoming agent")
            resident.add(event[3])
            agents[dst].append(tuple(event[3:]))
        else:
            banks[dst][0] += event[3]
    return {
        rid: Region(rid, region.signals, tuple(banks[rid]), tuple(sorted(agents[rid])))
        for rid, region in pending.items()
    }


class Worker:
    """One trusted worker session. Never accepts reassignment or a stale tick."""

    def __init__(self):
        self.regions = None
        self.tick = 0
        self.phase = "new"
        self.pending = None
        self.receipt = None
        self.cpu_ns = 0

    def handle(self, req):
        require(isinstance(req, dict), "invalid request")
        op = req.get("op")
        if op in ("init", "init_at"):
            fields(req, "op", "version", "regions", *("tick",) if op == "init_at" else ())
            require(self.phase == "new" and integer(req["version"], VERSION, VERSION), "invalid init")
            tick = req.get("tick", 0)
            require(integer(tick, 0, MAX_TICK), "invalid initial tick")
            owned = [read_region(row) for row in sequence(req["regions"], maximum=REGIONS)]
            require(owned and len({r.rid for r in owned}) == len(owned), "duplicate ownership")
            ids = [a[0] for r in owned for a in r.agents]
            require(len(set(ids)) == len(ids), "duplicate initial ID")
            self.regions = {r.rid: r for r in owned}
            self.tick = tick
            self.phase = "idle"
            reply = {"op": op, "rows": region_roots(self.regions)}
            if op == "init_at":
                reply["tick"] = tick
            return reply
        if op == "ping":
            fields(req, "op")
            return {"op": "pong"}
        if op == "status":
            fields(req, "op")
            return {"op": "status", "tick": self.tick, "phase": self.phase}
        if op in ("prepare", "prepare_controlled"):
            controlled = op == "prepare_controlled"
            fields(req, "op", "tick", "halo", *("controls",) if controlled else ())
            tick = req["tick"]
            require(self.phase == "idle" and integer(tick, 1, MAX_TICK) and tick == self.tick + 1,
                    "stale or overlapping prepare")
            halo = {}
            for item in sequence(req["halo"], maximum=REGIONS):
                rid, left, right = sequence(item, 3)
                require(integer(rid, 0, REGIONS - 1) and rid in self.regions and rid not in halo
                        and integer(left, 0, 255) and integer(right, 0, 255), "invalid halo")
                halo[rid] = left, right
            require(set(halo) == set(self.regions), "missing halo")
            controls = {}
            if controlled:
                seen_regions = set()
                for item in sequence(req["controls"], maximum=REGIONS):
                    rid, rows = sequence(item, 2)
                    require(type(rid) is int and rid in self.regions and rid not in seen_regions,
                            "invalid control region ownership")
                    seen_regions.add(rid)
                    controls.update(checked_controls(rows, {a[0] for a in self.regions[rid].agents}))
                require(seen_regions == set(self.regions), "missing control region")
            start = time.process_time_ns()
            pending, local, external = {}, [], []
            for rid, region in self.regions.items():
                next_region, events = propose(region, tick, *halo[rid], controls=controls)
                pending[rid] = next_region
                for event in events:
                    (local if event[2] in self.regions else external).append(event)
            self.pending = deliver(pending, local)
            self.cpu_ns += time.process_time_ns() - start
            self.phase = "prepared"
            return {"op": "prepared", "tick": tick, "events": external,
                    "local_agents": sum(e[0] == "e" for e in local),
                    "local_resources": sum(e[0] == "t" for e in local)}
        if op == "seal":
            fields(req, "op", "tick", "events")
            require(self.phase == "prepared" and integer(req["tick"], self.tick + 1, self.tick + 1), "stale seal")
            for event in sequence(req["events"]):
                check_event(event)
                require(event[1] not in self.regions and event[2] in self.regions, "wrong worker delivery")
            start = time.process_time_ns()
            self.pending = deliver(self.pending, req["events"])
            rows = region_roots(self.pending)
            self.receipt = hashlib.sha256(canonical(rows)).hexdigest()
            self.cpu_ns += time.process_time_ns() - start
            self.phase = "sealed"
            return {"op": "sealed", "tick": req["tick"], "rows": rows, "receipt": self.receipt}
        if op == "commit":
            fields(req, "op", "tick", "root", "receipt")
            require(self.phase == "sealed" and integer(req["tick"], self.tick + 1, self.tick + 1)
                    and digest(req["root"]) and req["receipt"] == self.receipt, "commit not bound to seal")
            self.regions, self.pending = self.pending, None
            self.tick = req["tick"]
            self.receipt = None
            self.phase = "idle"
            return {"op": "committed", "tick": self.tick, "root": req["root"]}
        if op == "stats":
            fields(req, "op")
            require(self.phase == "idle", "stats during staged work")
            return {"op": "stats", "tick": self.tick,
                    "compute_cpu_ns": self.cpu_ns, "process_cpu_ns": time.process_time_ns(),
                    "peak_rss_kib": resource.getrusage(resource.RUSAGE_SELF).ru_maxrss}
        if op == "snapshot":
            fields(req, "op")
            require(self.phase == "idle", "snapshot during staged work")
            return {"op": "snapshot", "tick": self.tick,
                    "regions": [self.regions[r].wire() for r in sorted(self.regions)],
                    "compute_cpu_ns": self.cpu_ns, "process_cpu_ns": time.process_time_ns(),
                    "peak_rss_kib": resource.getrusage(resource.RUSAGE_SELF).ru_maxrss}
        raise Rejected("unknown operation")


def remaining(deadline):
    result = deadline - time.monotonic()
    if result <= 0:
        raise TimeoutError("lab barrier deadline exceeded")
    return result


class Channel:
    """Bounded framed TCP with per-direction HMAC and monotonic sequence.

    Only the private lab session has these keys. No encryption, SwarmCraft
    authentication, remote-worker trust, or reusable production endpoint.
    """

    def __init__(self, sock, key, coordinator):
        require(isinstance(key, bytes) and len(key) == KEY_BYTES, "invalid session key")
        self.sock, self.key = sock, key
        self.tx_tag, self.rx_tag = (b"C", b"W") if coordinator else (b"W", b"C")
        self.tx_seq = self.rx_seq = 0
        self.sent = self.received = 0
        self.sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)

    def send(self, msg, deadline):
        encoded = canonical(msg)
        require(len(encoded) <= MAX_FRAME - 40, "oversized message")
        seq = struct.pack(">Q", self.tx_seq)
        auth = hmac.digest(self.key, self.tx_tag + seq + encoded, "sha256")
        body = seq + auth + encoded
        frame = struct.pack(">I", len(body)) + body
        self.sock.settimeout(remaining(deadline))
        self.sock.sendall(frame)
        self.tx_seq += 1
        self.sent += len(frame)

    def _read(self, size, deadline):
        data = bytearray()
        while len(data) < size:
            self.sock.settimeout(remaining(deadline))
            part = self.sock.recv(size - len(data))
            if not part:
                raise EOFError("worker connection closed")
            data.extend(part)
        return bytes(data)

    def recv(self, deadline):
        size, = struct.unpack(">I", self._read(4, deadline))
        require(40 < size <= MAX_FRAME, "invalid frame size")
        body = self._read(size, deadline)
        seq, auth, payload = body[:8], body[8:40], body[40:]
        require(seq == struct.pack(">Q", self.rx_seq), "stale or reordered frame")
        expected = hmac.digest(self.key, self.rx_tag + seq + payload, "sha256")
        require(hmac.compare_digest(auth, expected), "invalid lab frame authentication")
        msg = json.loads(payload)
        require(isinstance(msg, dict), "non-object frame")
        self.rx_seq += 1
        self.received += 4 + size
        return msg

    def close(self):
        self.sock.close()


class Coordinator:
    """Static partition with one accepted in-memory cut; no durable quorum."""

    def __init__(self, channels, regions, timeout=3.0, *, placement=None, start_tick=0):
        # Explicit layouts/resume are research-only. The previous static
        # lab retains its exact init messages and default partition.
        require(integer(start_tick, 0, MAX_TICK) and len(channels) in (1, 2, 3, 4),
                "unsupported worker count or start tick")
        require(set(regions) == set(range(REGIONS))
                and all(type(r) is int and regions[r].rid == r for r in regions),
                "incomplete or inconsistent initial world")
        if placement is None:
            require(len(channels) in (1, 2, 4) and start_tick == 0, "explicit placement required")
            placement = {r: r * len(channels) // REGIONS for r in range(REGIONS)}
        require(set(placement) == set(range(REGIONS))
                and all(type(r) is int for r in placement)
                and all(integer(slot, 0, len(channels)-1) for slot in placement.values())
                and set(placement.values()) == set(range(len(channels))), "invalid placement")
        ids = [a[0] for region in regions.values() for a in region.agents]
        require(len(ids) == len(set(ids)) <= MAX_AGENTS, "duplicate or excessive initial agents")
        self.channels = list(channels)
        self.owned = [set() for _ in channels]
        self.owner = dict(placement)
        for rid, slot in self.owner.items():
            self.owned[slot].add(rid)
        self.tick = start_tick
        self.poisoned = False
        self.timeout = timeout
        rows = region_roots(regions)
        self.invariants = totals(rows)
        self.summaries = checked_summaries(rows, range(REGIONS))
        self.root = world_root(start_tick, rows)
        init = {"op": "init_at", "version": VERSION, "tick": start_tick} if start_tick else {
            "op": "init", "version": VERSION}
        replies = self.exchange([{**init, "regions": [regions[r].wire() for r in sorted(owned)]}
                                 for owned in self.owned], time.monotonic() + timeout)
        for slot, reply in enumerate(replies):
            fields(reply, "op", "rows", *("tick",) if start_tick else ())
            require(reply["op"] == init["op"] and reply.get("tick", 0) == start_tick, "unexpected init receipt")
            require(checked_summaries(reply["rows"], self.owned[slot]) ==
                    {r: self.summaries[r] for r in self.owned[slot]}, "initial state mismatch")

    def exchange(self, requests, deadline):
        require(len(requests) == len(self.channels), "incomplete request set")
        for channel, request in zip(self.channels, requests):
            channel.send(request, deadline)
        return [channel.recv(deadline) for channel in self.channels]

    def step(self, fault=None, *, controls=None):
        require(not self.poisoned and self.tick < MAX_TICK, "run frozen")
        start = time.monotonic_ns()
        deadline = time.monotonic() + self.timeout
        tick = self.tick + 1
        try:
            requests = [
                {"op": "prepare", "tick": tick,
                 "halo": [[r, self.summaries[(r - 1) % REGIONS][5],
                           self.summaries[(r + 1) % REGIONS][4]] for r in sorted(owned)]}
                for owned in self.owned]
            if controls is not None:
                require(isinstance(controls, dict) and all(type(r) is int and r in self.owner
                                                          for r in controls), "invalid control region")
                seen_inputs = set()
                for rid, rows in controls.items():
                    ids = set(checked_controls(rows))
                    require(not seen_inputs.intersection(ids), "duplicate control across regions")
                    seen_inputs.update(ids)
                for request, owned in zip(requests, self.owned):
                    request["op"] = "prepare_controlled"
                    request["controls"] = [[rid, controls.get(rid, [])] for rid in sorted(owned)]
            replies = self.exchange(requests, deadline)
            inboxes = [[] for _ in self.channels]
            seen = set()
            transfers = resources = local_agents = local_resources = 0
            for slot, reply in enumerate(replies):
                fields(reply, "op", "tick", "events", "local_agents", "local_resources")
                require(reply["op"] == "prepared" and integer(reply["tick"], tick, tick)
                        and integer(reply["local_agents"], 0, MAX_AGENTS)
                        and integer(reply["local_resources"], 0, REGIONS), "stale prepare receipt")
                local_agents += reply["local_agents"]
                local_resources += reply["local_resources"]
                for event in sequence(reply["events"]):
                    key = check_event(event)
                    require(event[1] in self.owned[slot] and self.owner[event[2]] != slot
                            and key not in seen, "duplicate or unauthorized boundary event")
                    seen.add(key)
                    inboxes[self.owner[event[2]]].append(event)
                    if event[0] == "e":
                        transfers += 1
                    else:
                        resources += 1
            require(len(seen) <= MAX_EVENTS
                    and local_agents + transfers <= self.invariants[0]
                    and local_resources + resources <= REGIONS, "excessive boundary events")
            if fault:
                fault("prepared")
            sealed = self.exchange([{"op": "seal", "tick": tick, "events": inbox}
                                    for inbox in inboxes], deadline)
            summaries = {}
            for slot, reply in enumerate(sealed):
                fields(reply, "op", "tick", "rows", "receipt")
                require(reply["op"] == "sealed" and reply["tick"] == tick
                        and reply["receipt"] == hashlib.sha256(canonical(reply["rows"])).hexdigest(),
                        "stale or inconsistent seal")
                summaries.update(checked_summaries(reply["rows"], self.owned[slot]))
            require(set(summaries) == set(range(REGIONS)) and totals(summaries.values()) == self.invariants,
                    "world conservation or ownership mismatch")
            root = world_root(tick, list(summaries.values()))
            if fault:
                fault("sealed")
            replies = self.exchange([{"op": "commit", "tick": tick, "root": root, "receipt": reply["receipt"]}
                                     for reply in sealed], deadline)
            require(all(reply == {"op": "committed", "tick": tick, "root": root} for reply in replies),
                    "missing commit acknowledgment")
            # This is the ONLY accepted cut. Some workers might have a
            # tentative next state if a commit delivery/ack fails. Such a
            # run is unusable, and no further tick can be accepted.
            self.tick, self.root, self.summaries = tick, root, summaries
            return {"tick": tick, "root": root, "elapsed_ms": (time.monotonic_ns() - start) / 1e6,
                    "cross_worker_agents": transfers, "cross_worker_resources": resources,
                    "region_agents": local_agents + transfers, "region_resources": local_resources + resources}
        except BaseException:
            self.poisoned = True
            raise

    def stats(self):
        require(not self.poisoned, "frozen run")
        replies = self.exchange([{"op": "stats"} for _ in self.channels], time.monotonic() + self.timeout)
        for reply in replies:
            fields(reply, "op", "tick", "compute_cpu_ns", "process_cpu_ns", "peak_rss_kib")
            require(reply["op"] == "stats" and integer(reply["tick"], self.tick, self.tick)
                    and all(integer(reply[k], 0, 1 << 63) for k in
                            ("compute_cpu_ns", "process_cpu_ns", "peak_rss_kib")), "bad stats")
        return replies

    def snapshot(self):
        require(not self.poisoned, "frozen run cannot export a new cut")
        replies = self.exchange([{"op": "snapshot"} for _ in self.channels], time.monotonic() + self.timeout)
        state = {}
        metrics = []
        for slot, reply in enumerate(replies):
            fields(reply, "op", "tick", "regions", "compute_cpu_ns", "process_cpu_ns", "peak_rss_kib")
            require(reply["op"] == "snapshot" and reply["tick"] == self.tick, "wrong snapshot tick")
            rows = [read_region(row) for row in sequence(reply["regions"], maximum=REGIONS)]
            require(len(rows) == len(self.owned[slot]) and {r.rid for r in rows} == self.owned[slot],
                    "invalid snapshot ownership")
            state.update({r.rid: r for r in rows})
            metrics.append({key: reply[key] for key in ("compute_cpu_ns", "process_cpu_ns", "peak_rss_kib")})
        require(world_root(self.tick, region_roots(state)) == self.root, "snapshot differs from accepted cut")
        ids = [a[0] for region in state.values() for a in region.agents]
        require(len(ids) == len(set(ids)) == self.invariants[0], "duplicate or missing agent")
        return state, metrics
