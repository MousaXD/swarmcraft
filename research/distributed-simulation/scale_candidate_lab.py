#!/usr/bin/env python3
"""Paired local SQLite commit/receipt CPU and wall cost, NOT network qualification."""
import argparse
import hashlib
import itertools
import json
import os
from pathlib import Path
import statistics
import tempfile
import time

from region_runtime import canonical, fields, integer, region_roots, require, serial_step, world_root
from regional_authority import tag, wire
from scale_lab import PLAYERS, fixture
from scale_profile import PHASES, PhaseClock
from transition_runtime import PROTOCOL, RULESET, TransitionAuthority
from verify_player_evidence import reference_view
from verify_scale_evidence import cut

CLASSIFICATION = "LOCAL_SQLITE_CANDIDATE_COST_NOT_DISTRIBUTED_GAMEPLAY"
COUNTS = (256, 8192)
MODES = tuple(itertools.product(("reparse", "reuse"), ("trace", "boundary")))
REPEATS = 2
WARMUP = 2
SAMPLES = 10
GROUPS = [list(range(4)), list(range(4, 8))]


def sha(value):
    return hashlib.sha256(canonical(value)).hexdigest()


def cases():
    for count in COUNTS:
        for repeat in range(REPEATS):
            # Reverse order on the second repetition, reducing order bias.
            for candidate, observer in MODES if repeat == 0 else reversed(MODES):
                yield count, repeat, candidate, observer


def reference(count, repeat):
    world = sha([CLASSIFICATION, count, repeat])
    states = [fixture("spread", count)]
    for t in range(1, WARMUP + SAMPLES + 1):
        d = 1 if t % 2 else -1
        states.append(serial_step(states[-1], t, controls={p:(d,d) for p in PLAYERS}))
    roots = [world_root(t, region_roots(s)) for t,s in enumerate(states)]
    cuts = [cut(world,t,r) for t,r in enumerate(roots)]
    def view(t,p):
        return {**reference_view(world,t,cuts[t],states[t],PLAYERS,p),
                "protocol":PROTOCOL,"ruleset":RULESET}
    commands, batches, inputs, receipts = [], [], [], []
    for t in range(1, len(states)):
        d = 1 if t % 2 else -1
        batch, incoming, replies = [], [], []
        for p in PLAYERS:
            q = {"op":"act","world":world,"id":sha(["sc63-command",world,p,t]),
                 "tick":t,"parent":cuts[t-1],"direction":d,"action":d}
            r = {"op":"committed","id":q["id"],"view":view(t,p),
                 "inventory_delta":view(t,p)["inventory"]-view(t-1,p)["inventory"]}
            batch.append([p,d,d,q["id"]])
            incoming.append((p,q))
            replies.append(r)
            commands.append({"player":p,"request":q,"response":r})
        inputs.append(incoming)
        receipts.append(replies)
        batches.append({"tick":t,"parent":cuts[t-1],"batch":batch,"result":cuts[t]})
    journal = {"genesis":json.loads(canonical(wire(states[0]))),
               "batches":batches,"commands":commands}
    return world, states, roots, cuts, inputs, receipts, journal


def run_case(count, repeat, candidate, observer):
    world, states, roots, cuts, inputs, replies, journal = reference(count, repeat)
    with tempfile.TemporaryDirectory(prefix="sc63-cost-") as temp:
        path = Path(temp)/"world.db"
        ledger = TransitionAuthority.create(path,world,states[0],PLAYERS,GROUPS)
        try:
            ledger.reuse_validated_candidate = candidate == "reuse"
            grants = ledger.claim(1,0,roots[0])["grants"]
            clock = PhaseClock()
            ledger.phase = clock.phase
            if observer == "trace":
                ledger.db.set_trace_callback(clock.trace)
            else:
                ledger.transaction_event = clock.boundary
            samples = []
            for t in range(1,len(states)):
                # Generate the identical reference candidate outside the
                # measured interval. These are synthetic, signed receipts;
                # no network, worker process or resource guard is measured.
                signed = []
                for grant in grants:
                    body = {"world":world,"generation":1,"session":grant["session"],
                            "tick":t,"parent_root":roots[t-1],"runtime_root":roots[t],
                            "regions":[states[t][r].wire() for r in grant["regions"]]}
                    signed.append({"body":body,"mac":tag(grant["key"],body)})
                ledger.seal_next(1,t-1,roots[t-1],inputs=inputs[t-1])
                clock.begin()
                wall, cpu = time.monotonic_ns(), time.process_time_ns()
                with clock.phase("publication_receipt_validation"):
                    accepted = ledger.commit(signed)
                with clock.phase("client_receipt_lookup"):
                    received = ledger.committed_receipts(inputs[t-1])
                cpu = time.process_time_ns()-cpu
                elapsed = time.monotonic_ns()-wall
                phases = clock.finish(elapsed)
                require(accepted["runtime_root"] == roots[t] and accepted["cut_root"] == cuts[t]
                        and received == replies[t-1],"candidate/receipt differs from reference")
                require(clock.transactions == {"write":1,"read":1,"commit":2,"rollback":0},
                        "unexpected measured transaction boundaries")
                if t > WARMUP:
                    samples.append({"tick":t,"wall_ns":elapsed,"cpu_ns":cpu,"phase_ns":phases,
                                    "transactions":clock.transactions,"root":roots[t],"cut":cuts[t]})
            require(ledger.audit() == journal and ledger.read()["state"] == states[-1],
                    "durable history differs")
        finally:
            ledger.close()
        # Reopen through the normal independent replay, not the live cache.
        reopened = TransitionAuthority(path)
        try:
            observed_journal = reopened.audit()
            saved = reopened.save()
            require(observed_journal == journal and saved["regions"] == json.loads(canonical(wire(states[-1]))),
                    "reopened history differs")
        finally:
            reopened.close()
    return {"agents":count,"repeat":repeat,"candidate_mode":candidate,"observer":observer,
            "world":world,"samples":samples,"journal_hash":sha(observed_journal),
            "final_state_hash":sha(saved["regions"])}


