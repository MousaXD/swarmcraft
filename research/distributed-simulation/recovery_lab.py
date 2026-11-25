#!/usr/bin/env python3
"""#62 actual capped worker retirement, loss and partition; local research only."""
import argparse
import json
import os
from pathlib import Path
import secrets
import socket
import tempfile
import time

from authority_lab import GROUPS
from network_lab import Lab, command, cpu_plan, safe_namespace
from placement_lab import CappedLab, Scope, values
from player_runtime import fixture
from recovery_runtime import RecoveryAuthority
from region_runtime import Rejected, require
from regional_authority import candidate_receipts
from transition_lab import Clients, commands
from transition_runtime import DurableRuntime
from worker_placement import Offer, MAX_SAMPLE_AGE_NS, choose, evaluate, region_load

CLASSIFICATION = "LOCAL_DURABLE_CAPPED_RECOVERY_NOT_MINECRAFT_NOT_QUORUM"


class RecoveryLab(CappedLab):
    """Reuse the actual #58 resource guards with the #60 fenced worker."""
    def __init__(self, scope, positive, delays, cpus, ledger, scope_id):
        self.cleanup = []
        self.fault_slot = None
        self.ledger = ledger
        self.scope_id = scope_id
        super().__init__(scope, positive, delays, cpus)

    def _fence_unexpected(self):
        status = self.ledger.status()
        require(status["scope"] == self.scope_id, "unexpected scope change")
        if status["phase"] not in ("paused", "draining"):
            self.ledger.fence(self.scope_id, status["generation"], "startup" if
                              status["phase"] in ("probing", "ready") else "policy")
        self.scope.poisoned = True

    def __enter__(self):
        try:
            return super().__enter__()
        except BaseException:
            self._fence_unexpected()
            raise

    def __exit__(self, kind, value, traceback):
        # No unexpected failure can leave a publishable lifecycle while
        # the actual worker sessions are being torn down.
        try:
            if kind is not None:
                self._fence_unexpected()
        finally:
            super().__exit__(kind, value, traceback)

    def worker_command(self, index):
        result = super().worker_command(index)
        require(result[-1] == "--worker", "unexpected worker entry")
        result[-1] = "--fenced-worker"
        return result

    def close(self):
        if not self.scope.active:
            return
        try:
            Lab.close(self)
            for offer, process, leaf in zip(self.offers, self.processes, self.leaves):
                require(process.returncode == 0 or
                        (offer.slot == self.fault_slot and process.returncode == -9),
                        "unexpected worker termination")
                populated = values(leaf/"cgroup.events")["populated"]
                require(populated == 0 and not (leaf/"cgroup.procs").read_text().strip(),
                        "worker budget still occupied")
                self.cleanup.append({"slot": offer.slot, "pid": process.pid,
                                     "exitcode": process.returncode, "populated": populated,
                                     "cpu_millicores": offer.cpu_millicores,
                                     "memory_mib": offer.process_memory_mib})
            require(len(self.cleanup) == len(self.offers) == len(self.leaves), "missing cleanup")
            self.scope.end(self.leaves)
            require(all(not leaf.exists() for leaf in self.leaves)
                    and {name for _, name in socket.if_nameindex()} == {"lo"}, "scope cleanup incomplete")
        except BaseException:
            self.scope.poisoned = True
            raise


def offers(a, b):
    return [a, b, Offer(2, 1, 0, 0), Offer(3, 1, 0, 0)]


