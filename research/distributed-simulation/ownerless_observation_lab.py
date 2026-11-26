#!/usr/bin/env python3
"""Real private TCP observation across network namespaces; not a live lease."""
import argparse
import json
import os
from pathlib import Path
import select
import socket
import subprocess
import tempfile
import time

from combined_ownerless_lab import Quorum, artifact_manifest, sha
from network_lab import command, safe_namespace
from region_runtime import require

CLASSIFICATION = "LOCAL_OWNERLESS_SIGNED_TCP_OBSERVATIONS_NOT_LEASE_NOT_MULTI_HOST"
FLAGS = ("freshness_lease", "latest_global_head_proven", "live_quorum_verified",
         "write_authority", "canonical_authority", "runtime_activation", "player_admission", "worker_admission")
INTERFACE = "sc69c"
PEER_INTERFACE = "sc69w"


def line(process):
    readable, _, _ = select.select([process.stdout], [], [], 10)
    require(readable, "observation listener readiness timed out")
    result = process.stdout.readline()
    require(result.endswith(b"\n"), "observation listener did not start")
    return result


class PrivateListener:
    """Create exactly one veth pair inside a fresh mapped user/net namespace."""
    def __init__(self, quorum, slot):
        self.quorum, self.slot = quorum, slot
        self.process = None
        self.created = False

    def __enter__(self):
        q = self.quorum
        require({name for _, name in socket.if_nameindex()} == {"lo"}, "nonempty lab namespace")
        args = [q.cli, "--data-dir", str(q.profiles[self.slot]), "ownerless", "authority",
                "observe", "serve", *q.context(), "--bind", "10.69.0.2:0", "--confirm"]
        try:
            self.parent_namespace = os.readlink("/proc/self/ns/net")
            self.process = subprocess.Popen(
                ["unshare", "--net", "sh", "-ec",
                 'printf "namespace-ready\\n"; read -r token; test "$token" = go; exec "$@"',
                 "sh", *args], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            require(line(self.process) == b"namespace-ready\n", "namespace startup failed")
            pid = str(self.process.pid)
            self.namespace = os.readlink(f"/proc/{pid}/ns/net")
            require(self.namespace != self.parent_namespace, "listener reused collector network namespace")
            command("ip", "link", "add", INTERFACE, "type", "veth", "peer", "name", PEER_INTERFACE)
            self.created = True
            command("ip", "link", "set", PEER_INTERFACE, "netns", pid)
            command("ip", "addr", "add", "10.69.0.1/30", "dev", INTERFACE)
            command("ip", "link", "set", INTERFACE, "up")
            prefix = ("nsenter", "-t", pid, "-n")
            command(*prefix, "ip", "addr", "add", "10.69.0.2/30", "dev", PEER_INTERFACE)
            command(*prefix, "ip", "link", "set", PEER_INTERFACE, "up")
            command(*prefix, "ip", "link", "set", "lo", "up")
            require(not json.loads(command(*prefix, "ip", "-j", "route", "show", "default", capture=True)),
                    "listener has an external route")
            self.process.stdin.write(b"go\n")
            self.process.stdin.close()
            self.ready = json.loads(line(self.process))
            require(self.ready["address"].startswith("10.69.0.2:") and
                    self.ready["head"] == q.head["head"] and self.ready["runtime_activation"] is False,
                    "wrong listener readiness")
            self.before = self.counters()
            return self
        except BaseException:
            self.close()
            raise

    def counters(self):
        row, = json.loads(command("ip", "-s", "-j", "link", "show", "dev", INTERFACE, capture=True))
        return row["stats64"]

    def partition(self):
        command("ip", "link", "set", INTERFACE, "down")
        row, = json.loads(command("ip", "-j", "link", "show", "dev", INTERFACE, capture=True))
        require("UP" not in row["flags"], "link partition not applied")

    def restore(self):
        command("ip", "link", "set", INTERFACE, "up")
        row, = json.loads(command("ip", "-j", "link", "show", "dev", INTERFACE, capture=True))
        require("UP" in row["flags"], "link restoration failed")

    def collect(self, slot, reject=None):
        q = self.quorum
        key = next(v["peer_id"] for v in q.head["validators"] if v["public_key"] == q.keys[self.slot])
        require(key == self.ready["validator"], "listener identity differs from selected roster")
        return q.auth(slot, "observe", "collect", *q.context(), "--peer",
                      key+"@"+self.ready["address"], "--confirm", reject=reject)

    def finish(self, result):
        # Counters must be sampled while the namespace still exists. The
        # one-shot responder may exit promptly, so the collector's signed
        # response is the primary evidence of private TCP traversal.
        require(self.process.wait(timeout=5) == 0, "listener failed: "+
                self.process.stderr.read(2048).decode(errors="replace"))
        q = self.quorum
        require(result["classification"] == "observed_recent_ownerless_reference_majority"
                and result["head"] == q.head["head"]
                and result["checkpoint"] == q.head["checkpoint"]
                and result["observation_count"] == result["majority_threshold"] == 2
                and result["unavailable_or_invalid_peers"] == []
                and result["collection_elapsed_ms"] < result["collection_window_ms"]
                and all(result[k] is False for k in FLAGS),
                "network observation improperly granted authority or differed from accepted head")
        return {"listener_pid": self.process.pid, "collector_namespace": self.parent_namespace,
                "listener_namespace": self.namespace, "listener_address": self.ready["address"],
                "listener_exitcode": self.process.returncode, "observation": result}

    def close(self):
        if self.process is not None:
            if self.process.poll() is None:
                self.process.kill()
                self.process.wait(timeout=5)
            for stream in (self.process.stdin, self.process.stdout, self.process.stderr):
                if not stream.closed:
                    stream.close()
        if self.created and INTERFACE in {name for _, name in socket.if_nameindex()}:
            try:
                command("ip", "link", "del", INTERFACE)
            except subprocess.CalledProcessError:
                require(INTERFACE not in {name for _, name in socket.if_nameindex()},
                        "lab interface cleanup failed")
        require({name for _, name in socket.if_nameindex()} == {"lo"}, "lab network cleanup incomplete")

    def __exit__(self, *_):
        self.close()


def run(cli, directory):
    manifest = artifact_manifest(cli)
    manifest[Path(__file__).name] = sha(Path(__file__).read_bytes())
    q = Quorum(cli, directory, manifest)
    q.depart()
    q.certify("founder_independent_network_observation_recovery", ["--recover"])
    before = [q.profiles[i]/"ownerless-authority-lab-v1"/q.world.split(":")[1]/"journal.json" for i in (1,2)]
    digests = [sha(p.read_bytes()) for p in before]
    with PrivateListener(q,2) as link:
        first = link.finish(link.collect(1))
    print("surviving majority observed across distinct private network namespaces",flush=True)
    with PrivateListener(q,2) as link:
        link.partition()
        started = time.monotonic_ns()
        refusal = link.collect(1,reject="missing observation majority")
        elapsed = (time.monotonic_ns()-started)/1e6
        require(link.process.poll() is None, "partition unexpectedly reached listener")
        link.restore()
        second = link.finish(link.collect(1))
    require(first["observation"]["request"]["context"]["nonce"] != second["observation"]["request"]["context"]["nonce"],
            "observation nonce reused after reconnect")
    require([sha(p.read_bytes()) for p in before] == digests, "observations mutated validator journals")
    require(sha(q.founder_path.read_bytes()) == q.founder_digest, "departed founder journal changed")
    for i in q.active:
        q.check_head(q.auth(i,"status","--expected-world",q.world))
    print("partition refused; restored private link recovered a fresh observation",flush=True)
    return {"schema":1,"classification":CLASSIFICATION,"physical_machines":1,"world_id":q.world,
            "artifact_manifest":manifest,"history":json.loads(q.history.read_bytes()),
            "certified_head":q.head,"accepted_checkpoint":json.loads(q.checkpoint.read_bytes()),
            "first":first,"partition":{"link_down":True,"refusal":refusal,"elapsed_ms":elapsed},
            "restored":second,"founder_journal_unchanged":True,"validator_journals_unchanged":True,
            "all_listeners_exited":True,"external_routes":False}


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--cli",type=Path,required=True)
    p.add_argument("--output",type=Path,required=True)
    args = p.parse_args()
    require(not args.output.exists(), "new receipt path required")
    os.umask(0o077)
    safe_namespace()
    with tempfile.TemporaryDirectory(prefix="sc69-observation-") as directory:
        report = run(args.cli.resolve(),Path(directory))
    with args.output.open("x") as out:
        json.dump(report,out,separators=(",",":"),sort_keys=True,allow_nan=False)
        out.write("\n")
    print(json.dumps({"observations":2,"partition_refused":True,"physical_machines":1}))


if __name__ == "__main__":
    main()
