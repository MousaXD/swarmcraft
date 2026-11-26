"""#61 durable reference player commands and atomic all-region transactions.

Research only. The #60 trusted local authority remains the sole publisher.
A separately identified database/protocol cannot open through the unfenced
legacy authority. A journaled command or sealed batch is not a committed cut.
"""
from __future__ import annotations

import hashlib
import json

from player_runtime import player_index, player_view
from region_runtime import (MAX_AGENTS, MAX_TICK, WIDTH, canonical, digest,
                            fields, integer, region_roots, require, serial_step, world_root)
from regional_authority import Authority, FencedRuntime, validated_state, wire

RULESET = "swarmcraft-reference-ring/transaction-v2"
PROTOCOL = 2
COMMAND_FIELDS = ("op", "world", "id", "tick", "parent", "direction", "action")


def checked_command(player, request, world, players):
    fields(request, *COMMAND_FIELDS)
    require(integer(player, 0, MAX_AGENTS - 1) and player in players
            and request["op"] == "act" and request["world"] == world
            and digest(request["id"]) and digest(request["parent"])
            and integer(request["tick"], 1, MAX_TICK)
            and integer(request["direction"], -1, 1)
            and integer(request["action"], -1, 1), "invalid authenticated player command")
    return request


def transition_view(world, tick, cut, state, players, player):
    return {**player_view(world, tick, cut, state, players, player),
            "protocol": PROTOCOL, "ruleset": RULESET}


def response(world, tick, cut, state, players, player, request, previous):
    view = transition_view(world, tick, cut, state, players, player)
    return {"op": "committed", "id": request["id"], "view": view,
            "inventory_delta": view["inventory"] - previous}


