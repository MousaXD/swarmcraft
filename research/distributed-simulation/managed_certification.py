"""Durable exact-intent reconciliation for the trusted managed reference lab.

The SQLite intent is a publication barrier, not a distributed lease.
The Rust journal owns votes and certificates; this controller never
deletes a vote, rewrites history, or retargets an interrupted decision.
"""
import json
from pathlib import Path
import secrets
import tempfile

from combined_ownerless_lab import review_checkpoint
from ownerless_permission import parts
from region_runtime import canonical, digest, fields, integer, require
from verify_combined_ownerless_evidence import verify_sync_signatures
from worker_placement import Offer


def check_plan(quorum,plan,*,check_active=True):
    fields(plan,"scope","generation","label","base","base_history","base_checkpoint",
           "candidate","proposal","active","voters","offers","blocked")
    base = plan["base"]
    proposal = plan["proposal"]
    fields(proposal,"proposal_hash","proposal")
    target = proposal["proposal_hash"]
    require(integer(plan["scope"],1,2**63-1) and integer(plan["generation"],1,2**63-1)
            and isinstance(plan["label"],str) and 0<len(plan["label"])<=128
            and base["world_id"]==quorum.world
            and parts(quorum.binding)==(quorum.world.removeprefix("scownerless:"),base["policy_hash"],base["compatibility"])
            and integer(base["sequence"],0,127) and digest(base["head"]) and digest(base["checkpoint"])
            and digest(target) and target!=base["head"] and base["pending_proposal"] is None
            and base["participation_mode"]=="invite_only"
            and base["runtime_activation"] is False and base["live_quorum_verified"] is False,
            "invalid managed certification intent")
    action = proposal["proposal"]
    require(action["sequence"]==base["sequence"]+1
            and bytes(action["parent"]).hex()==base["head"]
            and bytes(action["base_checkpoint"]).hex()==base["checkpoint"]
            and bytes(action["world_id"]).hex()==parts(quorum.binding)[0]
            and bytes(action["policy_hash"]).hex()==base["policy_hash"]
            and bytes(action["compatibility"]).hex()==base["compatibility"]
            and action["base_fence"]==base["reference_fence"]
            and set(action["action"])=={"checkpoint"}
            and digest(bytes(action["action"]["checkpoint"]["commitment"]).hex()),
            "managed certification proposal is not the pinned successor")
    require(len(plan["base_history"]["records"])==base["sequence"],"managed certification base history length differs")
    active = plan["active"]
    slots = [r["slot"] for r in active]
    require(slots==sorted(set(slots)) and 1 in slots and len(slots)>=2
            and plan["voters"]==slots[:2] and 1 in plan["voters"],"invalid managed certification peer set")
    for row in active:
        fields(row,"slot","peer_id","public_key")
        require(integer(row["slot"],0,2) and row["public_key"]==quorum.keys[row["slot"]]
                and {"peer_id":row["peer_id"],"public_key":row["public_key"]} in base["validators"],
                "managed certification peer identity differs")
    if check_active:
        require(slots==quorum.active and not set(slots)&quorum.offline,
                "managed certification configured active peers changed")
    offers = [Offer(**o) for o in plan["offers"]]
    require([o.slot for o in offers]==list(range(4)),"invalid managed certification consent")
    for offer in offers:
        offer.validate()
    require(plan["blocked"]==sorted(set(plan["blocked"])) and set(plan["blocked"])<={o.slot for o in offers if o.opted_in},
            "invalid managed certification blocked slots")
    review_checkpoint(quorum.binding,quorum.mapping,plan["base_checkpoint"],plan["candidate"])
    return target


def expected_head(plan):
    target = plan["proposal"]["proposal_hash"]
    commitment = bytes(plan["proposal"]["proposal"]["action"]["checkpoint"]["commitment"]).hex()
    return {**plan["base"],"head":target,"sequence":plan["base"]["sequence"]+1,"checkpoint":commitment}


def _write(root,name,value):
    path = root/name
    path.write_bytes(canonical(value))
    return path