def run(ledger, scope, cpus):
    stages, observations, clients, faults, pauses, refused = [], [], [], [], [], []

    def capture(label):
        observations.append({"label": label, "save": ledger.save(),
                             "status": ledger.status(), "journal": ledger.audit()})

    def assert_refused(label, action):
        before, status, journal = ledger.save(), ledger.status(), ledger.audit()
        try:
            action()
        except Rejected as error:
            refused.append({"label": label, "reason": str(error), "tick": before["tick"],
                            "generation": before["generation"], "root": before["runtime_root"]})
        else:
            raise Rejected("unexpected acceptance: " + label)
        require((before, status, journal) == (ledger.save(), ledger.status(), ledger.audit()),
                "refusal changed durable state")

    def paused(label):
        before, status, events = ledger.save(), ledger.status(), ledger.events()
        require(ledger.begin_probe() == [] and ledger.status() == status
                and ledger.save() == before and ledger.events() == events and not scope.active,
                "empty capacity started a generation")
        pauses.append({"label": label, "status": status, "spawned_slots": []})
        capture(label)

    def stage(label, expected_slots, task=None):
        require(not scope.active and not scope.poisoned, "old kernel scope not released")
        positive = ledger.begin_probe()
        probe = ledger.status()
        require(positive and probe["phase"] == "probing", "no authorized probes")
        row = {"label": label, "start": probe, "workers": [], "ticks": [],
               "decision": None, "cleanup": []}
        stages.append(row)
        with RecoveryLab(scope, positive, {}, cpus, ledger, probe["scope"]) as lab:
            samples, measurements = lab.probe()
            decision_ns = time.monotonic_ns()
            chosen = choose([Offer(**o) for o in probe["offers"]], samples,
                            region_load(ledger.read()["state"]), decision_ns)
            plan = chosen["plan"]
            selected = sorted(set(plan["owner_by_region"])) if plan else []
            require(selected == expected_slots, "unexpected placement: " + str(chosen))
            retired = []
            for index, offer in enumerate(lab.offers):
                if offer.slot not in selected:
                    lab.retire(index)
                    retired.append(offer.slot)
            decision = ledger.admit(probe["scope"], samples, retired, decision_ns)
            require(decision == chosen, "durable admission differs")
            row.update(decision=decision, decision_ns=decision_ns, probes=measurements,
                       workers=[{"slot": o.slot, "pid": p.pid, "namespace": ns}
                                for o, p, ns in zip(lab.offers, lab.processes, lab.namespaces)],
                       coordinator_namespace=lab.parent_ns, machine=lab.machine)
            indices = [i for i, o in enumerate(lab.offers) if o.slot in selected]
            if selected:
                runtime = DurableRuntime(ledger, [lab.channels[i] for i in indices])
                row["generation"] = runtime.runtime.generation
                row["initial_grants"] = ledger.save()["grants"]

                def guard(state):
                    require(all(0 <= time.monotonic_ns()-samples[s].observed_ns <= MAX_SAMPLE_AGE_NS
                                for s in selected), "worker measurement expired")
                    observed = [lab.observe(i) for i in indices]
                    require(all(k["effective_cpu_millicores"] >= samples[s].effective_cpu_millicores
                                for k, s in zip(observed, selected)), "CPU capacity changed")
                    scope.recheck(observed)
                    require(evaluate(plan["owner_by_region"], region_load(state), samples) is not None,
                            "current load exceeds verified placement")
                    return observed

                def advance(label, request, injected=None):
                    with Clients(ledger) as peers:
                        for p, q in request.items():
                            peers.spawn(p, q)
                        require(all(s["submission"]["op"] == "queued" for s in peers.sessions),
                                "expected pending client input")
                        before = ledger.save()
                        pre = guard(ledger.read()["state"])
                        post = None
                        def before_publish():
                            nonlocal post
                            state, _ = runtime.runtime.coordinator.snapshot()
                            post = guard(state)
                        try:
                            accepted = runtime.advance(fault=injected, before_publish=before_publish)
                        except (Rejected, OSError, EOFError, TimeoutError) as error:
                            require(injected is not None and lab.fault_slot is not None
                                    and runtime.runtime.frozen and runtime.runtime.coordinator.poisoned
                                    and ledger.save() == before, "unexpected failure or changed cut")
                            row["fault_error"] = type(error).__name__
                            clients.append({"label": label, "sessions": peers.finish(lose_ack=True)})
                            return False
                        require(injected is None and post is not None, "fault did not prevent publication")
                        row["ticks"].append({"label": label, "tick": accepted["tick"],
                                             "root": accepted["runtime_root"],
                                             "kernel_before": pre, "kernel_after": post})
                        clients.append({"label": label, "sessions": peers.finish()})
                        capture(label)
                        return True

                task(lab, runtime, advance, guard)
                status = ledger.status()
                if status["phase"] != "draining":
                    ledger.fence(status["scope"], status["generation"], "stop")
                require(ledger.status()["phase"] == "draining", "scope not fenced before close")
                # No later publication may use old grants even before processes exit.
                runtime.runtime.frozen = runtime.runtime.coordinator.poisoned = True
            else:
                row["generation"] = ledger.status()["generation"]
                require(ledger.status()["phase"] == "draining", "infeasible plan did not fence")
            row["before_cleanup"] = ledger.status()
        require(not scope.active and not scope.poisoned and len(lab.cleanup) == len(positive),
                "cleanup failed; cannot release scope")
        row["cleanup"] = lab.cleanup
        row["leaves_removed"] = True
        row["network_clean"] = True
        current = ledger.status()
        ledger.release(current["scope"], current["generation"], [o.slot for o in positive])
        row["released"] = ledger.status()
        capture(label + "_released")

    def restart():
        nonlocal ledger
        path = ledger.path
        before, status, journal, events = ledger.save(), ledger.status(), ledger.audit(), ledger.events()
        ledger.close()
        ledger = RecoveryAuthority(path)
        require((ledger.save(), ledger.status(), ledger.audit(), ledger.events()) ==
                (before, status, journal, events), "reopen changed durable recovery state")

    def initial(lab, runtime, advance, guard):
        nonlocal pending
        advance("tick1", commands(ledger, 1, 1))
        pending = commands(ledger, -1, -1)
        with Clients(ledger) as peers:
            for p, q in pending.items():
                peers.spawn(p, q)
            require(all(s["submission"]["op"] == "queued" for s in peers.sessions), "missing queued input")
            # Complete the tentative work but withdraw at the next accepted
            # cut boundary before publishing it.
            guard(ledger.read()["state"])
            controls = runtime.seal()
            control = runtime.runtime.coordinator
            control.step(controls=controls)
            stale = candidate_receipts(control)
            old_status = ledger.status()
            ledger.configure(offers(Offer(0, 3, 96, 1000), Offer(1, 3, 0, 0)))
            require(ledger.status()["phase"] == "draining", "withdrawal did not fence")
            assert_refused("withdrawn_generation_candidate", lambda: ledger.commit(stale))
            assert_refused("withdrawn_generation_seal", runtime.seal)
            assert_refused("start_before_cleanup", ledger.begin_probe)
            assert_refused("incomplete_cleanup", lambda: ledger.release(old_status["scope"],
                          old_status["generation"], [0]))
            clients.append({"label": "tick2_withdrawn", "sessions": peers.finish(lose_ack=True)})
            capture("withdrawal_fenced")
        require(all(p.poll() is None for p in lab.processes), "workers not live at withdrawal")
        stages[-1]["withdrawal_workers_alive"] = True

    pending = None
    capture("genesis")
    ledger.configure(offers(Offer(0, 2, 56, 1000), Offer(1, 2, 56, 1000)))
    stage("two_workers", [0, 1], initial)
    restart()

    def scale_in(lab, runtime, advance, guard):
        require(advance("tick2_recovered", pending), "scale-in recovery failed")
        ledger.configure(offers(Offer(0, 4, 0, 0), Offer(1, 3, 0, 0)))

    stage("scale_in", [0], scale_in)
    restart()
    paused("all_player_only")
    ledger.configure(offers(Offer(0, 5, 56, 1000), Offer(1, 4, 56, 1000)))

    def fault_task(kind, target, label, request, lab, runtime, advance):
        index = next(i for i, o in enumerate(lab.offers) if o.slot == target)
        phase = "prepared" if kind == "worker_loss" else "sealed"
        def inject(at):
            if at == phase:
                lab.fault_slot = target
                lab.expected_fault = True
                if kind == "worker_loss":
                    lab.processes[index].kill()
                    lab.processes[index].wait(timeout=3)
                else:
                    command("ip", "link", "set", lab.interfaces[index], "down")
        require(not advance(label, request, inject), "fault was not refused")
        current = ledger.status()
        ledger.fence(current["scope"], current["generation"], kind, [target])
        faults.append({"label": label, "kind": kind, "phase": phase, "slot": target,
                       "pid": lab.processes[index].pid, "last_tick": current["tick"],
                       "last_root": current["root"], "frozen": runtime.runtime.frozen,
                       "blocked": ledger.status()["blocked"]})
        capture(label)

    def kill_stage(lab, runtime, advance, guard):
        nonlocal pending
        advance("tick3", commands(ledger, 1, 1))
        pending = commands(ledger, -1, -1)
        fault_task("worker_loss", 1, "tick4_worker_killed", pending, lab, runtime, advance)

    stage("worker_loss", [0, 1], kill_stage)
    restart()
    stage("insufficient_after_loss", [])
    restart()
    ledger.configure(offers(Offer(0, 6, 96, 1000), Offer(1, 4, 56, 1000)))

    def partition_stage(lab, runtime, advance, guard):
        nonlocal pending
        advance("tick4_recovered", pending)
        pending = commands(ledger, 1, 1)
        fault_task("partition", 0, "tick5_partition", pending, lab, runtime, advance)

    stage("partition", [0], partition_stage)
    restart()
    paused("all_workers_unavailable")
    ledger.configure(offers(Offer(0, 6, 96, 1000), Offer(1, 5, 96, 1000)))

    def final(lab, runtime, advance, guard):
        advance("tick5_recovered", pending)
        advance("tick6", commands(ledger, -1, -1))

    stage("recovered", [1], final)
    restart()
    result = {"schema": 1, "classification": CLASSIFICATION, "stages": stages,
              "observations": observations, "clients": clients, "faults": faults,
              "pauses": pauses, "refusals": refused, "events": ledger.events(),
              "final": ledger.save(), "journal": ledger.audit(),
              "all_scopes_released": not scope.active and not scope.poisoned}
    ledger.close()
    return result


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    require(not args.output.exists(), "new receipt required")
    safe_namespace()
    os.umask(0o077)
    control_cpu, cpus, affinity = cpu_plan()
    scope = Scope(affinity)
    os.sched_setaffinity(0, {control_cpu})
    with tempfile.TemporaryDirectory(prefix="sc62-") as tmp:
        ledger = RecoveryAuthority.create(Path(tmp)/"world.db", secrets.token_hex(32),
                                          fixture(), [0, 1, 2], GROUPS)
        result = run(ledger, scope, cpus)
    with args.output.open("x") as output:
        json.dump(result, output, separators=(",", ":"), allow_nan=False)
        output.write("\n")
    print(json.dumps({"ticks": result["final"]["tick"], "stages": len(result["stages"]),
                      "faults": result["faults"], "all_scopes_released": result["all_scopes_released"]}, indent=2))


if __name__ == "__main__":
    main()