def batch_controls(state, batch):
    positions = player_index(state, {row[0] for row in batch})
    result = {}
    for p, direction, action, _ in batch:
        result.setdefault(positions[p][0] // WIDTH, []).append([p, direction, action])
    return result


def serial_controls(batch):
    return {p: (direction, action) for p, direction, action, _ in batch}


class TransitionAuthority(Authority):
    """Distinct schema, atomic command results + accepted world cut."""
    app_id = 0x53433631
    schema = 1
    ruleset = RULESET

    @staticmethod
    def cut(world, tick, root, players):
        require(digest(world) and digest(root) and integer(tick, 0, MAX_TICK), "invalid cut")
        return hashlib.sha256(b"sc61-reference-cut-v2\0" +
                              canonical([RULESET, world, tick, root, sorted(players)])).hexdigest()

    @classmethod
    def _create_extra(cls, db, state):
        db.execute("CREATE TABLE genesis(singleton INTEGER PRIMARY KEY CHECK(singleton=1), state BLOB NOT NULL)")
        db.execute("INSERT INTO genesis VALUES(1,?)", (canonical(wire(state)),))
        db.execute("""CREATE TABLE commands(
            tick INTEGER NOT NULL, player INTEGER NOT NULL, request_id TEXT NOT NULL UNIQUE,
            request BLOB NOT NULL, response BLOB, PRIMARY KEY(tick,player))""")
        db.execute("""CREATE TABLE batches(
            tick INTEGER PRIMARY KEY, parent TEXT NOT NULL, inputs BLOB NOT NULL, result TEXT)""")

    def __init__(self, path):
        super().__init__(path)
        try:
            self.audit()
        except BaseException:
            self.close()
            raise

    def view(self, player):
        with self.transaction():
            cur = self._read()
            require(integer(player, 0, MAX_AGENTS - 1) and player in cur["players"], "unknown player")
            return transition_view(cur["world"], cur["tick"], cur["cut_root"], cur["state"],
                                   cur["players"], player)

    def submit(self, player, request):
        """Authenticated identity is a connection property, not client payload.

        Queued means durably recorded, never applied. Exact replay is stable
        across restart; conflicting use of either ID or player/tick is denied.
        """
        with self.transaction(write=True):
            cur = self._read()
            checked_command(player, request, cur["world"], cur["players"])
            existing = self.db.execute("SELECT player,request,response FROM commands WHERE request_id=?",
                                       (request["id"],)).fetchone()
            if existing is not None:
                require(existing[0] == player and existing[1] == canonical(request),
                        "conflicting command replay")
                if existing[2] is not None:
                    return json.loads(existing[2])
                require(request["tick"] == cur["tick"] + 1 and request["parent"] == cur["cut_root"],
                        "orphaned pending command")
            else:
                require(request["tick"] == cur["tick"] + 1 and request["parent"] == cur["cut_root"],
                        "stale or future command cut")
                require(self.db.execute("SELECT 1 FROM batches WHERE tick=?", (request["tick"],)).fetchone()
                        is None, "tick input already sealed")
                require(self.db.execute("SELECT 1 FROM commands WHERE tick=? AND player=?",
                                        (request["tick"], player)).fetchone() is None,
                        "player already has an input for this tick")
                self.db.execute("INSERT INTO commands VALUES(?,?,?,?,NULL)",
                                (request["tick"], player, request["id"], canonical(request)))
            return {"op": "queued", "id": request["id"], "tick": request["tick"]}

    def _commands(self, current, tick, parent):
        commands = {}
        rows = self.db.execute("SELECT player,request_id,request,response FROM commands WHERE tick=? ORDER BY player",
                               (tick,)).fetchall()
        for player, request_id, raw, result in rows:
            request = checked_command(player, json.loads(raw), current["world"], current["players"])
            require(request["id"] == request_id and request["tick"] == tick and
                    request["parent"] == parent and raw == canonical(request), "invalid command journal")
            commands[player] = (request, result)
        batch = [[p, commands[p][0]["direction"], commands[p][0]["action"], commands[p][0]["id"]]
                 if p in commands else [p, 0, 0, None] for p in current["players"]]
        return commands, batch

    def _before_seal(self, current):
        pass

    def seal_next(self, generation, tick, root):
        with self.transaction(write=True):
            cur = self._read()
            require(cur["claimed"] and cur["generation"] == generation and
                    (cur["tick"], cur["runtime_root"]) == (tick, root) and tick < MAX_TICK,
                    "retired runtime cannot seal input")
            self._before_seal(cur)
            commands, batch = self._commands(cur, tick + 1, cur["cut_root"])
            require(all(result is None for _, result in commands.values()), "pending result already committed")
            existing = self.db.execute("SELECT parent,inputs,result FROM batches WHERE tick=?", (tick + 1,)).fetchone()
            if existing is None:
                self.db.execute("INSERT INTO batches VALUES(?,?,?,NULL)",
                                (tick + 1, cur["cut_root"], canonical(batch)))
            else:
                require(existing == (cur["cut_root"], canonical(batch), None), "sealed input differs")
            return batch_controls(cur["state"], batch)

    def _record_candidate(self, current, state, candidate, cut):
        # Called INSIDE the #60 write transaction after fenced receipt validation.
        tick = current["tick"] + 1
        commands, batch = self._commands(current, tick, current["cut_root"])
        sealed = self.db.execute("SELECT parent,inputs,result FROM batches WHERE tick=?", (tick,)).fetchone()
        require(sealed == (current["cut_root"], canonical(batch), None), "missing or changed sealed input")
        require(all(result is None for _, result in commands.values()), "duplicate command result")
        with self.phase("independent_serial_replay"):
            expected = serial_step(current["state"], tick, controls=serial_controls(batch))
            require(state == expected and candidate == world_root(tick, region_roots(expected)),
                    "candidate violates sealed player actions or reference rules")
        with self.phase("durable_client_receipts"):
            positions = player_index(current["state"], set(current["players"]))
            for p, (request, _) in commands.items():
                result = response(current["world"], tick, cut, state, current["players"], p, request, positions[p][1])
                self.db.execute("UPDATE commands SET response=? WHERE tick=? AND player=?",
                                (canonical(result), tick, p))
            self.db.execute("UPDATE batches SET result=? WHERE tick=?", (cut, tick))

    def _audit(self, cur):
        genesis, = self.db.execute("SELECT state FROM genesis WHERE singleton=1").fetchone()
        state, _ = validated_state(json.loads(genesis))
        genesis_state = wire(state)
        commits = self.db.execute("SELECT tick,runtime_root,cut_root,parent_root FROM commits ORDER BY tick").fetchall()
        rows = self.db.execute("SELECT tick,parent,inputs,result FROM batches ORDER BY tick").fetchall()
        require(len(commits) == cur["tick"] + 1 and len(rows) in (cur["tick"], cur["tick"] + 1),
                "incomplete transition history")
        root = world_root(0, region_roots(state))
        cut = self.cut(cur["world"], 0, root, cur["players"])
        require(commits[0] == (0, root, cut, None), "invalid transition genesis")
        journal = []
        for index, (tick, parent, inputs, result) in enumerate(rows, 1):
            require(tick == index and parent == cut, "broken transition parent chain")
            commands, batch = self._commands(cur, tick, parent)
            require(inputs == canonical(batch), "sealed history differs from commands")
            if tick <= cur["tick"]:
                positions = player_index(state, set(cur["players"]))
                state = serial_step(state, tick, controls=serial_controls(batch))
                previous_root = root
                root = world_root(tick, region_roots(state))
                cut = self.cut(cur["world"], tick, root, cur["players"])
                require(result == cut and commits[tick] == (tick, root, cut, previous_root),
                        "transition replay differs from accepted cut")
                for p, (request, stored) in commands.items():
                    expected = response(cur["world"], tick, cut, state, cur["players"], p, request, positions[p][1])
                    require(stored == canonical(expected), "invalid committed command receipt")
            else:
                require(result is None and all(v[1] is None for v in commands.values()), "unaccepted result visible")
            journal.append({"tick": tick, "parent": parent, "batch": batch, "result": result})
        require(state == cur["state"] and root == cur["runtime_root"] and cut == cur["cut_root"],
                "replayed head mismatch")
        commands, _ = self._commands(cur, cur["tick"] + 1, cur["cut_root"])
        require(all(v[1] is None for v in commands.values()) and
                self.db.execute("SELECT COUNT(*) FROM commands WHERE tick<1 OR tick>?",
                                (cur["tick"] + 1,)).fetchone()[0] == 0, "orphaned command journal")
        require(self.db.execute("SELECT COUNT(*) FROM commands WHERE tick<=? AND response IS NULL",
                                (cur["tick"],)).fetchone()[0] == 0, "missing committed response")
        recorded = [{"player": p, "request": json.loads(q), "response": json.loads(r) if r else None}
                    for p, q, r in self.db.execute("SELECT player,request,response FROM commands ORDER BY tick,player")]
        return {"genesis": json.loads(canonical(genesis_state)), "batches": journal, "commands": recorded}

    def audit(self):
        with self.transaction():
            return self._audit(self._read())


class DurableRuntime:
    def __init__(self, authority, channels):
        require(isinstance(authority, TransitionAuthority), "transition authority required")
        self.authority = authority
        self.runtime = FencedRuntime(authority, channels)

    def seal(self):
        require(not self.runtime.frozen, "fenced runtime frozen")
        c = self.runtime.coordinator
        return self.authority.seal_next(self.runtime.generation, c.tick, c.root)

    def advance(self, *, fault=None, before_publish=None):
        try:
            with self.authority.phase("seal_inputs"):
                controls = self.seal()
            return self.runtime.advance(controls=controls, fault=fault, before_publish=before_publish)
        except BaseException:
            self.runtime.frozen = self.runtime.coordinator.poisoned = True
            raise
