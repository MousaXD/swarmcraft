"""Reference rules, boundary staging and framed-wire negative tests.

The separate network_lab.py harness supplies actual independent-process
and network-namespace evidence; these tests do not claim that evidence.
"""
import copy
import hashlib
import hmac
import socket
import struct
import time
import unittest

from region_runtime import (CELLS, MAX_FRAME, REGIONS, WIDTH, Channel, Coordinator, Region,
                            Rejected, Worker, canonical, check_event, deliver, initial_world,
                            region_roots, serial_step, totals, world_root)


class MemoryChannel:
    """Direct invocation for deterministic invalid-response injection only."""

    def __init__(self):
        self.worker = Worker()
        self.reply = None
        self.mutate = None

    def send(self, request, deadline):
        self.reply = self.worker.handle(copy.deepcopy(request))
        if self.mutate:
            self.reply = self.mutate(self.reply)

    def recv(self, deadline):
        result, self.reply = self.reply, None
        return copy.deepcopy(result)


class RegionRulesTests(unittest.TestCase):
    def test_full_reference_parity_all_partitions_and_shapes(self):
        for shape in ("spread", "clustered", "boundary"):
            initial = initial_world(shape, 512, seed=345)
            expected = initial
            references = []
            for tick in range(1, 9):
                counter = {}
                expected = serial_step(expected, tick, counter)
                references.append((expected, counter))
            for n in (1, 2, 4):
                with self.subTest(shape=shape, workers=n):
                    control = Coordinator([MemoryChannel() for _ in range(n)], initial)
                    for tick, (state, counter) in enumerate(references, 1):
                        actual = control.step()
                        self.assertEqual(actual["root"], world_root(tick, region_roots(state)))
                        self.assertEqual(actual["region_agents"], counter["region_agents"])
                        self.assertEqual(actual["region_resources"], counter["region_resources"])
                        self.assertEqual(totals(control.summaries.values()), totals(region_roots(initial)))
                    self.assertEqual(control.snapshot()[0], expected)

    def test_boundary_fixture_crosses_both_directions_and_ring_wrap(self):
        initial = initial_world("boundary", 256)
        self.assertEqual(len(initial[0].agents), 32)
        first = serial_step(initial, 1)
        by_id = {a[0]: a for r in first.values() for a in r.agents}
        self.assertEqual(by_id[0][1], CELLS - 1)
        self.assertEqual(by_id[1][1], WIDTH)
        second = serial_step(first, 2)
        self.assertEqual({a[0]: a[1] for r in second.values() for a in r.agents},
                         {a[0]: a[1] for r in initial.values() for a in r.agents})
        self.assertEqual(totals(region_roots(initial)), totals(region_roots(second)))

    def test_single_cell_signal_and_resource_across_region_edge(self):
        # Independent reference verifies neighbor influence and a resource
        # debit/credit across the ring. Empty agents make the outcome exact.
        regions = {r: Region(r, (0,) * WIDTH, (0,) * WIDTH, ()) for r in range(REGIONS)}
        last = REGIONS - 1
        regions[last] = Region(last, (0,) * (WIDTH - 1) + (41,),
                               (0,) * (WIDTH - 1) + (5,), ())
        initial = regions.copy()
        control = Coordinator([MemoryChannel() for _ in range(4)], initial)
        tick = control.step()
        state, _ = control.snapshot()
        self.assertEqual(tick["cross_worker_resources"], 1)
        self.assertEqual(state[0].signals[0], 41)
        self.assertEqual(state[last].banks[-1], 4)
        self.assertEqual(state[0].banks[0], 1)
        self.assertEqual(totals(region_roots(state)), (0, 5))
        self.assertEqual(state, serial_step(initial, 1))

    def test_event_bounds_and_wrong_position(self):
        self.assertEqual(check_event(["e", 7, 0, 42, 0, 2]), ("e", 42))
        self.assertEqual(check_event(["e", 0, 7, 42, CELLS - 1, 2]), ("e", 42))
        for event in (
            ["e", 0, 2, 1, 2 * WIDTH, 1],  # skip region
            ["e", 0, 1, 1, WIDTH + 1, 1],  # not at boundary
            ["e", 0, 1, True, WIDTH, 1],  # bool is not a valid integer
            ["e", 0, 1, 1, WIDTH, -1],
            ["t", 0, 7, 1],
            ["t", 0, 1, True],
            ["t", 0, 1, 2],
        ):
            with self.subTest(event=event), self.assertRaises(Rejected):
                check_event(event)

    def test_duplicate_incoming_entity_never_changes_pending_state(self):
        state = {1: Region(1, (0,) * WIDTH, (0,) * WIDTH, ())}
        original = state.copy()
        event = ["e", 0, 1, 4, WIDTH, 3]
        with self.assertRaisesRegex(Rejected, "duplicate"):
            deliver(state, [event, event])
        self.assertEqual(state, original)
        resident = {1: Region(1, (0,) * WIDTH, (0,) * WIDTH, ((4, WIDTH + 1, 3),))}
        with self.assertRaisesRegex(Rejected, "duplicate incoming"):
            deliver(resident, [event])
        self.assertEqual(resident[1].agents, ((4, WIDTH + 1, 3),))


