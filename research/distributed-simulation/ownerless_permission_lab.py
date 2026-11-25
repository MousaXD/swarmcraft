#!/usr/bin/env python3
"""Real two-worker private network and signed creation-policy pin test."""
import argparse
import json
from pathlib import Path
import os
import socket
import sys
import tempfile

from network_lab import Lab, cpu_plan, safe_namespace
from player_runtime import fixture
from region_runtime import Rejected, region_roots, require, serial_step, world_root
from regional_authority import Authority, FencedRuntime, candidate_receipts
from ownerless_permission import PolicyChannel, authority_for, parts, rpc, verify_profile


class PolicyLab(Lab):
    def __init__(self, cpus, binding):
        super().__init__(2, 0, cpus)
        self.binding = binding

    def worker_command(self, slot):
        return ["unshare", "--net", sys.executable,
                str(Path(__file__).with_name("network_lab.py").resolve()),
                "--ownerless-worker-binding", self.binding]


def run(cli, cert, expected, compatibility):
    safe_namespace()
    coordinator, cpus, _ = cpu_plan()
    os.sched_setaffinity(0, {coordinator})
    with tempfile.TemporaryDirectory(prefix="sc65-workers-") as temp:
        binding = verify_profile(cli, cert, expected, compatibility, Path(temp)/"no-profile")
        world, _, _ = parts(binding)
        state = fixture()
        db = Path(temp)/"policy.db"
        Ledger = authority_for(binding)
        ledger = Ledger.create(db, world, state, [0,1,2], [[0,1,2,3],[4,5,6,7]])
        evidence = {"classification": "LOCAL_POLICY_PIN_REFERENCE_WORKERS_NOT_CANONICAL",
                    "binding": binding, "runs": [], "refusals": []}
        try:
            def refuse(label, action):
                prior = ledger.save()
                try:
                    action()
                except Rejected as error:
                    evidence["refusals"].append({"label": label, "reason": str(error), "tick": prior["tick"]})
                else:
                    raise RuntimeError("unexpected acceptance: "+label)
                require(ledger.save() == prior, "refused receipt mutated ledger")

            refuse("legacy_schema", lambda: Authority(db))
            retired = None
            for gen in (1,2):
                with PolicyLab(cpus, binding) as lab:
                    current = ledger.read()
                    channel = lab.channels[0]
                    grant = current["grants"][0]
                    wrong = binding[:-1]+("0" if binding[-1]!="0" else "1")
                    for label, request in (
                        ("legacy_grant", {"op":"bind_grant","grant":grant}),
                        ("wrong_policy_pin", {"op":"bind_policy_grant","grant":grant,"binding":wrong}),
                        ("wrong_world_grant", {"op":"bind_policy_grant","grant":{**grant,"world":"f"*64},"binding":binding}),
                    ):
                        response = rpc(channel,request)
                        require(response["op"]=="refused","incorrect grant accepted")
                        evidence["refusals"].append({"label":label,"reason":response["reason"],"tick":current["tick"]})
                    require(rpc(channel,{"op":"status"})["phase"]=="new","refusal bound worker")
                    runtime = FencedRuntime(ledger,[PolicyChannel(c,binding) for c in lab.channels])
                    for op in ("op","set_policy","console","rcon","grant_vote"):
                        for c in lab.channels:
                            response = rpc(c,{"op":op})
                            require(response["op"]=="refused","privileged request accepted")
                            evidence["refusals"].append({"label":op,"reason":response["reason"],"tick":current["tick"]})
                    runtime.coordinator.step()
                    receipts=candidate_receipts(runtime.coordinator)
                    if retired is not None:
                        refuse("retired_generation",lambda:ledger.commit(retired))
                    refuse("missing_pin",lambda:ledger.commit([{k:v for k,v in r.items() if k not in ("binding","policy_mac")} for r in receipts]))
                    refuse("wrong_receipt_pin",lambda:ledger.commit([{**receipts[0],"binding":wrong},receipts[1]]))
                    refuse("forged_policy_mac",lambda:ledger.commit([{**receipts[0],"policy_mac":"0"*64},receipts[1]]))
                    accepted=ledger.commit(receipts)
                    state=serial_step(state,gen)
                    require(accepted["tick"]==gen and accepted["runtime_root"]==world_root(gen,region_roots(state)),
                            "accepted reference cut differs")
                    retired=receipts
                    evidence["runs"].append({"generation":gen,"worker_pids":[p.pid for p in lab.processes],
                                             "namespaces":lab.namespaces,"coordinator_namespace":lab.parent_ns,
                                             "root":accepted["runtime_root"],"tick":gen,
                                             "receipt_bindings":[r["binding"] for r in receipts]})
                require(all(p.returncode==0 for p in lab.processes) and {n for _,n in socket.if_nameindex()}=={"lo"},
                        "worker cleanup incomplete")
                if gen==1:
                    before=ledger.save()
                    ledger.close()
                    ledger=Ledger(db)
                    require(ledger.save()==before,"reopen changed policy cut")
                    ledger.rotate(1,1,before["runtime_root"],[[4,5,6,7],[0,1,2,3]])
            evidence["all_workers_exited"]=True
            evidence["final_tick"]=ledger.save()["tick"]
            return evidence
        finally:
            ledger.close()


if __name__=="__main__":
    p=argparse.ArgumentParser()
    p.add_argument("--cli",type=Path,required=True)
    p.add_argument("--certificate",type=Path,required=True)
    p.add_argument("--expected-world",required=True)
    p.add_argument("--compatibility",required=True)
    p.add_argument("--output",type=Path,required=True)
    a=p.parse_args()
    require(not a.output.exists(),"new output required")
    result=run(a.cli,a.certificate,a.expected_world,a.compatibility)
    with a.output.open("x") as f:
        json.dump(result,f,indent=2)
        f.write("\n")
    print(json.dumps({"ticks":result["final_tick"],"refusals":len(result["refusals"]),"workers":4}))
