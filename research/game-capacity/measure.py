#!/usr/bin/env python3
"""Bounded, loopback-only Minecraft capacity baseline; Linux /proc samples."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import queue
import re
import socket
import subprocess
import tempfile
import threading
import time

HERE = Path(__file__).resolve().parent

def line_reader(proc, messages, log):
    for line in proc.stdout:
        log.write(line)
        log.flush()
        messages.put(line.strip())

def await_line(proc, messages, predicate, timeout):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if proc.poll() is not None and messages.empty():
            raise RuntimeError(f"process exited {proc.returncode}")
        try:
            line = messages.get(timeout=min(0.5, max(0.01, deadline-time.monotonic())))
        except queue.Empty:
            continue
        result = predicate(line)
        if result is not None:
            return result
    raise TimeoutError("expected process output not observed")

def command(proc, messages, value, regex, timeout=30):
    proc.stdin.write(value + "\n")
    proc.stdin.flush()
    return await_line(proc, messages, lambda line: re.search(regex, line), timeout)

def port_number():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]

def proc_sample(pid):
    base = Path("/proc") / str(pid)
    fields = (base / "stat").read_text().rsplit(")", 1)[1].split()
    status = (base / "status").read_text()
    rss = re.search(r"^VmRSS:\s+(\d+) kB", status, re.M)
    return {"cpu_seconds": (int(fields[11])+int(fields[12]))/os.sysconf("SC_CLK_TCK"),
            "rss_mib": int(rss[1])/1024 if rss else None}

def available_mib():
    return int(re.search(r"^MemAvailable:\s+(\d+) kB", Path("/proc/meminfo").read_text(), re.M)[1])/1024

def disk_bytes(path):
    return sum(p.stat().st_size for p in path.rglob("*") if p.is_file())

def terminate(proc):
    if proc.poll() is not None:
        return
    proc.terminate()
    try:
        proc.wait(10)
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait(10)

def run(args):
    if not re.search(r"^eula\s*=\s*true\s*$", args.accepted_eula.read_text(), re.M):
        raise ValueError("explicit prior Minecraft EULA acceptance is required")
    if args.heap < 1024 or args.heap > 2048 or available_mib() < args.heap + 1024:
        raise ValueError("insufficient local headroom for bounded lab")
    if (not 10 <= args.seconds <= 120 or any(not 0 <= x <= 16 for x in args.players)
            or args.players != sorted(set(args.players))):
        raise ValueError("out of bounds lab settings")
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=False)
    port = port_number()
    with args.jar.open("rb") as jar_file:
        server_hash = hashlib.file_digest(jar_file, "sha256").hexdigest()
    results = {"server_sha256": server_hash, "minecraft_version": "26.1.2",
               "java": subprocess.check_output([str(args.java), "-version"], stderr=subprocess.STDOUT, text=True).splitlines()[0],
               "cpu": next((l.split(":",1)[1].strip() for l in Path("/proc/cpuinfo").read_text().splitlines() if l.startswith("model name")), "unknown"),
               "logical_cpus": os.cpu_count(), "heap_mib": args.heap, "active_processors": 4,
               "view_distance": 6, "simulation_distance": 4, "seed": 42035,
               "initial_available_mib": available_mib(), "requested_players": args.players,
               "requested_stage_seconds": args.seconds, "stages": []}
    def save():
        (out / "measurements.json").write_text(json.dumps(results, indent=2)+"\n")
    save()
    with tempfile.TemporaryDirectory(prefix="swarmcraft-capacity-world-") as tmp, (out/"server.log").open("w") as slog:
        root = Path(tmp)
        (root/"eula.txt").write_text("eula=true\n")
        (root/"server.properties").write_text(
            f"server-ip=127.0.0.1\nserver-port={port}\nonline-mode=false\nenforce-secure-profile=false\n"
            "enable-rcon=false\nenable-query=false\nmax-players=16\nview-distance=6\nsimulation-distance=4\n"
            "level-seed=42035\nlevel-name=world\ndifficulty=peaceful\nspawn-protection=0\n"
            "pause-when-empty-seconds=-1\nallow-flight=false\n")
        server = subprocess.Popen([str(args.java), "-Xms512m", f"-Xmx{args.heap}m",
                                   "-XX:ActiveProcessorCount=4", "-jar", str(args.jar), "nogui"],
                                  cwd=root, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                  stderr=subprocess.STDOUT, text=True, bufsize=1)
        sq = queue.Queue()
        st = threading.Thread(target=line_reader, args=(server,sq,slog), daemon=True)
        st.start()
        try:
            await_line(server,sq,lambda line: True if "Starting minecraft server version 26.1.2" in line else None,180)
            await_line(server,sq,lambda line: "Done (" if "Done (" in line else None,180)
            for n in args.players:
                if available_mib() < 700:
                    results["stop_reason"] = "host available memory below 700 MiB before next stage"
                    break
                bots = None
                bthread = None
                bq = queue.Queue()
                with (out/f"bots-{n}.log").open("w") as blog:
                    try:
                        if n:
                            bots = subprocess.Popen(["node","--max-old-space-size=768", str(HERE/"bots.cjs"),str(n),str(port)],
                                                     stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                                     text=True, bufsize=1)
                            bthread = threading.Thread(target=line_reader,args=(bots,bq,blog),daemon=True)
                            bthread.start()
                            await_line(bots,bq,lambda line: True if '"event":"ready"' in line else None,90)
                        listed = int(command(server,sq,"list",r"There are (\d+) of a max of \d+ players online")[1])
                        if listed != n:
                            raise RuntimeError(f"expected {n} connected players, got {listed}")
                        first = int(command(server,sq,"time query gametime",r"The game time is (\d+)")[1])
                        start = time.monotonic()
                        samples = [proc_sample(server.pid)]
                        client_samples = [proc_sample(bots.pid)] if bots else []
                        available = [available_mib()]
                        tick_lines = []
                        mspt_samples = []
                        while time.monotonic()-start < args.seconds:
                            time.sleep(min(2,max(0,args.seconds-(time.monotonic()-start))))
                            if bots and bots.poll() is not None:
                                raise RuntimeError("bot process exited during measurement")
                            samples.append(proc_sample(server.pid))
                            if bots:
                                client_samples.append(proc_sample(bots.pid))
                            available.append(available_mib())
                            if available[-1] < 500:
                                raise RuntimeError("host available memory fell below 500 MiB")
                            if len(samples) % 3 == 0:
                                match = command(server,sq,"tick query",r"Average time per tick: ([0-9.]+)ms",10)
                                tick_lines.append(match.string)
                                mspt_samples.append(float(match[1]))
                        last = int(command(server,sq,"time query gametime",r"The game time is (\d+)")[1])
                        elapsed = time.monotonic()-start
                        samples.append(proc_sample(server.pid))
                        listed_end = int(command(server,sq,"list",r"There are (\d+) of a max of \d+ players online")[1])
                        if listed_end != n:
                            raise RuntimeError("player count changed during measurement")
                        bot_stats = []
                        traffic = None
                        if bots:
                            bots.stdin.write("stop\n")
                            bots.stdin.flush()
                            summary = await_line(bots,bq,lambda line: json.loads(line) if '"event":"summary"' in line else None,15)
                            bot_stats = summary["stats"]
                            traffic = summary["traffic"]
                            if len(bot_stats)!=n or not all(b["spawned"] for b in bot_stats):
                                raise RuntimeError("incomplete bot observations")
                            if bots.wait(10) != 0:
                                raise RuntimeError("bot process reported a failure")
                        command(server,sq,"save-all flush",r"Saved the game",60)
                        row = {"players":n,"game_ticks":last-first,"seconds":round(elapsed,3),
                               "observed_tps":round((last-first)/elapsed,3),
                               "server_cpu_percent":round(100*(samples[-1]["cpu_seconds"]-samples[0]["cpu_seconds"])/elapsed,1),
                               "server_peak_sampled_rss_mib":round(max(s["rss_mib"] for s in samples),1),
                               "client_peak_sampled_rss_mib":round(max(s["rss_mib"] for s in client_samples),1) if client_samples else None,
                               "host_min_available_mib":round(min(available),1),"world_bytes_after_save":disk_bytes(root/"world"),
                               "tick_query_lines":tick_lines,"mspt_samples":mspt_samples,"client_traffic":traffic,"bots":bot_stats}
                        results["stages"].append(row)
                        save()
                        print(json.dumps(row),flush=True)
                    finally:
                        if bots:
                            terminate(bots)
                        if bthread:
                            bthread.join(5)
            results["complete"] = len(results["stages"]) == len(args.players)
        except Exception as exc:
            results["failure"] = str(exc)
            raise
        finally:
            try:
                if server.poll() is None:
                    server.stdin.write("stop\n")
                    server.stdin.flush()
                    server.wait(40)
            except (OSError, subprocess.TimeoutExpired):
                terminate(server)
            st.join(5)
            save()
    return 0 if results["complete"] else 1

if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("java","jar","accepted-eula","output"):
        parser.add_argument("--"+name, required=True, type=Path)
    parser.add_argument("--heap", type=int, default=1536)
    parser.add_argument("--seconds", type=int, default=20)
    parser.add_argument("--players", type=int, nargs="+", default=[0,1,2,4])
    options=parser.parse_args()
    options.java=options.java.resolve()
    options.jar=options.jar.resolve()
    raise SystemExit(run(options))
