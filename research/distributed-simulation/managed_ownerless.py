"""#68 consent-gated, single-cut ownerless reference contribution lifecycle.

The trusted local controller supplies actual resource observations and
cleanup proof. A signed role proof is not a lease or real game admission.
Each scope executes at most one locally staged tick against a pinned
accepted cut. Any restart requires fencing and proven cleanup; no
automatic reuse of the old proof, process or resource sample.
"""
from dataclasses import asdict
import json
import secrets
import tempfile
import time
from pathlib import Path

from combined_ownerless_lab import (CertifiedClientGate, FALSE_FLAGS, FULL, _restoration_status,
                                    combined_authority, committed_snapshot, sha)
from ownerless_permission import parts
from recovery_runtime import RecoveryAuthority
from region_runtime import Rejected, canonical, digest, fields, integer, require
from worker_placement import (MAX_SAMPLE_AGE_NS, Offer, Sample, assess, evaluate, region_load)


class ScopeFault(Rejected):
    def __init__(self,reason,scope,generation):
        super().__init__(reason)
        self.scope,self.generation = scope,generation


def managed_authority(quorum, slot_subjects):
    """Fixed worker-slot identity mapping, separately pinned database type."""
    mapping = tuple(slot_subjects)
    require(len(mapping)==4 and all(s is None or integer(s,0,len(quorum.keys)-1) for s in mapping)
            and len({s for s in mapping if s is not None})==sum(s is not None for s in mapping),
            "invalid managed worker mapping")
    keymap = [None if s is None else quorum.keys[s] for s in mapping]
    require(all(k is None or digest(k) for k in keymap),"invalid managed worker identity")
    binding = quorum.binding
    Combined = combined_authority(binding)

    class Managed(Combined, RecoveryAuthority):
        app_id = 0x53433639
        ruleset = "swarmcraft-reference-managed-ownerless/v1/"+binding

        @classmethod
        def _create_extra(cls,db,state):
            super()._create_extra(db,state)
            db.execute("CREATE TABLE managed_config(singleton INTEGER PRIMARY KEY CHECK(singleton=1), worker_keys BLOB NOT NULL)")
            db.execute("INSERT INTO managed_config VALUES(1,?)",(canonical(keymap),))
            db.execute("""CREATE TABLE managed_scopes(scope INTEGER PRIMARY KEY, owner TEXT NOT NULL,
                        base BLOB NOT NULL, offers BLOB NOT NULL, proofs BLOB NOT NULL,
                        samples BLOB, staged BLOB, released INTEGER NOT NULL CHECK(released IN (0,1)))""")

        def __init__(self,path,*,read_only=False):
            self._owner = secrets.token_hex(32)
            self._guard = None
            super().__init__(path,read_only=read_only)
            try:
                with self.transaction():
                    require(self.db.execute("SELECT worker_keys FROM managed_config WHERE singleton=1").fetchone()
                            ==(canonical(keymap),),"managed worker mapping differs")
                    state = self._contribution(self._read())
                    if state["scope"]:
                        self._scope_record(state)
            except BaseException:
                self.close()
                raise

        def _scope_record(self,state):
            row = self.db.execute("SELECT owner,base,offers,proofs,samples,staged,released FROM managed_scopes WHERE scope=?",
                                  (state["scope"],)).fetchone()
            require(row is not None,"missing managed scope authorization")
            owner,base,offers,proofs,samples,staged,released = row
            base,offers,proofs = map(json.loads,(base,offers,proofs))
            samples = None if samples is None else json.loads(samples)
            staged = None if staged is None else json.loads(staged)
            require(digest(owner) and released in (0,1)
                    and bool(released)==(state["phase"]=="paused"),"managed scope release mismatch")
            fields(base,"head","tick","root","cut")
            require(integer(base["tick"],0,999999) and digest(base["root"]) and digest(base["cut"])
                    and base["head"]["world_id"]==quorum.world
                    and parts(binding)==(parts(quorum.binding)[0],base["head"]["policy_hash"],base["head"]["compatibility"]),
                    "invalid managed scope base")
            require(len(offers)==4 and [o["slot"] for o in offers]==list(range(4)),
                    "invalid managed scope offers")
            for o in offers:
                Offer(**o).validate()
            return {"owner":owner,"base":base,"offers":offers,"proofs":proofs,
                    "samples":samples,"staged":staged,"released":bool(released)}

        def scope_record(self):
            with self.transaction():
                state = self._contribution(self._read())
                require(state["scope"]>0,"no managed scope")
                result = self._scope_record(state)
                return {k:v for k,v in result.items() if k!="owner"}

        def _head_check(self,head):
            # The caller pins a reviewed local certified journal; a
            # different accepted head or pending vote stops the scope.
            require(head==quorum.head,"managed certified head changed")
            verifiers = [v["peer_id"] for v in head["validators"] if v["public_key"]==quorum.keys[1]]
            require(len(verifiers)==1,"managed verifier not current")
            _restoration_status(quorum,head,verifiers[0])

        def _before_probe(self,current,state,slots):
            require(current["tick"]<999999 and current["players"]==[0,1],"invalid managed tick or roster")
            head = dict(quorum.head)
            self._head_check(head)
            checkpoint = canonical(committed_snapshot(self,quorum.mapping,current))
            with tempfile.TemporaryDirectory(prefix="sc68-managed-base-") as tmp:
                path = Path(tmp)/"accepted.bin"
                copy = quorum.auth(1,"recovery-copy","--expected-world",quorum.world,
                                   "--expected-head",head["head"],"--output",path)
                require(copy["checkpoint"]==head["checkpoint"] and copy["head"]==head["head"]
                        and copy["write_authority"] is False and path.read_bytes()==checkpoint
                        and quorum.state_hash(path)==head["checkpoint"],"managed base is not the certified checkpoint")
            proofs = []
            for slot in slots:
                subject = mapping[slot]
                require(subject is not None,"worker slot has no reviewed identity")
                offer = state["offers"][slot]
                # Quorum.role is the existing explicit test-fixture
                # volunteer confirmation + durable one-use verifier.
                proof = quorum.role("managed-"+secrets.token_hex(12),subject,offer.cpu_millicores)
                result = proof["result"]
                require(proof["subject_slot"]==subject and result["head"]==head["head"]
                        and result["role"]=="reference_volunteer"
                        and result["offered_budget"]["memory_mib"]>=offer.process_memory_mib
                        and result["offered_budget"]["cpu_millicores"]==offer.cpu_millicores
                        and all(result[k] is False for k in FALSE_FLAGS),"managed volunteer proof differs")
                proofs.append({"slot":slot,"revision":offer.revision,"proof":proof})
            self._head_check(head)
            base = {"head":head,"tick":current["tick"],"root":current["runtime_root"],"cut":current["cut_root"]}
            self.db.execute("INSERT INTO managed_scopes VALUES(?,?,?,?,?,NULL,NULL,0)",
                            (state["scope"],self._owner,canonical(base),
                             canonical([asdict(o) for o in state["offers"]]),canonical(proofs)))
            self._guard = None

        def _active_record(self,current,state):
            record = self._scope_record(state)
            require(record["owner"]==self._owner,"managed scope belongs to retired controller; fence and clean up")
            require(record["staged"] is None and not record["released"],"managed scope already staged or released")
            base = record["base"]
            require((current["tick"],current["runtime_root"],current["cut_root"])==
                    (base["tick"],base["root"],base["cut"])
                    and record["offers"]==[asdict(o) for o in state["offers"]],
                    "managed scope base or consent changed")
            require([p["slot"] for p in record["proofs"]]==state["probes"]
                    and all(p["revision"]==state["offers"][p["slot"]].revision for p in record["proofs"]),
                    "managed scope volunteer proof mismatch")
            try:
                self._head_check(base["head"])
            except Exception as error:
                raise ScopeFault(str(error),state["scope"],current["generation"]) from error
            return record

        def _fenced_operation(self,operation,*args,**kwargs):
            try:
                return operation(*args,**kwargs)
            except ScopeFault as error:
                # The failed transaction has rolled back. Invalidate this
                # controller even if persisting the separate drain fails.
                # Do not let a stale contender fence somebody else's scope.
                self._guard = None
                self._owner = secrets.token_hex(32)
                status = self.status()
                if (status["scope"],status["generation"])==(error.scope,error.generation) and status["phase"] not in ("paused","draining"):
                    self.fence(error.scope,error.generation,"policy")
                raise

        def admit(self,*args,**kwargs):
            return self._fenced_operation(super().admit,*args,**kwargs)

        def claim(self,*args,**kwargs):
            return self._fenced_operation(super().claim,*args,**kwargs)

        def seal_next(self,*args,**kwargs):
            return self._fenced_operation(super().seal_next,*args,**kwargs)

        def commit(self,*args,**kwargs):
            return self._fenced_operation(super().commit,*args,**kwargs)

        def _before_admit(self,current,state,samples,now_ns):
            record = self._active_record(current,state)
            # The existing planner checks sample age at the exact supplied
            # observation time. Also require age at the actual current
            # monotonic clock; no caller-supplied historic time bypass.
            now = time.monotonic_ns()
            require(integer(now_ns,0,now) and all(0<=now-s.observed_ns<=MAX_SAMPLE_AGE_NS for s in samples.values()),
                    "managed worker measurement expired")
            require(record["samples"] is None,"managed scope already measured")
            self.db.execute("UPDATE managed_scopes SET samples=? WHERE scope=?",
                            (canonical({str(k):asdict(v) for k,v in sorted(samples.items())}),state["scope"]))

        def attach_resource_guard(self,scope,guard):
            """Trusted in-process kernel/ownership observation callback.

            No callback survives restart. No worker can invoke this local
            API. It must raise on reduced capacity or ownership loss.
            """
            require(callable(guard) and self._guard is None,"invalid or reused managed resource guard")
            with self.transaction():
                current = self._read()
                state = self._contribution(current)
                require(state["phase"]=="ready" and state["scope"]==scope,"managed scope not ready")
                self._active_record(current,state)
            self._guard = (scope,guard)

        def _check_resources(self,current,state,record,candidate):
            require(self._guard is not None and self._guard[0]==state["scope"],"missing managed resource guard")
            try:
                require(record["samples"] is not None,"missing managed measurements")
                samples = {int(k):Sample(**v) for k,v in record["samples"].items()}
                selected = [s for s,_ in state["assignment"]]
                now = time.monotonic_ns()
                require(selected and all(assess(state["offers"][s],samples[s],now)=="eligible" for s in selected),
                        "managed worker measurement expired or ineligible")
                owner = [next(s for s,g in zip(selected,current["grants"]) if r in g["regions"]) for r in FULL]
                require(evaluate(owner,region_load(candidate),samples) is not None,"managed load exceeds measured placement")
                require(self._guard[1](candidate,tuple(selected)) is True,"managed resource guard refused")
            except Exception as error:
                raise ScopeFault(str(error),state["scope"],current["generation"]) from error

        def _before_claim(self,current):
            state = self._contribution(current)
            require(state["phase"]=="ready","no admitted contribution scope")
            record = self._active_record(current,state)
            self._check_resources(current,state,record,current["state"])
            super()._before_claim(current)

        def _before_seal(self,current):
            state = self._running(current)
            record = self._active_record(current,state)
            self._check_resources(current,state,record,current["state"])

        def _record_candidate(self,current,state,candidate,cut):
            lifecycle = self._running(current)
            record = self._active_record(current,lifecycle)
            self._check_resources(current,lifecycle,record,state)
            super()._record_candidate(current,state,candidate,cut)
            # Staged receipt and one-tick fence commit in the same FULL
            # transaction. Nothing can seal or commit a second tick.
            staged = {"tick":current["tick"]+1,"root":candidate,"cut":cut,"generation":current["generation"]}
            self.db.execute("UPDATE managed_scopes SET staged=? WHERE scope=?",
                            (canonical(staged),lifecycle["scope"]))
            lifecycle["phase"]="draining"
            self._write(current,lifecycle,"staged_cut_fenced")

        def _write(self,current,state,event):
            super()._write(current,state,event)
            if event=="released":
                self.db.execute("UPDATE managed_scopes SET released=1 WHERE scope=?",(state["scope"],))
                self._guard = None

        def certification_candidate(self):
            """Read-only handoff of exactly one staged cut after cleanup."""
            with self.transaction():
                current = self._read()
                state = self._contribution(current)
                require(state["phase"]=="paused","managed scope cleanup not released")
                record = self._scope_record(state)
                require(record["released"] and record["staged"] is not None,"no released managed staged cut")
                require(record["offers"]==[asdict(o) for o in state["offers"]],
                        "managed consent changed before certification")
                require(record["staged"]=={"tick":current["tick"],"root":current["runtime_root"],
                            "cut":current["cut_root"],"generation":current["generation"]},
                        "managed staged cut differs")
                self._head_check(record["base"]["head"])
                return committed_snapshot(self,quorum.mapping,current)

    return Managed


def certify_managed_checkpoint(ledger,quorum,gate,label,*,before_sync=None):
    """No offline fallback; release only the audited, drained staged cut."""
    require(isinstance(gate,CertifiedClientGate) and gate.ledger is ledger and gate.quorum is quorum,
            "managed publication gate differs")
    candidate = ledger.certification_candidate()
    accepted = quorum.certify_network_checkpoint(label,candidate,before_sync=before_sync)
    return accepted,gate.publish(accepted)