def verify(report):
    fields(report,"schema","classification","cpu","warmup","samples","runs")
    require(type(report["schema"]) is int and report["schema"] == 1
            and report["classification"] == CLASSIFICATION and report["warmup"] == WARMUP
            and report["samples"] == SAMPLES and integer(report["cpu"],0,65535),
            "invalid candidate cost classification")
    expected = list(cases())
    require(len(report["runs"]) == len(expected),"incomplete paired matrix")
    refs = {}
    for row, case in zip(report["runs"],expected):
        fields(row,"agents","repeat","candidate_mode","observer","world","samples",
               "journal_hash","final_state_hash")
        count, repeat, candidate, observer = case
        require((row["agents"],row["repeat"],row["candidate_mode"],row["observer"]) == case,
                "wrong paired case")
        if (count,repeat) not in refs:
            world,states,roots,cuts,_,_,journal = reference(count,repeat)
            refs[count,repeat] = world,roots,cuts,sha(journal),sha(wire(states[-1]))
        world,roots,cuts,journal_hash,state_hash = refs[count,repeat]
        require((row["world"],row["journal_hash"],row["final_state_hash"]) == (world,journal_hash,state_hash)
                and len(row["samples"]) == SAMPLES,"wrong reopened history or sample count")
        for t,s in enumerate(row["samples"],WARMUP+1):
            fields(s,"tick","wall_ns","cpu_ns","phase_ns","transactions","root","cut")
            require(s["tick"] == t and s["root"] == roots[t] and s["cut"] == cuts[t]
                    and integer(s["wall_ns"],1,60_000_000_000)
                    and integer(s["cpu_ns"],1,s["wall_ns"]),"invalid measured sample")
            phases = s["phase_ns"]
            fields(phases,*PHASES)
            require(all(integer(v,0,s["wall_ns"]) for v in phases.values())
                    and sum(phases.values()) == s["wall_ns"]
                    and phases["independent_serial_replay"] > 0
                    and phases["sqlite_durable_commit"] > 0,
                    "invalid measured phase")
            require(s["transactions"] == {"write":1,"read":1,"commit":2,"rollback":0}
                    and all(type(v) is int for v in s["transactions"].values()),"invalid measured boundaries")
    return report


def summarize(report):
    results = []
    for count in COUNTS:
        for candidate,observer in MODES:
            rows = [s for r in report["runs"] if
                    (r["agents"],r["candidate_mode"],r["observer"]) == (count,candidate,observer)
                    for s in r["samples"]]
            results.append({"agents":count,"candidate_mode":candidate,"observer":observer,
                            "samples":len(rows),
                            "median_wall_ms":round(statistics.median(s["wall_ns"] for s in rows)/1e6,3),
                            "median_cpu_ms":round(statistics.median(s["cpu_ns"] for s in rows)/1e6,3),
                            "mean_phase_ms":{k:round(statistics.mean(s["phase_ns"][k] for s in rows)/1e6,3)
                                             for k in PHASES}})
    return results


def main():
    parser = argparse.ArgumentParser()
    group = parser.add_mutually_exclusive_group(required=True)
    group.add_argument("--output",type=Path)
    group.add_argument("--verify",type=Path)
    args = parser.parse_args()
    if args.verify:
        report = verify(json.loads(args.verify.read_text()))
    else:
        require(not args.output.exists(),"new receipt path required")
        os.umask(0o077)
        affinity = os.sched_getaffinity(0)
        cpu = min(affinity)
        try:
            os.sched_setaffinity(0,{cpu})
            report = {"schema":1,"classification":CLASSIFICATION,"cpu":cpu,
                      "warmup":WARMUP,"samples":SAMPLES,"runs":[]}
            for case in cases():
                report["runs"].append(run_case(*case))
            verify(report)
            with args.output.open("x") as out:
                json.dump(report,out,separators=(",",":"),allow_nan=False)
                out.write("\n")
        finally:
            os.sched_setaffinity(0,affinity)
    print(json.dumps(summarize(report),indent=2))


if __name__ == "__main__":
    main()
