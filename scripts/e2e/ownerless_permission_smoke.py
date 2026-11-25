#!/usr/bin/env python3
"""#65 real Fabric permission lab + isolated reference-worker policy receipts.

Requires explicitly supplied local artifacts, Java 25, and a fresh mapped-root
Linux user/network namespace. Never opens a real world or authenticates a
Minecraft account; this is not canonical authority, or a production launcher.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT/"research/distributed-simulation"))
from network_lab import safe_namespace
from ownerless_permission import parts, verify_profile
from ownerless_permission_lab import run as run_workers
from player_runtime import fixture
from region_runtime import canonical, digest, integer, region_roots, require, serial_step, world_root

CLASSIFICATION = "LOCAL_FABRIC_PERMISSION_AND_REFERENCE_WORKER_LAB_NOT_CANONICAL"
PROPS = {
    "server-ip":"127.0.0.1", "server-port":"25565",
    "online-mode":"true", "enforce-secure-profile":"true",
    "white-list":"true", "enforce-whitelist":"true",
    "enable-rcon":"false", "enable-query":"false", "rcon.port":"25575",
    "management-server-enabled":"false", "management-server-port":"25585",
    "enable-jmx-monitoring":"false", "op-permission-level":"0",
    "function-permission-level":"0", "level-type":"minecraft:flat",
    "generator-settings":'{"layers":[{"block":"minecraft:bedrock","height":1},{"block":"minecraft:dirt","height":2},{"block":"minecraft:grass_block","height":1}],"biome":"minecraft:plains","structure_overrides":[]}',
    "generate-structures":"false", "view-distance":"2", "simulation-distance":"2",
    "level-seed":"65", "level-name":"world",
}
CHECKS = ["signed-policy-pin-active","management-listeners-disabled","actual-management-sockets-closed",
          "initial-op-list-empty"] + [f"{p}-{s}" for p in ("Founder","Successor","Ordinary") for s in
          ("ordinary-player","op-and-elevation-denied","ordinary-command-available",
           "protected-commands-denied","direct-op-and-bypass-denied","live-admission-gated")] + [
          "console-elevation-denied","rcon-command-denied","policy-change-denied",
          "command-block-execution-denied","final-op-list-empty"]
LEGACY = ["legacy-mode-inactive","legacy-console-unchanged","legacy-operator-grant-unchanged"]
NEGATIVE = {
    "wrong_pin":"Invalid ownerless permission lab binding",
    "mixed_v1_ipc":"Ownerless lab cannot attach to a V1 managed runtime",
    "unsafe_rcon":"Unsafe ownerless permission lab settings",
    "unsafe_management":"Unsafe ownerless permission lab settings",
    "inherited_ops":"Ownerless permission lab refuses inherited operator list",
}


def sha(path):
    with path.open("rb") as f:
        return hashlib.file_digest(f,"sha256").hexdigest()


def invoke(cli, profile, *args):
    return subprocess.run([str(cli),"--data-dir",str(profile),*args],
                          check=True,capture_output=True,timeout=15).stdout


def signed_fixture(cli, temp, compatibility):
    keys = []
    profiles = []
    for i in range(3):
        profile = temp/f"validator-{i}"
        result=invoke(cli,profile,"identity").decode()
        key=re.search(r"^Public key: ([0-9a-f]{64})$",result,re.M)
        require(key is not None,"identity output missing public key")
        keys.extend(["--validator-key",key[1]])
        profiles.append(profile)
    organizer=temp/"organizer"
    draft=temp/"draft.json"
    root=world_root(0,region_roots(fixture()))
    draft.write_bytes(invoke(cli,organizer,"ownerless","draft","--name","Ownerless permission lab",
                             "--compatibility-fingerprint",compatibility,
                             "--initial-state-commitment",root,*keys))
    world=json.loads(invoke(cli,organizer,"ownerless","inspect","--draft",str(draft)))["world_id"]
    approvals=[]
    for i,profile in enumerate(profiles):
        path=temp/f"approval-{i}.json"
        path.write_bytes(invoke(cli,profile,"ownerless","approve","--draft",str(draft),
                                "--expected-world",world,"--confirm"))
        approvals.extend(["--approval",str(path)])
        require(not any((profile/"worlds").iterdir()),"unexpected canonical world")
    cert=temp/"certificate.json"
    cert.write_bytes(invoke(cli,organizer,"ownerless","assemble","--draft",str(draft),
                            "--expected-world",world,*approvals))
    binding=verify_profile(cli,cert,world,compatibility,organizer)
    return world,cert,binding


def stage_artifacts(args,temp):
    stage=temp/"artifacts"
    stage.mkdir()
    loader=stage/"loader"
    loader.mkdir()
    files={"server":args.server,"bridge":args.bridge,"probe":args.probe}
    copied={}
    manifest={}
    for name,path in files.items():
        dst=stage/(name+".jar")
        shutil.copyfile(path,dst)
        copied[name]=dst
        manifest[name]={"sha256":sha(dst),"bytes":dst.stat().st_size}
    libs=sorted(args.loader_libs.glob("*.jar"))
    require(len(libs)==7 and any(p.name=="fabric-loader-0.19.3.jar" for p in libs)
            and any(p.name.startswith("sponge-mixin-") for p in libs),"wrong loader dependency set")
    for path in libs:
        dst=loader/path.name
        shutil.copyfile(path,dst)
        manifest["loader/"+dst.name]={"sha256":sha(dst),"bytes":dst.stat().st_size}
    copied["loader"]=loader
    # Lab-only artifact fingerprint; not the production compatibility manifest.
    compatibility=hashlib.sha256(b"sc65-lab-artifacts-v1\0"+canonical(manifest)).hexdigest()
    return copied,manifest,compatibility


def launch(args, temp, artifacts, binding, label, *, restore=None, ops=None, overrides=None, extra_env=None):
    runtime=temp/label
    runtime.mkdir()
    (runtime/"mods").mkdir()
    for name in ("bridge","probe"):
        shutil.copyfile(artifacts[name],runtime/"mods"/(name+".jar"))
    if restore is not None:
        shutil.copytree(restore/"world",runtime/"world")
    if ops is not None:
        shutil.copyfile(ops,runtime/"ops.json")
    props={**PROPS,**(overrides or {})}
    (runtime/"server.properties").write_text("".join(f"{k}={v}\n" for k,v in props.items()))
    (runtime/"eula.txt").write_text("eula=true\n")
    env={"PATH":"/usr/bin:/bin","HOME":str(runtime),
         "SC65_PROBE_MODE":"legacy" if label=="legacy" else "restricted"}
    if binding is not None:
        env["SWARMCRAFT_OWNERLESS_LAB_BINDING"]=binding
    env.update(extra_env or {})
    argv=[str(args.java),"-XX:ActiveProcessorCount=2","-Xms128m","-Xmx1024m",
          "-Dfabric.gameJarPath="+str(artifacts["server"]),
          "-cp",str(artifacts["loader"]/"*")+os.pathsep+str(artifacts["server"]),
          "net.fabricmc.loader.impl.launch.knot.KnotServer","nogui"]
    process=subprocess.run(argv,env=env,cwd=runtime,stdin=subprocess.DEVNULL,
                           stdout=subprocess.PIPE,stderr=subprocess.STDOUT,timeout=90)
    log=process.stdout.decode(errors="replace")
    checks=re.findall(r"OWNERLESS_PROBE_PASS ([A-Za-z0-9-]+)",log)
    complete=re.findall(r"OWNERLESS_PROBE_COMPLETE ([a-z]+)",log)
    refusal=NEGATIVE.get(label)
    result={"label":label,"exitcode":process.returncode,"checks":checks,"complete":complete,
            "binding_observed":binding is not None and ("permission lab active: "+binding) in log,
            "game_listener_started":"Starting Minecraft server on 127.0.0.1:25565" in log,
            "saved_ops":json.loads((runtime/"ops.json").read_text()) if (runtime/"ops.json").exists() else None,
            "log_sha256":hashlib.sha256(process.stdout).hexdigest(),
            "refusal":refusal if refusal and refusal in log else None,
            "auth_service_unreachable":"Network is unreachable" in log,
            "probe_failure":"OWNERLESS_PROBE_FAIL" in log}
    if refusal:
        require(result["refusal"]==refusal and not checks and not complete and not result["game_listener_started"],
                "unsafe startup not refused: "+label+"\n"+log[-5000:])
    else:
        expected=LEGACY if label=="legacy" else CHECKS
        require(process.returncode==0 and checks==expected and complete==[env["SC65_PROBE_MODE"]]
                and not result["probe_failure"] and result["game_listener_started"],
                "runtime probe failed: "+label+"\n"+log[-5000:])
        require(result["binding_observed"]==(label!="legacy"),"wrong active policy")
        require(bool(result["saved_ops"])==(label=="legacy"),"unexpected saved operator list")
    print(f"{label}: {'refused' if refusal else str(len(checks))+' checks passed'}",flush=True)
    return result,runtime


def verify_observations(data):
    require(data["schema"]==1 and data["classification"]==CLASSIFICATION and data["physical_machines"]==1
            and data["live_players_admitted"]==0 and data["canonical_authority"] is False,
            "wrong evidence classification")
    binding=data["binding"]
    world,_,compatibility=parts(binding)
    require(data["world_id"]=="scownerless:"+world and compatibility==hashlib.sha256(
            b"sc65-lab-artifacts-v1\0"+canonical(data["artifact_manifest"])).hexdigest(),
            "wrong artifact binding")
    require(len(data["artifact_manifest"])==10 and all(
        digest(value["sha256"]) and integer(value["bytes"],1,2**32)
        for value in data["artifact_manifest"].values()),"invalid artifact manifest")
    genesis=data["creation_certificate"]["genesis"]
    require(genesis["compatibility_fingerprint"]==list(bytes.fromhex(compatibility))
            and genesis["initial_state_commitment"]==list(bytes.fromhex(world_root(0,region_roots(fixture())))),
            "signed initial state or artifact pin differs")
    labels=["founder_runtime","successor_runtime","legacy",*NEGATIVE]
    require([r["label"] for r in data["fabric_runs"]]==labels,"incomplete Fabric matrix")
    for r in data["fabric_runs"]:
        label=r["label"]
        require(digest(r["log_sha256"]),"missing observed log digest")
        if label in NEGATIVE:
            require(r["refusal"]==NEGATIVE[label] and not r["checks"] and not r["complete"]
                    and not r["binding_observed"] and not r["game_listener_started"],"unsafe launch accepted")
        else:
            legacy=label=="legacy"
            require(r["exitcode"]==0 and r["checks"]==(LEGACY if legacy else CHECKS)
                    and r["complete"]==["legacy" if legacy else "restricted"] and not r["probe_failure"]
                    and r["game_listener_started"] and r["binding_observed"] is (not legacy)
                    and (isinstance(r["saved_ops"],list) and bool(r["saved_ops"]) if legacy
                         else r["saved_ops"]==[]) and r["refusal"] is None,"incorrect permission probe")
    workers=data["workers"]
    require(workers["classification"]=="LOCAL_POLICY_PIN_REFERENCE_WORKERS_NOT_CANONICAL"
            and workers["binding"]==binding and workers["all_workers_exited"] is True
            and workers["final_tick"]==2 and len(workers["runs"])==2,"wrong worker evidence")
    state=fixture()
    pids=set()
    for tick,r in enumerate(workers["runs"],1):
        state=serial_step(state,tick)
        require(r["tick"]==r["generation"]==tick and r["root"]==world_root(tick,region_roots(state))
                and r["receipt_bindings"]==[binding,binding]
                and len(set(r["namespaces"]+[r["coordinator_namespace"]]))==3
                and len(r["worker_pids"])==2 and not pids.intersection(r["worker_pids"]),"worker evidence differs")
        pids.update(r["worker_pids"])
    refusal_labels=["legacy_schema"]
    for generation in (1,2):
        refusal_labels+=["legacy_grant","wrong_policy_pin","wrong_world_grant"]
        refusal_labels += [op for op in ("op","set_policy","console","rcon","grant_vote") for _ in range(2)]
        if generation==2:
            refusal_labels+=["retired_generation"]
        refusal_labels+=["missing_pin","wrong_receipt_pin","forged_policy_mac"]
    require([r["label"] for r in workers["refusals"]]==refusal_labels,"missing worker refusals")
    reasons={"legacy_schema":"invalid local authority database","legacy_grant":"policy binding required",
             "wrong_policy_pin":"policy grant mismatch or already bound",
             "wrong_world_grant":"policy grant mismatch or already bound",
             "retired_generation":"stale ownerless receipt session",
             "wrong_receipt_pin":"ownerless receipt policy mismatch",
             "forged_policy_mac":"ownerless policy receipt authentication failed"}
    for r in workers["refusals"]:
        reason=reasons.get(r["label"],"no privileged worker operation")
        if r["label"]=="missing_pin":
            require("field" in r["reason"] or "shape" in r["reason"],"missing pin not structurally refused")
        else:
            require(r["reason"]==reason,"unexpected worker refusal reason")
        require(r["tick"] in (0,1),"unexpected refusal tick")
    return {"fabric_checks":2*len(CHECKS)+len(LEGACY),"unsafe_launches_refused":len(NEGATIVE),
            "worker_refusals":len(refusal_labels),"worker_ticks":2}


def main():
    p=argparse.ArgumentParser(description=__doc__)
    for arg in ("java","cli","server","bridge","probe","loader-libs","output"):
        p.add_argument("--"+arg,type=Path,required=True)
    p.add_argument("--accept-eula",action="store_true",help="Explicit consent for disposable Minecraft test")
    a=p.parse_args()
    require(a.accept_eula and not a.output.exists(),"explicit EULA and fresh output required")
    for name in ("java","cli","server","bridge","probe","loader_libs","output"):
        setattr(a,name,getattr(a,name).resolve())
    safe_namespace()
    os.umask(0o077)
    with tempfile.TemporaryDirectory(prefix="sc65-fabric-") as d:
        temp=Path(d)
        artifacts,manifest,compatibility=stage_artifacts(a,temp)
        world,cert,binding=signed_fixture(a.cli,temp,compatibility)
        results=[]
        def test(label, pin=binding, **kw):
            row,path=launch(a,temp,artifacts,pin,label,**kw)
            results.append(row)
            return path
        first=test("founder_runtime")
        test("successor_runtime",restore=first)
        legacy=test("legacy",None)
        test("wrong_pin",binding.replace(parts(binding)[1],"f"*64))
        test("mixed_v1_ipc",extra_env={"SWARMCRAFT_IPC_HOST":"127.0.0.1"})
        test("unsafe_rcon",overrides={"enable-rcon":"true","rcon.password":"lab-only"})
        test("unsafe_management",overrides={"management-server-enabled":"true"})
        test("inherited_ops",ops=legacy/"ops.json")
        workers=run_workers(a.cli,cert,world,compatibility)
        data={"schema":1,"classification":CLASSIFICATION,"physical_machines":1,"live_players_admitted":0,
              "canonical_authority":False,"artifact_manifest":manifest,"world_id":world,
              "binding":binding,"creation_certificate":json.loads(cert.read_bytes()),
              "fabric_runs":results,"workers":workers}
        stats=verify_observations(data)
        with a.output.open("x") as out:
            json.dump(data,out,indent=2)
            out.write("\n")
        print(json.dumps(stats))


if __name__=="__main__":
    main()