class CoordinatorTests(unittest.TestCase):
    def make(self):
        initial = initial_world("boundary", 128)
        channels = [MemoryChannel(), MemoryChannel()]
        return Coordinator(channels, initial), channels

    def assert_frozen(self, control, before):
        self.assertTrue(control.poisoned)
        self.assertEqual((control.tick, control.root), before)
        with self.assertRaisesRegex(Rejected, "frozen"):
            control.step()
        with self.assertRaisesRegex(Rejected, "frozen"):
            control.snapshot()

    def test_duplicate_boundary_response_refused(self):
        control, channels = self.make()
        before = control.tick, control.root

        def duplicate(reply):
            if reply["op"] == "prepared":
                self.assertTrue(reply["events"])
                reply["events"].append(reply["events"][0])
            return reply

        channels[0].mutate = duplicate
        with self.assertRaisesRegex(Rejected, "duplicate"):
            control.step()
        self.assert_frozen(control, before)

    def test_stale_prepare_response_refused(self):
        control, channels = self.make()
        before = control.tick, control.root

        def stale(reply):
            if reply["op"] == "prepared":
                reply["tick"] = 0
            return reply

        channels[1].mutate = stale
        with self.assertRaisesRegex(Rejected, "stale prepare"):
            control.step()
        self.assert_frozen(control, before)

    def test_wrong_owner_receipt_refused_even_with_valid_local_digest(self):
        control, channels = self.make()
        before = control.tick, control.root

        def wrong_owner(reply):
            if reply["op"] == "sealed":
                reply["rows"][0][0] = 0
                reply["receipt"] = hashlib.sha256(canonical(reply["rows"])).hexdigest()
            return reply

        channels[1].mutate = wrong_owner
        with self.assertRaisesRegex(Rejected, "invalid region receipt"):
            control.step()
        self.assert_frozen(control, before)

    def test_resource_conservation_checked_before_commit(self):
        control, channels = self.make()
        before = control.tick, control.root

        def nonconservation(reply):
            if reply["op"] == "sealed":
                reply["rows"][0][3] += 1
                reply["receipt"] = hashlib.sha256(canonical(reply["rows"])).hexdigest()
            return reply

        channels[0].mutate = nonconservation
        with self.assertRaisesRegex(Rejected, "conservation"):
            control.step()
        self.assert_frozen(control, before)
        self.assertEqual([c.worker.tick for c in channels], [0, 0])

    def test_commit_ack_mismatch_freezes_even_after_worker_commits(self):
        control, channels = self.make()
        before = control.tick, control.root

        def wrong_ack(reply):
            if reply["op"] == "committed":
                reply["root"] = "00" * 32
            return reply

        channels[1].mutate = wrong_ack
        with self.assertRaisesRegex(Rejected, "commit acknowledgment"):
            control.step()
        self.assert_frozen(control, before)
        # This is deliberately NOT a durable/atomic distributed commit.
        # Honest local workers can be ahead of the accepted research root.
        self.assertEqual([c.worker.tick for c in channels], [1, 1])

    def test_worker_cannot_reassign_or_replay_tick_and_seal_is_bound(self):
        worker = Worker()
        original = initial_world("spread", 32)
        req = {"op": "init", "version": 1, "regions": [r.wire() for r in original.values()]}
        worker.handle(req)
        with self.assertRaisesRegex(Rejected, "invalid init"):
            worker.handle(req)
        halo = [[r, original[(r - 1) % REGIONS].signals[-1],
                 original[(r + 1) % REGIONS].signals[0]] for r in range(REGIONS)]
        with self.assertRaisesRegex(Rejected, "stale"):
            worker.handle({"op": "prepare", "tick": 0, "halo": halo})
        with self.assertRaisesRegex(Rejected, "missing halo"):
            worker.handle({"op": "prepare", "tick": 1, "halo": halo[:-1]})
        worker.handle({"op": "prepare", "tick": 1, "halo": halo})
        self.assertEqual(worker.regions, original)
        with self.assertRaisesRegex(Rejected, "overlapping"):
            worker.handle({"op": "prepare", "tick": 1, "halo": halo})
        with self.assertRaisesRegex(Rejected, "snapshot"):
            worker.handle({"op": "snapshot"})
        sealed = worker.handle({"op": "seal", "tick": 1, "events": []})
        self.assertEqual(worker.regions, original)
        with self.assertRaisesRegex(Rejected, "not bound"):
            worker.handle({"op": "commit", "tick": 1, "root": "12" * 32, "receipt": "34" * 32})
        self.assertEqual(worker.tick, 0)
        worker.handle({"op": "commit", "tick": 1, "root": "12" * 32, "receipt": sealed["receipt"]})
        self.assertEqual(worker.regions, serial_step(original, 1))
        with self.assertRaisesRegex(Rejected, "not bound"):
            worker.handle({"op": "commit", "tick": 1, "root": "12" * 32, "receipt": sealed["receipt"]})


