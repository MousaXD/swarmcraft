"""#60 isolated durable regional authority experiment, NOT production consensus.

One trusted local SQLite ledger is the only canonical publisher. Workers can
compute tentatively, including after their grants are retired; no worker can
publish a cut. A complete set of current-generation authenticated receipts is
required in the same durable transaction as the global state and tick CAS.
"""
from __future__ import annotations

from contextlib import contextmanager, nullcontext
import hashlib
import hmac
import json
import os
from pathlib import Path
import secrets
import sqlite3
import time

from player_runtime import MAX_PLAYERS, RULESET, cut_digest
from region_runtime import (KEY_BYTES, MAX_AGENTS, MAX_TICK, REGIONS, Coordinator, Worker,
                            canonical, digest, fields, integer, read_region,
                            region_roots, region_roots_bulk, require, sequence, totals, world_root)

SCHEMA = 1
APP_ID = 0x53433630
MAX_GENERATION = 1_000_000


def validated_state(rows):
    regions = [read_region(row) for row in sequence(rows, REGIONS, REGIONS)]
    return _assembled_state(regions)


def _assembled_state(regions):
    """Global validation of regions already parsed by read_region.

    Only internal callers with freshly validated, immutable Regions may
    use this helper. It is not an alternate untrusted input parser.
    """
    require([r.rid for r in regions] == list(range(REGIONS)), "incomplete or unordered world")
    ids = sorted(a[0] for r in regions for a in r.agents)
    require(len(ids) == len(set(ids)) <= MAX_AGENTS, "duplicate or excessive world identities")
    return {r.rid: r for r in regions}, ids


def wire(state):
    return [state[r].wire() for r in sorted(state)]


def checked_groups(groups):
    groups = [list(sequence(g, maximum=REGIONS)) for g in sequence(groups, maximum=REGIONS)]
    require(1 <= len(groups) <= 4 and all(g and
            all(integer(r, 0, REGIONS - 1) for r in g) and g == sorted(set(g))
            for g in groups) and sorted(r for g in groups for r in g) == list(range(REGIONS)),
            "overlapping or incomplete region placement")
    return groups


def checked_grant(grant):
    fields(grant, "world", "generation", "session", "regions", "base_tick", "base_root", "key")
    require(digest(grant["world"]) and digest(grant["session"]) and digest(grant["key"])
            and digest(grant["base_root"]) and integer(grant["generation"], 1, MAX_GENERATION)
            and integer(grant["base_tick"], 0, MAX_TICK), "invalid regional grant")
    regions = sequence(grant["regions"], maximum=REGIONS)
    require(regions and all(integer(r, 0, REGIONS - 1) for r in regions)
            and list(regions) == sorted(set(regions)), "invalid grant region scope")
    return grant


def tag(key, body):
    return hmac.digest(bytes.fromhex(key), b"sc60-region-receipt-v1\0" + canonical(body),
                       "sha256").hex()


def tag_encoded(key, encoded):
    h = hmac.new(bytes.fromhex(key), b"sc60-region-receipt-v1\0", "sha256")
    h.update(encoded)
    return h.hexdigest()


def encoded_receipt(receipt, body, encoded_body):
    """Reuse authenticated bytes in the exact canonical receipt envelope.

    Policy adapters may add fields or return an inner receipt. Preserve
    every original field in its canonical order; reuse only the body
    object actually authenticated. The caller never supplies these bytes.
    """
    return b"{" + b",".join(
        canonical(key) + b":" +
        (encoded_body if key == "body" and receipt[key] is body else canonical(receipt[key]))
        for key in sorted(receipt)) + b"}"


def encoded_receipt_hash(receipts):
    """SHA-256 of the original canonical sorted receipt array."""
    h = hashlib.sha256(b"[")
    for index, (_, encoded) in enumerate(sorted(receipts)):
        if index:
            h.update(b",")
        h.update(encoded)
    h.update(b"]")
    return h.hexdigest()


def new_grants(world, generation, tick, root, groups):
    return [checked_grant({"world": world, "generation": generation,
                           "session": secrets.token_hex(KEY_BYTES), "regions": group,
                           "base_tick": tick, "base_root": root,
                           "key": secrets.token_hex(KEY_BYTES)}) for group in checked_groups(groups)]


def public_grants(grants):
    return [{k: v for k, v in g.items() if k != "key"} for g in grants]