def _validate_documents(quorum,plan,root,*,check_active=True):
    """Verify the exact pinned base history, checkpoint and successor with Rust."""
    base, target = plan["base"], check_plan(quorum,plan,check_active=check_active)
    base_history = _write(root,"base-history.json",plan["base_history"])
    base_checkpoint = _write(root,"base.bin",plan["base_checkpoint"])
    candidate = _write(root,"candidate.bin",plan["candidate"])
    proposal = _write(root,"proposal.json",plan["proposal"])
    require(quorum.inspect(base_history)==base and quorum.state_hash(base_checkpoint)==base["checkpoint"]
            and quorum.state_hash(candidate)==expected_head(plan)["checkpoint"],
            "managed certification signed base or checkpoint differs")
    reviewed = quorum.auth(None,"propose","--history",base_history,"--expected-world",quorum.world,
                          "--expected-head",base["head"],"--checkpoint",candidate)
    require(canonical(reviewed)==canonical(plan["proposal"]) and reviewed["proposal_hash"]==target,
            "managed certification proposal reconstruction differs")
    return candidate,proposal


def build_plan(quorum,label,scope,generation,base,candidate,offers,blocked):
    history = quorum.auth(1,"export-history","--expected-world",quorum.world)
    with tempfile.TemporaryDirectory(prefix="sc68-cert-intent-") as tmp:
        root = Path(tmp)
        history_file = _write(root,"history.json",history)
        require(quorum.inspect(history_file)==base,"managed certification base changed")
        checkpoint = root/"base.bin"
        copied = quorum.auth(1,"recovery-copy","--expected-world",quorum.world,
                            "--expected-head",base["head"],"--output",checkpoint)
        require(copied["head"]==base["head"] and copied["checkpoint"]==base["checkpoint"]
                and copied["write_authority"] is False and quorum.state_hash(checkpoint)==base["checkpoint"],
                "managed certification base copy differs")
        prior = json.loads(checkpoint.read_bytes())
        require(canonical(prior)==checkpoint.read_bytes(),"noncanonical certified base")
        review_checkpoint(quorum.binding,quorum.mapping,prior,candidate)
        file = _write(root,"candidate.bin",candidate)
        proposal = quorum.auth(None,"propose","--history",history_file,"--expected-world",quorum.world,
                              "--expected-head",base["head"],"--checkpoint",file)
        plan = {"scope":scope,"generation":generation,"label":label,"base":base,
                "base_history":history,"base_checkpoint":prior,"candidate":candidate,"proposal":proposal,
                "active":[{"slot":s,"peer_id":quorum.peer_id(s),"public_key":quorum.keys[s]} for s in quorum.active],
                "voters":quorum.active[:2],"offers":offers,"blocked":blocked}
        check_plan(quorum,plan)
        require(quorum.state_hash(file)==expected_head(plan)["checkpoint"],"managed certification target commitment differs")
        # Normalize tuples in the in-memory simulation to their exact
        # durable JSON representation before comparing an intent.
        return json.loads(canonical(plan))


def check_completion(quorum,plan,completion,*,verify_signatures):
    fields(completion,"head","history","sync")
    head = expected_head(plan)
    require(canonical(completion["head"])==canonical(head),"managed certification completed head differs")
    history = completion["history"]
    require(history["creation"]==plan["base_history"]["creation"]
            and len(history["records"])==head["sequence"]
            and canonical([r["proposal"] for r in history["records"][:-1]])==
                canonical([r["proposal"] for r in plan["base_history"]["records"]])
            and canonical(history["records"][-1]["proposal"])==canonical(plan["proposal"]["proposal"]),
            "managed certification completed history differs")
    expected = [r for r in plan["active"] if r["slot"]!=1]
    sync = completion["sync"]
    require([row["recipient_slot"] for row in sync]==[r["slot"] for r in expected],
            "managed certification sync recipient set differs")
    for row,recipient in zip(sync,expected):
        fields(row,"recipient_slot","listener","result")
        listener,result = row["listener"],row["result"]
        require(listener["classification"]=="ownerless_reference_sync_listener"
                and listener["world_id"]==quorum.world and listener["head"]==head["head"]
                and listener["validator"]==recipient["peer_id"] and listener["runtime_activation"] is False
                and result["classification"]=="ownerless_reference_checkpoint_synced"
                and all(result[k] is False for k in ("live_quorum_verified","runtime_activation","write_authority",
                    "canonical_authority","player_admission","worker_admission")),
                "managed certification sync observation differs")
        receipt = result["receipt"]
        fields(receipt,"body","signature")
        expected_body = {"world":list(bytes.fromhex(parts(quorum.binding)[0])),
            "base":list(bytes.fromhex(plan["base"]["head"])),"head":list(bytes.fromhex(head["head"])),
            "sequence":head["sequence"],"fence":head["reference_fence"],
            "checkpoint":list(bytes.fromhex(head["checkpoint"])),
            "recipient":list(bytes.fromhex(recipient["peer_id"].removeprefix("scpeer:")))}
        require(canonical(receipt["body"])==canonical(expected_body),"managed certification sync receipt differs")
    if verify_signatures:
        verify_sync_signatures({"schema":3,"participant_keys":quorum.keys,
                               "certified_cuts":[{"network":{"sync":sync}}]})