class WireTests(unittest.TestCase):
    def setUp(self):
        listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        listener.bind(("127.0.0.1", 0))
        listener.listen(1)
        self.addCleanup(listener.close)
        self.raw = socket.create_connection(listener.getsockname(), timeout=1)
        self.addCleanup(self.raw.close)
        server, _ = listener.accept()
        self.addCleanup(server.close)
        self.key = bytes(range(32))
        self.worker = Channel(server, self.key, coordinator=False)

    def raw_frame(self, payload, seq=0, tag=b"C", key=None):
        seq = struct.pack(">Q", seq)
        auth = hmac.digest(self.key if key is None else key, tag + seq + payload, "sha256")
        body = seq + auth + payload
        self.raw.sendall(struct.pack(">I", len(body)) + body)

    def test_authenticated_roundtrip(self):
        client = Channel(self.raw, self.key, coordinator=True)
        deadline = time.monotonic() + 1
        client.send({"op": "ping"}, deadline)
        self.assertEqual(self.worker.recv(deadline), {"op": "ping"})
        self.worker.send({"op": "pong"}, deadline)
        self.assertEqual(client.recv(deadline), {"op": "pong"})
        self.assertEqual(client.sent, self.worker.received)
        self.assertEqual(client.received, self.worker.sent)

    def test_invalid_mac_refused(self):
        self.raw_frame(b'{"op":"ping"}', key=b"x" * 32)
        with self.assertRaisesRegex(Rejected, "authentication"):
            self.worker.recv(time.monotonic() + 1)
        self.assertEqual(self.worker.rx_seq, 0)

    def test_replay_and_reflection_refused(self):
        self.raw_frame(b'{"op":"ping"}')
        self.assertEqual(self.worker.recv(time.monotonic() + 1), {"op": "ping"})
        self.raw_frame(b'{"op":"ping"}')
        with self.assertRaisesRegex(Rejected, "stale"):
            self.worker.recv(time.monotonic() + 1)

    def test_reflected_direction_refused(self):
        self.raw_frame(b'{"op":"ping"}', tag=b"W")
        with self.assertRaisesRegex(Rejected, "authentication"):
            self.worker.recv(time.monotonic() + 1)

    def test_oversized_frame_rejected_before_payload(self):
        self.raw.sendall(struct.pack(">I", MAX_FRAME + 1))
        with self.assertRaisesRegex(Rejected, "frame size"):
            self.worker.recv(time.monotonic() + 1)

    def test_partial_frame_deadline(self):
        self.raw.sendall(struct.pack(">I", 50) + b"x")
        with self.assertRaises(TimeoutError):
            self.worker.recv(time.monotonic() + 0.02)


if __name__ == "__main__":
    unittest.main()