class FencedWorker:
    """Immutable lab grant, fresh process; signs only locally committed snapshots."""
    def __init__(self):
        self.worker = Worker()
        self.grant = None
        self.current_root = None
        self.parent_root = None

    def handle(self, request):
        require(isinstance(request, dict), "invalid fenced request")
        op = request.get("op")
        if op == "bind_grant":
            fields(request, "op", "grant")
            require(self.grant is None and self.worker.phase == "new", "grant already bound")
            grant = checked_grant(request["grant"])
            self.grant = grant
            self.current_root = grant["base_root"]
            return {"op": "bound", "session": grant["session"], "generation": grant["generation"]}
        if self.grant is None and op in ("ping", "status"):
            return self.worker.handle(request)
        require(self.grant is not None, "missing regional grant")
        grant = self.grant
        if op in ("init", "init_at"):
            regions = [read_region(row) for row in sequence(request.get("regions"), maximum=REGIONS)]
            require([r.rid for r in regions] == grant["regions"] and
                    request.get("tick", 0) == grant["base_tick"], "init outside grant")
        if op == "commit_snapshot":
            fields(request, "op", "tick", "root", "receipt")
            # Use the same local seal-bound commit and receipt generation.
            # Dynamic dispatch preserves additional receipt policy bindings
            # (e.g. PolicyWorker); there is no weaker signing path.
            ack = self.handle({**request, "op": "commit"})
            return {**ack, "op": "committed_snapshot",
                    "snapshot": self.handle({"op": "fenced_snapshot"})}
        if op == "fenced_snapshot":
            fields(request, "op")
            require(self.worker.phase == "idle" and self.parent_root is not None,
                    "no locally committed candidate")
            body = {"world": grant["world"], "generation": grant["generation"],
                    "session": grant["session"], "tick": self.worker.tick,
                    "parent_root": self.parent_root, "runtime_root": self.current_root,
                    "regions": wire(self.worker.regions)}
            return {"body": body, "mac": tag(grant["key"], body)}
        reply = self.worker.handle(request)
        if op == "commit":
            self.parent_root, self.current_root = self.current_root, request["root"]
        return reply