def verify_completed_documents(quorum,plan,completion):
    """Reopen verifies stored signed documents, not merely their shape."""
    with tempfile.TemporaryDirectory(prefix="sc68-cert-audit-") as tmp:
        root = Path(tmp)
        _validate_documents(quorum,plan,root,check_active=False)
        require(quorum.inspect(_write(root,"completed.json",completion["history"]))==expected_head(plan),
                "managed certification stored signed history differs")


def _status(quorum,plan,row):
    status = quorum.auth(row["slot"],"status","--expected-world",quorum.world)
    base,target = plan["base"],expected_head(plan)
    match = base if status["head"]==base["head"] else target
    pending = status["pending_proposal"]
    require(canonical(status)==canonical({**match,"local_identity":row["peer_id"],
                    "local_validator":True,"pending_proposal":pending})
            and (pending in (None,target["head"]) if match==base else pending is None),
            "managed certification replica is not the exact base/target or has conflicting vote")
    return status


def check_replicas(quorum,plan,head):
    check_plan(quorum,plan)
    require(head==expected_head(plan),"managed certification target differs")
    for row in plan["active"]:
        status = _status(quorum,plan,row)
        require(status["head"]==head["head"] and status["pending_proposal"] is None,
                "managed certification replica not reconciled")
    # Validate the actual accepted history, not just a report or hash.
    history = quorum.auth(1,"export-history","--expected-world",quorum.world)
    with tempfile.TemporaryDirectory(prefix="sc68-cert-complete-") as tmp:
        root = Path(tmp)
        require(quorum.inspect(_write(root,"history.json",history))==head
                and canonical(history["records"][-1]["proposal"])==canonical(plan["proposal"]["proposal"]),
                "managed certification signed target differs")
        file = root/"checkpoint.bin"
        quorum.auth(1,"recovery-copy","--expected-world",quorum.world,"--expected-head",head["head"],"--output",file)
        require(file.read_bytes()==canonical(plan["candidate"]),"managed certification accepted bytes differ")


def completion_from_quorum(quorum,plan,accepted):
    row = quorum.records[-1]
    require(row["head"]==accepted and row["label"]==plan["label"],"managed certification record differs")
    return {"head":accepted,"history":json.loads(quorum.history.read_bytes()),"sync":row["network"]["sync"]}


def _review_local_base(quorum,plan,row,root,candidate):
    slot = row["slot"]
    base = plan["base"]
    file = root/f"review-{slot}.bin"
    copied = quorum.auth(slot,"recovery-copy","--expected-world",quorum.world,
                        "--expected-head",base["head"],"--output",file)
    require(copied["checkpoint"]==base["checkpoint"] and copied["write_authority"] is False
            and file.read_bytes()==canonical(plan["base_checkpoint"]),"managed retry base bytes differ")
    review_checkpoint(quorum.binding,quorum.mapping,json.loads(file.read_bytes()),plan["candidate"])
    history = quorum.auth(slot,"export-history","--expected-world",quorum.world)
    history_file = _write(root,f"review-{slot}.json",history)
    require(quorum.inspect(history_file)==base,"managed retry base history differs")
    proposal = quorum.auth(None,"propose","--history",history_file,"--expected-world",quorum.world,
                          "--expected-head",base["head"],"--checkpoint",candidate)
    require(canonical(proposal)==canonical(plan["proposal"]),"managed retry review differs")


def _adopt(quorum,plan,completion):
    """Rebuild volatile fixture pointers only after durable reconciliation."""
    head = completion["head"]
    name = "reconciled-"+str(head["sequence"])+"-"+secrets.token_hex(8)
    history = quorum.write_json(name+".json",completion["history"])
    checkpoint = quorum.write(name+".bin",canonical(plan["candidate"]))
    quorum.head,quorum.history,quorum.checkpoint = head,history,checkpoint


