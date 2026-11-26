#!/usr/bin/env python3
"""Exercise the PUBLIC isolated Fabric transaction under real delegated admission.

Invoked by qualify_fabric_lifecycle.py in its disposable systemd scope.
No test-only admission, alternate launcher, or production configuration override.
The interruption scenarios affect only the directly spawned fixture CLI process.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import time

from verify_core_runtime import digest, require


NO_AUTHORITY = ("canonical_authority", "write_authority", "runtime_activation",
                "player_admission", "minecraft_gameplay_verified")


def no_authority(report):
    require(all(report.get(flag) is False for flag in NO_AUTHORITY), "unexpected authority claim")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("cli", "root", "profile", "package"):
        parser.add_argument(f"--{name}", required=True, type=Path)
    for name in ("world", "grant", "checkpoint"):
        parser.add_argument(f"--{name}", required=True)
    parser.add_argument("--heap-mib", type=int, required=True)
    parser.add_argument("--scenario", choices=("complete", "revoke", "kill", "memory-refusal"), required=True)
    args = parser.parse_args()
    os.umask(0o077)
    root = args.root.resolve(strict=True)
    require(root.name.startswith("sc8-fabric-lifecycle-"), "not a disposable fixture root")
    profile, package = args.profile.resolve(strict=True), args.package.resolve(strict=True)
    require(profile.parent == package.parent == root and profile.name == "device-a" and package.name == "package",
            "profile/package outside disposable fixture")
    delegated = Path(os.environ["SWARMCRAFT_HOST_CGROUP_ROOT"])
    require(delegated.name.startswith("swarmcraft-memory-cap-test-fabric-") and delegated.name.endswith(".scope"),
            "not a dedicated fixture scope")
    membership = Path("/proc/self/cgroup").read_text().strip()
    require(membership == "0::" + str(delegated.relative_to("/sys/fs/cgroup")).join(("/", "/controller")),
            "controller is outside exact delegated scope")
    store = profile / "exclusive-turn-reference" / args.world.removeprefix("scpass:")
    journal = store / "journal.postcard"
    sidecar = store / "runtime-local.postcard"
    record = store / "fabric-process.json"
    prefix = [str(args.cli.resolve(strict=True)), "--data-dir", str(profile), "exclusive-turn"]
    pin = ["--expected-world", args.world, "--expected-grant", args.grant]
    base = [*pin, "--expected-checkpoint", args.checkpoint, "--confirm"]

    def command(action, *parts, common=base):
        return [*prefix, "runtime", action, *common, *map(str, parts)]

    def call(action, *parts, common=base, refusal=None):
        result = subprocess.run(command(action, *parts, common=common), capture_output=True, text=True, timeout=150)
        if refusal:
            require(result.returncode != 0 and not result.stdout and refusal in result.stderr,
                    f"unexpected {action} refusal: {result.stderr}")
            return None
        require(result.returncode == 0, f"{action}: {result.stderr}")
        report = json.loads(result.stdout)
        no_authority(report)
        return report

    def inspect():
        result = subprocess.run(command("inspect-intent", common=pin), capture_output=True, text=True, timeout=5)
        if result.returncode != 0:
            if "runtime local state busy" in result.stderr and not result.stdout:
                return None
            raise RuntimeError(f"intent inspection: {result.stderr}")
        report = json.loads(result.stdout)
        no_authority(report)
        require(report["signed_intent_verified"] is True and report["current_journal_checked"] is False
                and report["current_grant_verified"] is False, "inspection claimed current authority")
        require(report["intent_scope"]["grant"] == args.grant and report["intent_scope"]["checkpoint"] == args.checkpoint,
                "signed intent scope differs")
        return report

    require(not record.exists() and not sidecar.exists(), "fixture already has runtime state")
    consent = call("consent", "--expected-revision", "0", "--heap-mib", args.heap_mib, "--cpu-millicores", 2000)
    require(consent["revision"] == "1", "unexpected consent revision")
    reserved = call("reserve", "--expected-revision", "1", "--package", package)
    require(reserved["revision"] == "2" and reserved["intent_kind"] == "unlaunched_no_process_v1",
            "unexpected reservation")
    reserved_id = reserved["intent_id"]
    original_journal, original_sidecar = digest(journal), digest(sidecar)
    qualification = command("qualify-fabric", "--expected-revision", "2", "--expected-intent", reserved_id,
                            "--package", package, "--accept-minecraft-eula")
    result_path = root / "public-qualification.json"

    if args.scenario == "memory-refusal":
        result = subprocess.run(qualification, capture_output=True, text=True, timeout=180)
        require(result.returncode != 0 and not result.stdout and "insufficient hosting memory" in result.stderr,
                f"expected ordinary memory admission refusal, got: {result.stderr}")
        require(digest(journal) == original_journal and digest(sidecar) == original_sidecar,
                "capacity refusal changed signed state")
        require(not record.exists() and not list(store.glob("fabric-work-*")) and not list(store.glob("runtime-stage-*")),
                "capacity refusal created process or workspace")
        observed = inspect()
        require(observed and observed["intent_id"] == reserved_id and observed["revision"] == "2"
                and observed["process_spawned"] is False, "capacity refusal changed reserved intent")
        cleared = call("clear", "--expected-revision", "2", "--expected-intent", reserved_id)
        require(cleared["revision"] == "3" and cleared["intent_id"] is None and cleared["consent"] is None,
                "unlaunched clearance did not consume consent")
        require(digest(journal) == original_journal, "clear changed journal")
        report = {"classification": "public_fabric_memory_admission_refused", "reason": result.stderr.strip(),
                  "normal_admission_tested": True, "full_success_verified": False,
                  "signed_reservation_preserved": True, "old_checkpoint_preserved": True,
                  "no_process_or_workspace": True, "consent_consumed_by_clear": True}
    elif args.scenario == "complete":
        result = subprocess.run(qualification, capture_output=True, text=True, timeout=360)
        require(result.returncode == 0, f"public qualification did not complete: {result.stderr}")
        report = json.loads(result.stdout)
        no_authority(report)
        require(report["classification"] == "exclusive_turn_isolated_fabric_checkpoint_published"
                and report["prior_checkpoint"] == args.checkpoint and report["revision"] == "5"
                and report["intent_id"] is None and report["consent"] is None, "unexpected publication state")
        require(all(report[flag] is True for flag in ("save_barrier_verified", "resource_enforcement_verified",
                "capacity_verified", "process_and_descendants_quiescent", "empty_cgroup_retired",
                "staged_generation_retired", "fabric_workspace_retired")), "incomplete public qualification")
        require(not record.exists() and not list(store.glob("fabric-work-*")) and not list(store.glob("runtime-stage-*")),
                "successful qualification left process record or generation")
        require(digest(journal) != original_journal, "public qualification did not publish changed journal")
        report["normal_admission_tested"] = True
        report["full_success_verified"] = True
    else:
        # Files and PID are scoped to this exact fixture-owned child. We do
        # not signal arbitrary JVM PIDs or manipulate the account record.
        with subprocess.Popen(qualification, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True) as child:
            try:
                deadline = time.monotonic() + 240
                while True:
                    require(child.poll() is None, "public qualification exited before actual spawn")
                    require(time.monotonic() < deadline, "public spawn acknowledgement timed out")
                    observed = inspect()
                    if observed and observed["intent_kind"] == "fabric_may_have_spawned_v1":
                        workspace = store / observed["fabric_workspace"]
                        require(workspace.parent == store and workspace.name.startswith("fabric-work-"), "unexpected workspace")
                        if record.is_file() and (workspace / "fabric-child-started").is_file():
                            break
                    time.sleep(0.05)
                intent, revision = observed["intent_id"], observed["revision"]
                require(revision == "3" and intent != reserved_id, "unexpected running signed intent")
                if args.scenario == "revoke":
                    revoked = call("revoke", "--expected-revision", revision, common=[*pin, "--confirm"])
                    require(revoked["intent_id"] == intent and revoked["consent"] is None
                            and revoked["revision"] == "4", "withdrawal cleared or changed running intent")
                    revision = "4"
                else:
                    # Actual loss of the public controller, not an injected
                    # success/failure hook in its checkpoint code.
                    child.kill()
                out, err = child.communicate(timeout=180)
                require(child.returncode != 0 and not out, f"interrupted qualification published success: {out}")
                if args.scenario == "revoke":
                    require("revision differs" in err or "lost exact local consent" in err,
                            f"unexpected withdrawal failure: {err}")
                require(digest(journal) == original_journal, "interrupted qualification published a new checkpoint")
                observed = inspect()
                require(observed and observed["intent_id"] == intent and observed["revision"] == revision
                        and observed["reference_operations_fenced"] is True, "interruption lost durable fence")
                call("clear", "--expected-revision", revision, "--expected-intent", intent,
                     refusal="exact unlaunched pin")
                call("discard-interrupted-fabric", "--expected-revision", revision, "--expected-intent", intent,
                     refusal="explicit --acknowledge-unconfirmed-save required")
                discard = command("discard-interrupted-fabric", "--expected-revision", revision,
                                  "--expected-intent", intent, "--acknowledge-unconfirmed-save")
                live_refusals = 0
                deadline = time.monotonic() + 180
                while True:
                    result = subprocess.run(discard, capture_output=True, text=True, timeout=15)
                    if result.returncode == 0:
                        retired = json.loads(result.stdout)
                        break
                    require(not result.stdout and ("process is still alive" in result.stderr
                            or "cgroup still contains processes" in result.stderr), f"unexpected recovery failure: {result.stderr}")
                    live_refusals += 1
                    require(time.monotonic() < deadline, "actual Fabric descendants did not exit")
                    time.sleep(0.2)
                no_authority(retired)
                require(retired["classification"] == "exclusive_turn_interrupted_fabric_discarded"
                        and retired["recovery_method"] == "exact_empty_cgroup_retired"
                        and retired["unconfirmed_writes_published"] is False and retired["old_checkpoint_preserved"] is True
                        and retired["intent_id"] is None and retired["consent"] is None
                        and int(retired["revision"]) == int(revision) + 1, "incorrect recovery result")
                require(not record.exists() and workspace.is_dir() and digest(journal) == original_journal,
                        "discard removed uncertain workspace or changed journal")
                report = {"classification": "public_fabric_interruption_discarded", "scenario": args.scenario,
                          "normal_admission_tested": True, "full_success_verified": False,
                          "real_java_spawn_observed": True, "old_checkpoint_preserved": True,
                          "live_recovery_refusals": live_refusals, "retained_workspace": workspace.name,
                          "recovery": retired}
            finally:
                if child.poll() is None:
                    child.kill()
                    child.wait(timeout=5)
    report.update({flag: False for flag in NO_AUTHORITY})
    result_path.write_text(json.dumps(report, sort_keys=True))
    print(json.dumps(report, sort_keys=True), flush=True)


if __name__ == "__main__":
    main()
