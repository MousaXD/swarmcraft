"""Player-controlled continuous reference world. Research only, NOT Minecraft.

A single ephemeral trusted coordinator routes inputs to currently resident
regions and publishes views only from a fully acknowledged, checked cut.
No production identity, durable recovery, or independent authority.
"""
from __future__ import annotations

import hashlib
from region_runtime import (CELLS, MAX_AGENTS, MAX_TICK, REGIONS, WIDTH,
                            canonical, digest, fields, integer, require)

PROTOCOL = 1
RULESET = "swarmcraft-reference-ring/player-v1"
MAX_PLAYERS = 16
VIEW_RADIUS = 2


def player_index(state, players):
    result = {}
    for rid, region in state.items():
        for agent, pos, inventory in region.agents:
            if agent in players:
                require(agent not in result and pos // WIDTH == rid,
                        "duplicate or misplaced player")
                result[agent] = (pos, inventory)
    require(set(result) == set(players), "missing registered player")
    return result


def cut_digest(world, tick, runtime_root, players):
    require(digest(world) and digest(runtime_root) and integer(tick, 0, MAX_TICK),
            "invalid world cut")
    return hashlib.sha256(b"swarmcraft-reference-player-cut-v1\0" + canonical(
        [RULESET, world, tick, runtime_root, sorted(players)])).hexdigest()


def player_view(world, tick, root, state, players, player):
    require(player in players, "unregistered player")
    pos, inventory = player_index(state, {player})[player]
    cells = []
    for offset in range(-VIEW_RADIUS, VIEW_RADIUS + 1):
        cell = (pos + offset) % CELLS
        region, local = divmod(cell, WIDTH)
        cells.append([cell, state[region].signals[local], state[region].banks[local]])
    return {"op": "view", "protocol": PROTOCOL, "ruleset": RULESET,
            "world": world, "tick": tick, "root": root, "player": player,
            "position": pos, "region": pos // WIDTH, "inventory": inventory,
            "cells": cells}


class PlayerWorld:
    def __init__(self, coordinator, world, players):
        require(digest(world), "invalid world identity")
        players = tuple(players)
        require(1 <= len(players) <= MAX_PLAYERS and
                all(integer(p, 0, MAX_AGENTS - 1) for p in players)
                and len(set(players)) == len(players), "invalid player roster")
        require(coordinator.tick == 0 and not coordinator.poisoned,
                "player lab requires a fresh session")
        self.coordinator = coordinator
        self.world = world
        self.players = frozenset(players)
        self.state, _ = coordinator.snapshot()
        self.positions = player_index(self.state, self.players)
        self.tick = 0
        self.root = cut_digest(world, self.tick, coordinator.root, self.players)
        self.pending = {}
        self.frozen = False

    def _live(self):
        require(not self.frozen and not self.coordinator.poisoned, "player world frozen")

    def join(self, player, request):
        self._live()
        fields(request, "op", "world", "protocol")
        require(request["op"] == "join" and request["world"] == self.world
                and integer(request["protocol"], PROTOCOL, PROTOCOL), "incompatible world or protocol")
        return self.view(player)

    def submit(self, player, request):
        """The authenticated connection supplies player identity, never the message."""
        self._live()
        fields(request, "op", "world", "tick", "direction")
        require(type(player) is int and player in self.players and player not in self.pending,
                "unknown or duplicate player input")
        require(request["op"] == "move" and request["world"] == self.world
                and integer(request["tick"], self.tick + 1, self.tick + 1)
                and integer(request["direction"], -1, 1), "invalid player input")
        self.pending[player] = request["direction"]

    def view(self, player):
        self._live()
        require(type(player) is int and player in self.players, "unregistered player")
        return player_view(self.world, self.tick, self.root, self.state, self.players, player)

    def advance(self, fault=None):
        self._live()
        # Missing input holds position; autonomous background agents still tick.
        directions = {p: self.pending.get(p, 0) for p in sorted(self.players)}
        controls = {}
        expected = {}
        for player, direction in directions.items():
            pos, _ = self.positions[player]
            controls.setdefault(pos // WIDTH, []).append([player, direction])
            expected[player] = (pos + direction) % CELLS
        try:
            result = self.coordinator.step(fault, controls=controls)
            state, _ = self.coordinator.snapshot()
            positions = player_index(state, self.players)
            require(all(positions[p][0] == expected[p] for p in self.players),
                    "accepted player position differs from input")
            tick = self.coordinator.tick
            root = cut_digest(self.world, tick, self.coordinator.root, self.players)
            # Do not publish even a partial or stale view after a worker fault.
            self.state, self.positions, self.tick, self.root = state, positions, tick, root
            self.pending.clear()
            return {**result, "runtime_root": result["root"], "root": root,
                    "directions": [[p, d] for p, d in directions.items()]}
        except BaseException:
            self.frozen = self.coordinator.poisoned = True
            self.pending.clear()
            raise


def fixture():
    """Fixed world: two moving players plus a third negative-test identity."""
    from region_runtime import Region, initial_world
    state = initial_world("spread", 256, seed=59)
    positions = {0: CELLS // 2 - 1, 1: CELLS - 1, 2: WIDTH}
    for rid, region in list(state.items()):
        agents = [a for a in region.agents if a[0] not in positions]
        agents.extend((p, pos, 5) for p, pos in positions.items() if pos // WIDTH == rid)
        state[rid] = Region(rid, region.signals, region.banks, tuple(sorted(agents)))
    return state


def scripted_direction(player, tick):
    """Repeated worker crossings, a reversal, then extended traversal."""
    require(player in (0, 1), "unknown scripted client")
    direction = -1 if 3 <= tick <= 4 else 1
    return direction
