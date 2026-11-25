#!/usr/bin/env python3
"""Real localhost infrastructure/publisher processes; no WAN/NAT certification."""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[2]
WORLD = re.compile(r"scworld:[a-f0-9]{64}")


def isolated_env(**extra):
    env = os.environ.copy()
    for key in ("SWARMCRAFT_BOOTSTRAP", "SWARMCRAFT_RELAY", "SWARMCRAFT_DATA_DIR"):
        env.pop(key, None)
    env.update(SWARMCRAFT_DISCOVERY_MDNS="0", RUST_LOG="off", **extra)
    return env


def command(args, env=None, timeout=35, expect=0):
    result = subprocess.run([str(a) for a in args], capture_output=True, text=True,
                            env=env or isolated_env(), timeout=timeout)
    if (result.returncode == 0) != (expect == 0):
        raise AssertionError(f"unexpected command exit {result.returncode}: {result.stderr[-600:]}")
    return result.stdout


class Service:
    def __init__(self, args, log, env=None):
        self.log = log
        self.file = log.open("wb")
        self.process = subprocess.Popen([str(a) for a in args], stdout=self.file,
                                        stderr=subprocess.STDOUT, env=env or isolated_env())

    def address(self, marker):
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            output = self.log.read_text(errors="replace")
            for line in output.splitlines():
                if line.startswith(marker):
                    return line.removeprefix(marker).strip()
            if self.process.poll() is not None:
                raise AssertionError(f"service exited before readiness: {output[-600:]}")
            time.sleep(0.05)
        raise AssertionError(f"service did not bind: {self.log.read_text(errors='replace')[-600:]}")

    def close(self):
        try:
            if self.process.poll() is None:
                self.process.terminate()
                try:
                    self.process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    self.process.kill()
                    self.process.wait(timeout=5)
        finally:
            self.file.close()


def smoke(cli, runtime, infra):
    with tempfile.TemporaryDirectory(prefix="swarmcraft-private-infra-") as folder:
        root = Path(folder)
        services = []
        try:
            locator_state = root / "locator"
            locator_args = [infra, "--state-dir", locator_state, "locator",
                            "--listen", "/ip4/127.0.0.1/tcp/0"]
            locator = Service(locator_args, root / "locator.log")
            services.append(locator)
            locator_address = locator.address("Locator address: ")
            locator_id = locator_address.rsplit("/p2p/", 1)[1]
            command(locator_args, expect=1)
            # A locator with no publisher cannot fabricate a signed world.
            client = root / "client"
            unknown = "scworld:" + "1" * 64
            result = json.loads(command([runtime, "--data-dir", client, "discovery-resolve",
                                         unknown, "--bootstrap", locator_address]))
            assert result["state"] != "found" and result["world"] is None
            owner = root / "owner"
            worlds = {}
            for visibility in ("public", "private"):
                output = command([cli, "--data-dir", owner, "world", "create", "--name",
                                  f"Infrastructure test {visibility}", "--minecraft", "26.1.2",
                                  "--fabric-loader", "0.19.3", "--visibility", visibility])
                worlds[visibility] = WORLD.search(output).group(0)
            publisher_args = [runtime, "--data-dir", owner, "discovery-serve",
                              "--listen", "/ip4/127.0.0.1/tcp/0"]
            publisher_env = isolated_env(SWARMCRAFT_BOOTSTRAP=locator_address)
            publisher = Service(publisher_args, root / "publisher.log", publisher_env)
            services.append(publisher)
            provider_address = publisher.address("Discovery provider address: ")
            assert provider_address != locator_address
            command(publisher_args, env=publisher_env, expect=1)
            # Both endpoints disable mDNS. The lookup knows ONLY the locator,
            # not the provider's transport address. It still verifies signed
            # canonical announcement, membership and freshness end-to-end.
            deadline = time.monotonic() + 55
            while True:
                result = json.loads(command([runtime, "--data-dir", client, "discovery-resolve",
                                             worlds["public"], "--bootstrap", locator_address]))
                if result.get("state") == "found":
                    break
                if time.monotonic() >= deadline:
                    raise AssertionError(f"locator did not find actual signed publisher: {result['state']}")
                time.sleep(0.3)
            assert result["world"]["world_id"] == worlds["public"]
            assert result["world"]["visibility"] == "public"
            browse = json.loads(command([runtime, "--data-dir", client, "discovery-search",
                                         "--query", "Infrastructure test", "--bootstrap", locator_address]))
            assert any(item["world_id"] == worlds["public"] for item in browse["results"]), browse
            assert all(item["world_id"] != worlds["private"] for item in browse["results"]), browse
            private = json.loads(command([runtime, "--data-dir", client, "discovery-resolve",
                                          worlds["private"], "--bootstrap", locator_address]))
            assert private["state"] != "found" and private["world"] is None
            assert not (locator_state / "worlds").exists(), "locator must never open a world store"

            # Start and restart the actual relay binary with an explicit
            # transport allowlist. Transport bytes/failure/replacement are
            # independently exercised by the Rust production relay test.
            relay_state = root / "relay"
            config = root / "relay.json"
            config.write_text(json.dumps({
                "listen": "/ip4/127.0.0.1/tcp/0", "advertise": None,
                "allowed_transport_peers": [locator_id],
                "max_circuit_bytes": 8388608, "max_circuit_seconds": 300,
                "max_circuits": 4,
            }))
            relay_args = [infra, "--state-dir", relay_state, "relay", "--config", config]
            relay = Service(relay_args, root / "relay.log")
            services.append(relay)
            relay_id = relay.address("Relay address: ").rsplit("/p2p/", 1)[1]
            command(relay_args, expect=1)
            relay.close()
            services.remove(relay)
            restarted = Service(relay_args, root / "relay-restarted.log")
            services.append(restarted)
            assert restarted.address("Relay address: ").rsplit("/p2p/", 1)[1] == relay_id
            assert not (relay_state / "worlds").exists()
            assert (locator_state / "role-v1").read_text().strip() == "locator"
            assert (relay_state / "role-v1").read_text().strip() == "relay"
            for service in services:
                content = service.log.read_text(errors="replace")
                assert "scinvite:" not in content and worlds["private"] not in content
            print("PASS: isolated locator -> signed publisher lookup, private world excluded, duplicate owners refused, stable relay restart.")
            print("Scope: real local services with mDNS disabled; no independent-network or actual Minecraft certification.")
        finally:
            for service in reversed(services):
                service.close()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", type=Path, default=ROOT / "target/debug")
    args = parser.parse_args()
    smoke(*(args.bin_dir.resolve() / name for name in ("swarmcraft", "swarmcraft-runtime", "swarmcraft-infra")))