def resume_managed_certification(ledger,quorum,*,expected_target,recover_only=False,before_sync=None):
    """Resume an explicit durable intent with no new candidate or vote reset.

    Changed local consent (including blocks) disables new voting. Only
    already-signed evidence may complete that exact intent. A locally
    accepted certificate is immutable even if consent later changes.
    """
    require(type(recover_only) is bool,"invalid managed certification recovery mode")
    plan,done,unchanged = ledger.certification_intent(expected_target)
    base,target = plan["base"],expected_head(plan)
    require(quorum.head in (base,target),"managed certification controller head is not pinned base or target")
    if done is not None:
        check_completion(quorum,plan,done,verify_signatures=True)
        verify_completed_documents(quorum,plan,done)
        check_replicas(quorum,plan,target)
        _adopt(quorum,plan,done)
        return {"mode":"already_reconciled","head":target,"completion":done,"before":[],"after":[]}
    active = {row["slot"]:row for row in plan["active"]}
    remote, = [s for s in plan["voters"] if s!=1]
    with tempfile.TemporaryDirectory(prefix="sc68-cert-resume-") as tmp:
        root = Path(tmp)
        candidate,proposal = _validate_documents(quorum,plan,root)
        review = ["--expected-world",quorum.world,"--expected-head",base["head"],
                  "--proposal",proposal,"--expected-proposal",expected_target,"--confirm"]
        before = [_status(quorum,plan,row) for row in plan["active"]]
        statuses = dict(zip(active,before))
        if statuses[1]["head"]==expected_target:
            mode = "accepted_certificate"
            history = quorum.auth(1,"export-history","--expected-world",quorum.world)
        else:
            # A peer already at target can provide its complete certificate.
            # Otherwise use only the originally reviewed remote voter.
            certified = [s for s in active if s!=1 and statuses[s]["head"]==expected_target]
            # Recheck the local consent immediately before initiating voting.
            _,_,still_unchanged = ledger.certification_intent(expected_target)
            if certified or recover_only or not unchanged or not still_unchanged:
                mode = "recover_existing_evidence"
                provider = certified[0] if certified else remote
                require(statuses[provider]["head"]==expected_target or
                        statuses[provider]["pending_proposal"]==expected_target,
                        "no matching durable remote recovery evidence; new votes forbidden")
                with quorum.listener(provider,"ownerless_reference_recovery_listener","serve-recovery",*review,
                        "--from",active[1]["peer_id"],"--bind","127.0.0.1:0",pins={"proposal":expected_target}) as ready:
                    history = quorum.auth(1,"decision","recover",*review,
                                         "--peer",active[provider]["peer_id"]+"@"+ready["address"])
            else:
                mode = "retry_exact_vote"
                for slot in plan["voters"]:
                    _review_local_base(quorum,plan,active[slot],root,candidate)
                _,_,still_unchanged = ledger.certification_intent(expected_target)
                require(still_unchanged,"managed consent changed before retry; new votes forbidden")
                with quorum.listener(remote,"ownerless_reference_decision_listener","serve-vote",*review,
                        "--from",active[1]["peer_id"],"--bind","127.0.0.1:0",pins={"proposal":expected_target}) as ready:
                    pinned,done,still_unchanged = ledger.certification_intent(expected_target)
                    require(pinned==plan and done is None and still_unchanged,
                            "managed consent or intent changed before retry; new votes forbidden")
                    history = quorum.auth(1,"decision","collect",*review,"--checkpoint",candidate,
                                         "--peer",active[remote]["peer_id"]+"@"+ready["address"])
        require(quorum.inspect(_write(root,"accepted.json",history))==target
                and canonical(history["records"][-1]["proposal"])==canonical(plan["proposal"]["proposal"]),
                "managed recovered certificate differs")
        # Even on a resumed already-accepted target, the intent remains
        # unresolved until all configured active peers sign exact receipts.
        if before_sync is not None:
            before_sync()
        sync_flags = ["--expected-world",quorum.world,"--expected-base",base["head"],
                      "--expected-head",expected_target,"--confirm"]
        sync = []
        for slot in active:
            if slot==1:
                continue
            with quorum.listener(slot,"ownerless_reference_sync_listener","serve-sync",*sync_flags,
                    "--from",active[1]["peer_id"],"--bind","127.0.0.1:0",pins={"head":expected_target}) as ready:
                result = quorum.auth(1,"decision","push-sync",*sync_flags,
                                     "--peer",active[slot]["peer_id"]+"@"+ready["address"])
            sync.append({"recipient_slot":slot,"listener":ready,"result":result})
        completion = {"head":target,"history":history,"sync":sync}
        ledger.finish_certification(plan,completion)
        after = [_status(quorum,plan,row) for row in plan["active"]]
        _adopt(quorum,plan,completion)
        return {"mode":mode,"head":target,"completion":completion,"before":before,"after":after}
