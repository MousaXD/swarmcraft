"""Real private loopback TCP/subprocess coverage of the fused commit receipt."""
from contextlib import contextmanager
import json
from pathlib import Path
import secrets
import socket
import subprocess
import sys
import tempfile
import unittest

from player_runtime import fixture
from region_runtime import Channel, Rejected, serial_step
from regional_authority import Authority, FencedRuntime

GROUPS = [list(range(4)), list(range(4, 8))]

# Test-only processes receive ephemeral channel keys on stdin, connect to
# an explicit loopback endpoint, and never open the authority database.
WORKER = """
import json, socket, sys, time
from region_runtime import Channel
from regional_authority import FencedWorker
config = json.loads(sys.stdin.readline())
worker = FencedWorker()
with socket.create_connection(("127.0.0.1", config["port"]), timeout=3) as sock:
    channel = Channel(sock, bytes.fromhex(config["key"]), coordinator=False)
    while True:
        try:
            request = channel.recv(time.monotonic()+5)
        except EOFError:
            break
        reply = worker.handle(request)
        if config["lose"] and request["op"] == "commit_snapshot":
            # The commit really happened in this process, but no reply
            # can reach the coordinator. Never manufacture an ack.
            print(json.dumps({"tick":worker.worker.tick,
                              "phase":worker.worker.phase,
                              "root":worker.current_root}), flush=True)
            break
        channel.send(reply, time.monotonic()+3)
"""


@contextmanager
def workers(*, lose=False):
    channels, processes = [], []
    try:
        for slot in range(2):
            with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as listener:
                listener.bind(("127.0.0.1", 0))
                listener.listen(1)
                listener.settimeout(5)
                key = secrets.token_bytes(32)
                process = subprocess.Popen(
                    [sys.executable, "-c", WORKER], cwd=Path(__file__).parent,
                    stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                    text=True)
                processes.append(process)
                process.stdin.write(json.dumps({"port":listener.getsockname()[1],
                                                "key":key.hex(),"lose":lose and slot==1})+"\n")
                process.stdin.close()
                sock, address = listener.accept()
                if address[0] != "127.0.0.1":
                    sock.close()
                    raise RuntimeError("unexpected test peer")
                channels.append(Channel(sock, key, coordinator=True))
        yield channels, processes
    finally:
        for channel in channels:
            channel.close()
        for process in processes:
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)
            if not process.stdin.closed:
                process.stdin.close()
            process.stdout.close()
            process.stderr.close()


class FusedTCPTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.ledger = Authority.create(Path(self.tmp.name)/"ledger", "a"*64,
                                       fixture(), [0,1,2], GROUPS)
        self.addCleanup(self.ledger.close)

    def test_exact_authenticated_frame_count_and_state_in_both_modes(self):
        for mode, rounds in (("separate",4),("fused",3)):
            current = self.ledger.read()
            if current["claimed"]:
                self.ledger.rotate(current["generation"],current["tick"],current["runtime_root"],GROUPS)
                current = self.ledger.read()
            with workers() as (channels,processes):
                runtime = FencedRuntime(self.ledger,channels,worker_exchange=mode)
                before = [(c.tx_seq,c.rx_seq) for c in channels]
                accepted = runtime.advance()
                self.assertEqual([(c.tx_seq-a,c.rx_seq-b) for c,(a,b) in zip(channels,before)],
                                 [(rounds,rounds)]*2)
                self.assertEqual(accepted["tick"],current["tick"]+1)
                self.assertEqual(self.ledger.read()["state"],serial_step(current["state"],accepted["tick"]))
                for channel in channels:
                    channel.close()
                for process in processes:
                    self.assertEqual(process.wait(timeout=5),0,process.stderr.read())

    def test_process_exits_after_local_commit_without_reply(self):
        with workers(lose=True) as (channels,processes):
            runtime = FencedRuntime(self.ledger,channels)
            before = self.ledger.save()
            history = self.ledger.history()
            with self.assertRaises(EOFError):
                runtime.advance()
            self.assertEqual(processes[1].wait(timeout=5),0,processes[1].stderr.read())
            proof = json.loads(processes[1].stdout.read())
            self.assertEqual(proof["tick"],1)
            self.assertEqual(proof["phase"],"idle")
            self.assertNotEqual(proof["root"],before["runtime_root"])
            self.assertTrue(runtime.frozen and runtime.coordinator.poisoned)
            self.assertEqual(self.ledger.save(),before)
            self.assertEqual(self.ledger.history(),history)
            with self.assertRaises(Rejected):
                runtime.advance()
            with self.assertRaises(Rejected):
                FencedRuntime(self.ledger,[])
        self.ledger.rotate(before["generation"],before["tick"],before["runtime_root"],GROUPS)
        with workers() as (channels,_):
            runtime = FencedRuntime(self.ledger,channels)
            self.assertEqual(runtime.advance()["tick"],1)
            self.assertEqual(self.ledger.read()["state"],serial_step(fixture(),1))


if __name__=="__main__":
    unittest.main()
