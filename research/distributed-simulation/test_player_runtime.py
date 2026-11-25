import copy
import json
from pathlib import Path
import unittest

from player_runtime import PlayerWorld, cut_digest, fixture
from region_runtime import Coordinator, Rejected, Worker, WIDTH
from test_region_runtime import MemoryChannel
from verify_player_evidence import verify


class PlayerTests(unittest.TestCase):
    def setUp(self):
        self.channels = [MemoryChannel(), MemoryChannel()]
        self.world = PlayerWorld(Coordinator(self.channels, fixture()), "a" * 64, (0, 1, 2))

    def move(self, **changes):
        return {"op": "move", "world": self.world.world, "tick": self.world.tick + 1,
                "direction": 1, **changes}

    def test_continuity_and_idle(self):
        self.world.submit(0, self.move())
        result = self.world.advance()
        self.assertEqual(self.world.view(0)["position"], 4 * WIDTH)
        self.assertEqual(result["directions"], [[0, 1], [1, 0], [2, 0]])
        self.assertEqual(self.world.view(1)["position"], 8 * WIDTH - 1)
        self.world.advance()
        self.assertEqual(self.world.view(0)["position"], 4 * WIDTH)

    def test_invalid_input_does_not_mutate(self):
        before = self.world.root
        for changes in ({"world": "b" * 64}, {"tick": 0}, {"tick": 2}, {"direction": True},
                        {"direction": 2}, {"player": 1}):
            with self.assertRaises(Rejected):
                self.world.submit(0, self.move(**changes))
        with self.assertRaises(Rejected):
            self.world.submit(3, self.move())
        self.assertEqual(self.world.pending, {})
        self.assertEqual(self.world.root, before)
        self.world.submit(0, self.move())
        with self.assertRaises(Rejected):
            self.world.submit(0, self.move(direction=-1))
        self.assertEqual(self.world.pending, {0: 1})

    def test_join_protocol_and_world(self):
        for world, protocol in (("b" * 64, 1), ("a" * 64, 2), ("a" * 64, True)):
            with self.assertRaises(Rejected):
                self.world.join(0, {"op": "join", "world": world, "protocol": protocol})
        self.assertEqual(self.world.join(0, {"op": "join", "world": "a" * 64, "protocol": 1}),
                         self.world.view(0))

    def test_world_and_roster_bound_to_cut(self):
        root = self.world.coordinator.root
        self.assertNotEqual(cut_digest("a" * 64, 0, root, (0, 1)),
                            cut_digest("b" * 64, 0, root, (0, 1)))
        self.assertNotEqual(cut_digest("a" * 64, 0, root, (0, 1)),
                            cut_digest("a" * 64, 0, root, (0, 2)))

    def test_worker_failure_freezes_publication(self):
        root = self.world.root
        def fault(phase):
            if phase == "sealed":
                raise RuntimeError("test fault")
        with self.assertRaises(RuntimeError):
            self.world.advance(fault)
        self.assertEqual(self.world.root, root)
        self.assertEqual(self.world.tick, 0)
        for op in (lambda: self.world.view(0), self.world.advance,
                   lambda: self.world.submit(0, self.move())):
            with self.assertRaises(Rejected):
                op()

    def test_invalid_snapshot_freezes_publication(self):
        root = self.world.root
        def mutate(reply):
            if reply["op"] == "snapshot":
                reply["tick"] = 0
            return reply
        self.channels[1].mutate = mutate
        with self.assertRaises(Rejected):
            self.world.advance()
        self.assertEqual((self.world.tick, self.world.root), (0, root))
        self.assertTrue(self.world.coordinator.poisoned)

    def test_wrong_region_input_rejected_before_prepare(self):
        worker = Worker()
        state = fixture()
        worker.handle({"op": "init", "version": 1, "regions": [state[3].wire(), state[2].wire()]})
        base = {"op": "prepare_controlled", "tick": 1, "halo": [[2, 0, 0], [3, 0, 0]]}
        for controls in ([[2, [[0, 1]]], [3, []]], [[3, [[0, 1], [0, 0]]], [2, []]],
                         [[3, [[0, True]]], [2, []]], [[3, [[0, 1]]]]):
            with self.assertRaises(Rejected):
                worker.handle({**base, "controls": controls})
            self.assertEqual(worker.phase, "idle")
            self.assertIsNone(worker.pending)
        worker.handle({**base, "controls": [[3, [[0, 1]]], [2, []]]})
        self.assertEqual(worker.phase, "prepared")


class PlayerEvidenceTests(unittest.TestCase):
    def test_retained_receipts_and_alterations(self):
        evidence = Path(__file__).parent / "evidence"
        for suffix in ("1", "2", "faults"):
            data = json.loads((evidence / f"2026-11-26-player-{suffix}.json").read_text())
            verify(data)
            if suffix == "faults":
                mutations = [
                    lambda d: d["faults"][0].update(last_tick=2),
                    lambda d: d["faults"][0]["client_receipt"]["views"].append(
                        d["faults"][0]["client_receipt"]["views"][-1]),
                    lambda d: d["faults"][1].update(coordinator_poisoned=False),
                ]
            else:
                mutations = [
                    lambda d: d["ticks"][0]["views"][0].update(inventory=999),
                    lambda d: d["ticks"][0]["views"][0]["cells"][0].__setitem__(1, -1),
                    lambda d: d["client_receipts"][0]["views"][1].update(player=2),
                    lambda d: d["ticks"][0].update(root="0" * 64),
                    lambda d: d["ticks"][0].update(loop_ms=-1),
                    lambda d: d["client_failures"][0].update(tick=2),
                ]
            for index, mutate in enumerate(mutations):
                with self.subTest(receipt=suffix, mutation=index):
                    bad = copy.deepcopy(data)
                    mutate(bad)
                    with self.assertRaises(Rejected):
                        verify(bad)


if __name__ == "__main__":
    unittest.main()