class Authority:
    """Durable local serialization point. Never exposes its database to workers."""
    app_id = APP_ID
    schema = SCHEMA
    ruleset = RULESET
    reuse_validated_candidate = True
    optimize_publication = True

    def _publication_roots(self, state):
        return region_roots_bulk(state) if self.optimize_publication else region_roots(state)

    @staticmethod
    def cut(world, tick, root, players):
        return cut_digest(world, tick, root, players)

    @classmethod
    def _create_extra(cls, db, state):
        pass

    def _record_candidate(self, current, state, candidate, cut):
        pass

    def _before_claim(self, current):
        pass

    def _validate_receipt(self, current, receipt):
        return receipt

    def __init__(self, path, *, read_only=False):
        self.path = Path(path).resolve()
        require(type(read_only) is bool, "invalid authority access mode")
        self.read_only = read_only
        # One-entry cache of validated immutable state, keyed by the exact
        # SQLite BLOB read in the current transaction (not a tick or root).
        # Never cache metadata, grants, journal decisions, or DB transactions.
        self._state_cache = None
        self.phase = nullcontext  # optional benchmark-only timing context
        self.transaction_event = None  # optional boundary-only benchmark observer
        # Neither mode creates a missing authority. Recovery inspection
        # must not write to the original even if a caller tries to rotate.
        self.db = sqlite3.connect(self.path.as_uri() + ("?mode=ro" if read_only else "?mode=rw"), uri=True,
                                  isolation_level=None, timeout=5)
        try:
            if read_only:
                self.db.execute("PRAGMA query_only=ON")
            self.db.execute("PRAGMA synchronous=FULL")
            require(self.db.execute("PRAGMA application_id").fetchone()[0] == self.app_id
                    and self.db.execute("PRAGMA user_version").fetchone()[0] == self.schema
                    and self.db.execute("PRAGMA journal_mode").fetchone()[0] == "delete",
                    "invalid local authority database")
            self.read()
        except BaseException:
            self.db.close()
            raise

    @classmethod
    def create(cls, path, world, state, players, groups):
        require(digest(world), "invalid world ID")
        state, ids = validated_state(wire(state))
        players = list(players)
        require(1 <= len(players) <= MAX_PLAYERS and all(integer(p, 0, MAX_AGENTS - 1) for p in players)
                and players == sorted(set(players)) and set(players) <= set(ids),
                "invalid player roster")
        root = world_root(0, region_roots(state))
        grants = new_grants(world, 1, 0, root, groups)
        cut = cls.cut(world, 0, root, players)
        path = Path(path)
        fd = os.open(path, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
        os.close(fd)
        db = sqlite3.connect(path, isolation_level=None)
        try:
            db.execute("PRAGMA journal_mode=DELETE")
            db.execute("PRAGMA synchronous=FULL")
            db.execute("BEGIN IMMEDIATE")
            db.execute(f"PRAGMA application_id={cls.app_id}")
            db.execute(f"PRAGMA user_version={cls.schema}")
            db.execute("""CREATE TABLE world(
                singleton INTEGER PRIMARY KEY CHECK(singleton=1),
                identity TEXT NOT NULL, ruleset TEXT NOT NULL, players BLOB NOT NULL,
                ids BLOB NOT NULL, resources INTEGER NOT NULL, generation INTEGER NOT NULL,
                accepted_generation INTEGER NOT NULL, claimed INTEGER NOT NULL CHECK(claimed IN (0,1)), tick INTEGER NOT NULL,
                runtime_root TEXT NOT NULL, cut_root TEXT NOT NULL,
                state BLOB NOT NULL, grants BLOB NOT NULL)""")
            db.execute("""CREATE TABLE commits(
                tick INTEGER PRIMARY KEY, generation INTEGER NOT NULL,
                runtime_root TEXT NOT NULL, cut_root TEXT NOT NULL,
                parent_root TEXT, receipt_hash TEXT)""")
            db.execute("""CREATE TABLE epochs(
                generation INTEGER PRIMARY KEY, base_tick INTEGER NOT NULL,
                base_root TEXT NOT NULL, grants BLOB NOT NULL, claimed INTEGER NOT NULL CHECK(claimed IN (0,1)))""")
            db.execute("INSERT INTO world VALUES(1,?,?,?,?,?,?,?,?,?,?,?,?,?)",
                       (world, cls.ruleset, canonical(players), canonical(ids), totals(region_roots(state))[1],
                        1, 0, 0, 0, root, cut, canonical(wire(state)), canonical(grants)))
            db.execute("INSERT INTO commits VALUES(0,0,?,?,NULL,NULL)", (root, cut))
            db.execute("INSERT INTO epochs VALUES(1,0,?,?,0)", (root, canonical(public_grants(grants))))
            cls._create_extra(db, state)
            db.execute("COMMIT")
        finally:
            db.close()
        # Persist directory entry on this Linux research host.
        directory = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
        return cls(path)

    def close(self):
        self.db.close()

    @contextmanager
    def transaction(self, write=False):
        require(not write or not self.read_only, "read-only authority inspection")
        with self.phase("sqlite_begin"):
            self.db.execute("BEGIN IMMEDIATE" if write else "BEGIN")
        try:
            if self.transaction_event is not None:
                self.transaction_event("write" if write else "read")
            yield
            with self.phase("sqlite_durable_commit" if write else "sqlite_read_commit"):
                self.db.execute("COMMIT")
            if self.transaction_event is not None:
                self.transaction_event("commit")
        except BaseException:
            if self.db.in_transaction:
                self.db.execute("ROLLBACK")
                if self.transaction_event is not None:
                    self.transaction_event("rollback")
            raise

    def _remember_state(self, raw, state, ids, rows):
        self._state_cache = (raw, tuple(state.items()), tuple(ids),
                             tuple(tuple(row) for row in rows))

    def _decode_state(self, raw):
        with self.phase("state_decode_roots"):
            cached = self._state_cache
            if cached is None or cached[0] != raw:
                state, ids = validated_state(json.loads(raw))
                rows = region_roots(state)
                # read_region returns frozen Regions with nested tuples;
                # keep only immutable objects and never share a mutable
                # dictionary, ID list or summary list with the caller.
                self._remember_state(raw, state, ids, rows)
                cached = self._state_cache
            return dict(cached[1]), list(cached[2]), cached[3]

    def _read(self):
        with self.phase("ledger_read"):
            return self._read_current()

    def _read_current(self):
        row = self.db.execute("SELECT * FROM world WHERE singleton=1").fetchone()
        require(row is not None, "missing world ledger")
        (_, world, ruleset, players, ids, resources, generation, accepted_generation, claimed,
         tick, runtime_root, cut_root, state_rows, grants_rows) = row
        players, ids, grants = json.loads(players), json.loads(ids), json.loads(grants_rows)
        state, current_ids, rows = self._decode_state(state_rows)
        require(digest(world) and ruleset == self.ruleset and integer(generation, 1, MAX_GENERATION)
                and integer(accepted_generation, 0, generation) and integer(claimed, 0, 1)
                and integer(tick, 0, MAX_TICK) and (accepted_generation != generation or claimed == 1)
                and (accepted_generation == 0) == (tick == 0)
                and ids == current_ids and totals(rows)[1] == resources
                and isinstance(players, list) and 1 <= len(players) <= MAX_PLAYERS and players == sorted(set(players))
                and all(integer(p, 0, MAX_AGENTS - 1) for p in players)
                and set(players) <= set(ids) and runtime_root == world_root(tick, rows)
                and cut_root == self.cut(world, tick, runtime_root, players), "invalid persisted cut")
        grants = [checked_grant(g) for g in sequence(grants, maximum=4)]
        checked_groups([g["regions"] for g in grants])
        require(len({g["session"] for g in grants}) == len(grants)
                and len({g["key"] for g in grants}) == len(grants)
                and all(g["world"] == world and g["generation"] == generation and
                        g["base_tick"] <= tick for g in grants), "invalid persisted grants")
        epoch = self.db.execute("SELECT base_tick,base_root,grants,claimed FROM epochs WHERE generation=?",
                                (generation,)).fetchone()
        require(epoch is not None and all((g["base_tick"], g["base_root"]) == epoch[:2] for g in grants)
                and epoch[2] == canonical(public_grants(grants)) and epoch[3] == claimed,
                "grant history mismatch")
        base = self.db.execute("SELECT runtime_root FROM commits WHERE tick=?", (epoch[0],)).fetchone()
        require(base == (epoch[1],), "grant base not committed")
        head = self.db.execute("SELECT tick,generation,runtime_root,cut_root FROM commits ORDER BY tick DESC LIMIT 1").fetchone()
        require(head == (tick, accepted_generation, runtime_root, cut_root), "commit history mismatch")
        require(self.db.execute("SELECT MAX(generation) FROM epochs").fetchone() == (generation,),
                "generation history mismatch")
        return {"world": world, "ruleset": ruleset, "players": players, "ids": ids,
                "resources": resources, "generation": generation, "accepted_generation": accepted_generation,
                "claimed": bool(claimed), "tick": tick, "runtime_root": runtime_root, "cut_root": cut_root,
                "state": state, "grants": grants}

    def read(self):
        with self.transaction():
            return self._read()

    def save(self):
        """One read transaction: no mixed generations/regions/ticks, no session keys."""
        with self.transaction():
            current = self._read()
            return {**{k: v for k, v in current.items() if k not in ("state", "grants")},
                    "regions": json.loads(canonical(wire(current["state"]))),
                    "grants": public_grants(current["grants"])}

    def rotate(self, expected_generation, expected_tick, expected_root, groups):
        """Replace all sessions, even unchanged placements; stale contenders lose."""
        with self.transaction(write=True):
            current = self._read()
            require(integer(expected_generation, 1, MAX_GENERATION) and
                    integer(expected_tick, 0, MAX_TICK) and digest(expected_root) and
                    (expected_generation, expected_tick, expected_root) ==
                    (current["generation"], current["tick"], current["runtime_root"]),
                    "stale generation or base cut")
            generation = current["generation"] + 1
            require(generation <= MAX_GENERATION, "generation exhausted")
            grants = new_grants(current["world"], generation, current["tick"],
                                current["runtime_root"], groups)
            self.db.execute("UPDATE world SET generation=?,grants=?,claimed=0 WHERE singleton=1",
                            (generation, canonical(grants)))
            self.db.execute("INSERT INTO epochs VALUES(?,?,?,?,0)",
                            (generation, current["tick"], current["runtime_root"],
                             canonical(public_grants(grants))))
        return grants

    def claim(self, expected_generation, expected_tick, expected_root):
        """Exactly one trusted coordinator may launch workers for this generation."""
        with self.transaction(write=True):
            current = self._read()
            require(integer(expected_generation, 1, MAX_GENERATION) and
                    integer(expected_tick, 0, MAX_TICK) and digest(expected_root) and
                    (expected_generation, expected_tick, expected_root) ==
                    (current["generation"], current["tick"], current["runtime_root"]),
                    "stale generation or base cut")
            require(not current["claimed"] and all(g["base_tick"] == current["tick"]
                    and g["base_root"] == current["runtime_root"] for g in current["grants"]),
                    "generation already claimed; rotate before worker restart")
            self._before_claim(current)
            self.db.execute("UPDATE world SET claimed=1 WHERE singleton=1")
            self.db.execute("UPDATE epochs SET claimed=1 WHERE generation=?", (current["generation"],))
            current["claimed"] = True
        return current

    def commit(self, receipts, *, before_commit=None):
        """Atomic all-region cut CAS; untrusted receipt inputs, trusted authority."""
        with self.transaction(write=True):
            current = self._read()
            tick = current["tick"] + 1
            require(current["claimed"] and tick <= MAX_TICK, "unclaimed generation or tick exhausted")
            grants = {g["session"]: g for g in current["grants"]}
            seen, regions = set(), {}
            encoded_receipts = []
            candidate = None
            for original in sequence(receipts, len(grants), 4):
                receipt = self._validate_receipt(current, original)
                fields(receipt, "body", "mac")
                body = receipt["body"]
                fields(body, "world", "generation", "session", "tick", "parent_root", "runtime_root", "regions")
                session = body["session"]
                require(digest(session) and session in grants and session not in seen,
                        "stale, unknown or duplicate session")
                grant = grants[session]
                require(body["world"] == current["world"] and
                        integer(body["generation"], current["generation"], current["generation"])
                        and integer(body["tick"], tick, tick)
                        and body["parent_root"] == current["runtime_root"]
                        and digest(body["runtime_root"]) and digest(receipt["mac"]),
                        "invalid or stale regional receipt")
                if self.optimize_publication:
                    encoded_body = canonical(body)
                    expected_mac = tag_encoded(grant["key"], encoded_body)
                else:
                    expected_mac = tag(grant["key"], body)
                require(hmac.compare_digest(receipt["mac"], expected_mac),
                        "invalid or stale regional receipt")
                owned = [read_region(row) for row in sequence(body["regions"], maximum=REGIONS)]
                require([r.rid for r in owned] == grant["regions"], "receipt outside exclusive grant")
                require(not set(regions).intersection(r.rid for r in owned), "overlapping receipts")
                regions.update((r.rid, r) for r in owned)
                seen.add(session)
                require(candidate is None or candidate == body["runtime_root"], "conflicting cut")
                candidate = body["runtime_root"]
                if self.optimize_publication:
                    encoded_receipts.append((original["body"]["session"],
                                             encoded_receipt(original, body, encoded_body)))
            require(seen == set(grants), "incomplete current receipt set")
            # Every region above was decoded exactly once from a receipt
            # authenticated against its current exclusive grant. Repeating
            # read_region on its immutable wire form cannot add validation.
            # Global coverage, ordering, identity uniqueness and limits
            # still run on the complete set, independently of the workers.
            if self.reuse_validated_candidate:
                state, ids = _assembled_state([regions[r] for r in sorted(regions)])
            else:
                state, ids = validated_state(wire(regions))
            rows = self._publication_roots(state)
            require(ids == current["ids"] and totals(rows)[1] == current["resources"]
                    and candidate == world_root(tick, rows), "inconsistent global cut")
            cut = self.cut(current["world"], tick, candidate, current["players"])
            receipt_hash = (encoded_receipt_hash(encoded_receipts) if self.optimize_publication else
                            hashlib.sha256(canonical(sorted(receipts, key=lambda r: r["body"]["session"]))).hexdigest())
            with self.phase("independent_candidate_validation"):
                self._record_candidate(current, dict(state), candidate, cut)
            with self.phase("checkpoint_serialization"):
                encoded_state = canonical(wire(state))
            with self.phase("canonical_journal_write"):
                self.db.execute("INSERT INTO commits VALUES(?,?,?,?,?,?)",
                                (tick, current["generation"], candidate, cut, current["runtime_root"], receipt_hash))
                self.db.execute("""UPDATE world SET accepted_generation=?,tick=?,runtime_root=?,cut_root=?,state=?
                                   WHERE singleton=1""",
                                (current["generation"], tick, candidate, cut, encoded_state))
            if before_commit:
                before_commit()  # isolated crash injection only
        # Only after a successful durable COMMIT. The next read still
        # fetches the exact SQLite BLOB and rechecks all ledger metadata.
        # Reuse only the immutable locally validated candidate and roots.
        if self.reuse_validated_candidate:
            self._remember_state(encoded_state, state, ids, rows)
        return {"generation": current["generation"], "tick": tick, "runtime_root": candidate,
                "cut_root": cut, "receipt_hash": receipt_hash}

    def history(self):
        with self.transaction():
            self._read()
            return {"commits": [list(row) for row in self.db.execute(
                "SELECT * FROM commits ORDER BY tick")],
                "epochs": [[g, t, r, json.loads(s), bool(c)] for g, t, r, s, c in self.db.execute(
                "SELECT * FROM epochs ORDER BY generation")]}


def bind_channels(channels, grants):
    require(len(channels) == len(grants), "grant/channel count mismatch")
    deadline = time.monotonic() + 3
    for channel, grant in zip(channels, grants):
        channel.send({"op": "bind_grant", "grant": grant}, deadline)
    for channel, grant in zip(channels, grants):
        require(channel.recv(deadline) == {"op": "bound", "session": grant["session"],
                                          "generation": grant["generation"]}, "grant bind refused")


def candidate_receipts(coordinator):
    require(not coordinator.poisoned, "frozen coordinator")
    return coordinator.exchange([{"op": "fenced_snapshot"} for _ in coordinator.channels],
                                time.monotonic() + coordinator.timeout)


class FencedRuntime:
    """Adapter from real region workers to a single durable publication boundary.

    A successful in-memory Coordinator.step is only tentative. The local
    ledger alone returns a canonical accepted cut; a failed commit poisons
    this adapter and its worker coordinator. Never restart with old grants.
    """
    def __init__(self, authority, channels, *, worker_exchange="fused"):
        require(worker_exchange in ("fused", "separate"), "unknown worker exchange")
        self.authority = authority
        self.worker_exchange = worker_exchange
        current = authority.read()
        # Even a crash before tick one consumes the generation.
        current = authority.claim(current["generation"], current["tick"], current["runtime_root"])
        self.generation = current["generation"]
        self.session_ids = [g["session"] for g in current["grants"]]
        bind_channels(channels, current["grants"])
        placement = {r: slot for slot, g in enumerate(current["grants"]) for r in g["regions"]}
        self.coordinator = Coordinator(channels, current["state"], placement=placement,
                                       start_tick=current["tick"])
        self.frozen = False

    def advance(self, *, controls=None, fault=None, before_publish=None):
        require(not self.frozen, "fenced runtime frozen")
        try:
            with self.authority.phase("runtime_preflight"):
                current = self.authority.read()
            require(current["generation"] == self.generation and
                    [g["session"] for g in current["grants"]] == self.session_ids and
                    (current["tick"], current["runtime_root"]) ==
                    (self.coordinator.tick, self.coordinator.root), "runtime grant or cut retired")
            with self.authority.phase("worker_tick"):
                candidate = self.coordinator.step(fault, controls=controls,
                                                  commit_snapshot=self.worker_exchange == "fused",
                                                  receipt_phase=self.authority.phase)
            if self.worker_exchange == "fused":
                receipts = candidate["receipts"]
            else:
                with self.authority.phase("worker_receipts"):
                    receipts = candidate_receipts(self.coordinator)
            require(len(receipts) == len(self.session_ids) and all(
                        isinstance(r, dict) and isinstance(r.get("body"), dict)
                        and integer(r["body"].get("tick"), candidate["tick"], candidate["tick"])
                        and r["body"].get("runtime_root") == candidate["root"]
                        and r["body"].get("session") == session
                        for r, session in zip(receipts, self.session_ids)),
                    "worker snapshots differ from staged candidate")
            if before_publish is not None:
                with self.authority.phase("resource_guard"):
                    before_publish()
            with self.authority.phase("publication_receipt_validation"):
                accepted = self.authority.commit(receipts)
            return accepted
        except BaseException:
            self.frozen = self.coordinator.poisoned = True
            raise
