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
                                    _restore_reference_staging, combined_authority, committed_snapshot,
                                    restoration_source_pin, sha)
from ownerless_permission import parts
from recovery_runtime import RecoveryAuthority
from region_runtime import Rejected, canonical, digest, fields, integer, require
from transition_runtime import checked_command
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
        # Research-only schema. Do not manufacture authorization or
        # dissemination provenance for older managed databases.
        schema = 3
        ruleset = "swarmcraft-reference-managed-ownerless/v1/"+binding

        @classmethod
        def _create_extra(cls,db,state):
            super()._create_extra(db,state)
            db.execute("CREATE TABLE managed_config(singleton INTEGER PRIMARY KEY CHECK(singleton=1), worker_keys BLOB NOT NULL)")
            db.execute("INSERT INTO managed_config VALUES(1,?)",(canonical(keymap),))
            db.execute("""CREATE TABLE managed_scopes(scope INTEGER PRIMARY KEY, owner TEXT NOT NULL,
                        base BLOB NOT NULL, offers BLOB NOT NULL, proofs BLOB NOT NULL,
                        samples BLOB, staged BLOB, released INTEGER NOT NULL CHECK(released IN (0,1)))""")
            db.execute("""CREATE TABLE command_authorizations(request_id TEXT PRIMARY KEY,
                        player INTEGER NOT NULL, request_hash TEXT NOT NULL,
                        sequence INTEGER NOT NULL, head TEXT NOT NULL, player_key TEXT NOT NULL)""")
            db.execute("""CREATE TABLE managed_restores(generation INTEGER PRIMARY KEY, record BLOB NOT NULL)""")
            db.execute("""CREATE TABLE abandoned_commands(request_id TEXT PRIMARY KEY, generation INTEGER NOT NULL)""")
            db.execute("""CREATE TABLE managed_certifications(scope INTEGER PRIMARY KEY,
                        plan BLOB NOT NULL, completion BLOB)""")

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
                    self._audit_authorizations()
                    self._audit_certifications()
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

        def _audit_authorizations(self):
            rows = self.db.execute("""SELECT c.player,c.request,c.request_id,a.player,a.request_hash,
                a.sequence,a.head,a.player_key FROM commands c
                LEFT JOIN command_authorizations a ON a.request_id=c.request_id
                ORDER BY c.tick,c.player""").fetchall()
            require(len(rows)==self.db.execute("SELECT COUNT(*) FROM command_authorizations").fetchone()[0],
                    "orphaned managed command authorization")
            for player,raw,request_id,subject,request_hash,sequence,head,key in rows:
                require(player in (0,1) and player==subject and json.loads(raw)["id"]==request_id
                        and request_hash==sha(raw) and integer(sequence,1,128) and digest(head)
                        and key==quorum.mapping[player],"missing or invalid managed command authorization")
            for request_id,generation in self.db.execute("SELECT request_id,generation FROM abandoned_commands"):
                require(digest(request_id) and self.db.execute("SELECT 1 FROM managed_restores WHERE generation=?",
                            (generation,)).fetchone() is not None
                        and self.db.execute("SELECT 1 FROM commands WHERE request_id=?",(request_id,)).fetchone() is None,
                        "invalid abandoned managed command")

        def _invitation_history(self):
            """Use the actual local Rust-verified history, not a cached role report."""
            head = dict(quorum.head)
            self._head_check(head)
            history = quorum.auth(1,"export-history","--expected-world",quorum.world)
            with tempfile.TemporaryDirectory(prefix="sc68-managed-invites-") as tmp:
                path = Path(tmp)/"history.json"
                path.write_bytes(canonical(history))
                require(quorum.inspect(path)==head,"managed invitation history pin changed")
            self._head_check(head)
            records = history["records"]
            require(len(records)==head["sequence"]<=128,"invalid managed invitation history")
            # A next signed proposal commits its predecessor's hash. The
            # last hash comes from the independently verified head.
            hashes = [bytes(row["proposal"]["parent"]).hex() for row in records]+[head["head"]]
            return head,records,hashes

        def _invited_at(self,records,key,sequence):
            invited = False
            for row in records[:sequence]:
                action = row["proposal"]["action"]
                role = action.get("set_player") if isinstance(action,dict) else None
                if role is not None and bytes(role["player"]["public_key"]).hex()==key:
                    invited = role["allowed"]
            return invited

        def _authorized_command(self,player,request,history,*,new=False,committed=False):
            head,records,hashes = history
            key = quorum.mapping[player]
            require(self._invited_at(records,key,head["sequence"]),"managed player has no current private invitation")
            raw = canonical(request)
            stored = self.db.execute("""SELECT player,request_hash,sequence,head,player_key
                FROM command_authorizations WHERE request_id=?""",(request["id"],)).fetchone()
            if new:
                require(stored is None,"orphaned managed command authorization")
                self.db.execute("INSERT INTO command_authorizations VALUES(?,?,?,?,?,?)",
                                (request["id"],player,sha(raw),head["sequence"],head["head"],key))
                return
            require(stored is not None,"missing managed command authorization")
            subject,request_hash,sequence,anchor,stored_key = stored
            require(subject==player and request_hash==sha(raw) and stored_key==key
                    and integer(sequence,1,head["sequence"]) and hashes[sequence]==anchor
                    and self._invited_at(records,key,sequence),"managed command invitation anchor differs")
            if not committed:
                # Re-invitation must not resurrect an old sealed command.
                # A revoked pending command requires explicit abandonment
                # into a separate certified restoration, not an in-place edit.
                for row in records[sequence:]:
                    action = row["proposal"]["action"]
                    role = action.get("set_player") if isinstance(action,dict) else None
                    require(not (role is not None and role["allowed"] is False
                                 and bytes(role["player"]["public_key"]).hex()==key),
                            "managed command invitation was revoked after queueing")

        def _submit_locked(self,current,player,request,*,pending_only=False):
            # Validate the canonical command/duplicate semantics first;
            # both the queue and its authorization row share the same txn.
            checked_command(player,request,current["world"],current["players"])
            self._before_certified_access(current,quorum.head)
            require(self.db.execute("SELECT 1 FROM abandoned_commands WHERE request_id=?",
                                   (request["id"],)).fetchone() is None,"abandoned managed command ID")
            existing = self.db.execute("SELECT response FROM commands WHERE request_id=?",
                                       (request["id"],)).fetchone()
            result = super()._submit_locked(current,player,request,pending_only=pending_only)
            history = self._invitation_history()
            require(quorum.state_hash(quorum.checkpoint)==history[0]["checkpoint"]
                    and quorum.checkpoint.read_bytes()==canonical(committed_snapshot(self,quorum.mapping,current)),
                    "managed command base is not the certified checkpoint")
            self._authorized_command(player,request,history,
                                     new=existing is None,committed=result["op"]=="committed")
            return result

        def _check_pending(self,current,commands=None):
            if commands is None:
                commands,_ = self._commands(current,current["tick"]+1,current["cut_root"])
            if commands:
                history = self._invitation_history()
                for player,(request,_) in commands.items():
                    self._authorized_command(player,request,history)

        def _before_probe(self,current,state,slots):
            self._before_certified_access(current,quorum.head)
            require(current["tick"]<999999 and current["players"]==[0,1],"invalid managed tick or roster")
            head = dict(quorum.head)
            self._head_check(head)
            self._check_pending(current)
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

        def _before_sealed_inputs(self,current,commands):
            state = self._running(current)
            try:
                self._check_pending(current,commands)
            except Exception as error:
                raise ScopeFault(str(error),state["scope"],current["generation"]) from error

        def _record_candidate(self,current,state,candidate,cut):
            lifecycle = self._running(current)
            record = self._active_record(current,lifecycle)
            self._check_resources(current,lifecycle,record,state)
            self._before_sealed_inputs(current,self._commands(current,current["tick"]+1,current["cut_root"])[0])
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

        def _require_reconciled(self):
            require(self.db.execute("SELECT 1 FROM managed_certifications WHERE completion IS NULL").fetchone()
                    is None,"managed certification reconciliation incomplete")

        def _before_certified_access(self,current,head):
            self._require_reconciled()
            if current["tick"]==0:
                return
            # An accepted Rust checkpoint alone is insufficient for the
            # managed path. Require the exact completed local intent
            # even if a caller obtained a certificate outside this path.
            candidate = canonical(committed_snapshot(self,quorum.mapping,current))
            matches = []
            for raw,done in self.db.execute("SELECT plan,completion FROM managed_certifications WHERE completion IS NOT NULL"):
                plan,completion = json.loads(raw),json.loads(done)
                if canonical(plan["candidate"])==candidate and completion["head"]["checkpoint"]==head["checkpoint"]:
                    matches.append(plan)
            require(len(matches)==1,"managed certified cut lacks reconciled intent")

        def _staged_candidate(self,current,state):
            require(state["phase"]=="paused","managed scope cleanup not released")
            record = self._scope_record(state)
            require(record["released"] and record["staged"] is not None,"no released managed staged cut")
            require(record["staged"]=={"tick":current["tick"],"root":current["runtime_root"],
                        "cut":current["cut_root"],"generation":current["generation"]},
                    "managed staged cut differs")
            return record,committed_snapshot(self,quorum.mapping,current)

        def _certification_candidate_locked(self,current,state):
            self._require_reconciled()
            require(state["phase"]=="paused","managed scope cleanup not released")
            record = self._scope_record(state)
            require(record["released"] and record["staged"] is not None,"no released managed staged cut")
            require(record["offers"]==[asdict(o) for o in state["offers"]],
                    "managed consent changed before certification")
            require(not set(state["blocked"]) & {p["slot"] for p in record["proofs"]},
                    "managed blocked worker before certification")
            record,candidate = self._staged_candidate(current,state)
            self._head_check(record["base"]["head"])
            parent = self.db.execute("SELECT cut_root FROM commits WHERE tick=?",(current["tick"]-1,)).fetchone()[0]
            commands,_ = self._commands(current,current["tick"],parent)
            self._check_pending(current,commands)
            return record,candidate

        def certification_candidate(self):
            """Read-only handoff of exactly one staged cut after cleanup."""
            with self.transaction():
                current = self._read()
                return self._certification_candidate_locked(current,self._contribution(current))[1]

        def _audit_certifications(self):
            from managed_certification import check_plan, check_completion, verify_completed_documents
            unresolved = []
            for scope,raw,done in self.db.execute("SELECT * FROM managed_certifications ORDER BY scope"):
                plan = json.loads(raw)
                require(raw==canonical(plan) and plan["scope"]==scope
                        and self.db.execute("SELECT 1 FROM managed_scopes WHERE scope=?",(scope,)).fetchone(),
                        "invalid managed certification record")
                check_plan(quorum,plan,check_active=False)
                if done is None:
                    unresolved.append(plan)
                else:
                    completion = json.loads(done)
                    require(done==canonical(completion),"invalid managed certification completion")
                    check_completion(quorum,plan,completion,verify_signatures=True)
                    verify_completed_documents(quorum,plan,completion)
            require(len(unresolved)<=1,"multiple unresolved managed certifications")
            if unresolved:
                current = self._read()
                state = self._contribution(current)
                self._check_intent_stage(current,state,unresolved[0])

        def _check_intent_stage(self,current,state,plan):
            record,candidate = self._staged_candidate(current,state)
            require(plan["scope"]==state["scope"] and plan["generation"]==current["generation"]
                    and plan["base"]==record["base"]["head"]
                    and canonical(plan["candidate"])==canonical(candidate),
                    "managed certification intent differs from staged cut")
            return record

        def prepare_certification(self,label):
            from managed_certification import build_plan
            with self.transaction(write=True):
                current = self._read()
                state = self._contribution(current)
                record,candidate = self._certification_candidate_locked(current,state)
                require(self.db.execute("SELECT 1 FROM managed_certifications WHERE scope=?",(state["scope"],)).fetchone()
                        is None,"managed scope already has certification intent")
                plan = build_plan(quorum,label,state["scope"],current["generation"],record["base"]["head"],
                                  candidate,[asdict(o) for o in state["offers"]],state["blocked"])
                self._head_check(plan["base"])
                self.db.execute("INSERT INTO managed_certifications VALUES(?,?,NULL)",(state["scope"],canonical(plan)))
                return plan

        def certification_intent(self,expected_target):
            from managed_certification import check_plan
            with self.transaction():
                current = self._read()
                state = self._contribution(current)
                row = self.db.execute("SELECT plan,completion FROM managed_certifications WHERE scope=?",
                                      (state["scope"],)).fetchone()
                require(row is not None,"no managed certification intent")
                plan = json.loads(row[0])
                check_plan(quorum,plan)
                require(plan["proposal"]["proposal_hash"]==expected_target,"managed certification target pin differs")
                self._check_intent_stage(current,state,plan)
                unchanged = (plan["offers"]==[asdict(o) for o in state["offers"]] and
                             plan["blocked"]==state["blocked"])
                return plan,None if row[1] is None else json.loads(row[1]),unchanged

        def finish_certification(self,plan,completion):
            from managed_certification import check_completion, check_replicas
            with self.transaction(write=True):
                current = self._read()
                state = self._contribution(current)
                self._check_intent_stage(current,state,plan)
                row = self.db.execute("SELECT plan,completion FROM managed_certifications WHERE scope=?",
                                      (state["scope"],)).fetchone()
                require(row is not None and row[0]==canonical(plan),"managed certification intent changed")
                check_completion(quorum,plan,completion,verify_signatures=True)
                check_replicas(quorum,plan,completion["head"])
                if row[1] is None:
                    self.db.execute("UPDATE managed_certifications SET completion=? WHERE scope=?",
                                    (canonical(completion),state["scope"]))
                else:
                    require(row[1]==canonical(completion),"managed certification completion differs")


        def _restoration_pin_locked(self,current):
            state = self._contribution(current)
            self._audit_authorizations()
            # Bind the exact unaccepted journal and all consent metadata,
            # not just the world tick: withdrawal and pending commands
            # can change without advancing the simulation.
            metadata = {}
            for table,order in (("managed_config","singleton"),("contribution","singleton"),
                                ("lifecycle","seq"),("managed_scopes","scope"),
                                ("command_authorizations","request_id"),
                                ("managed_restores","generation"),("abandoned_commands","request_id"),
                                ("managed_certifications","scope")):
                metadata[table] = [[v.hex() if isinstance(v,bytes) else v for v in row]
                                   for row in self.db.execute(f"SELECT * FROM {table} ORDER BY {order}")]
            return {"source":restoration_source_pin(current),
                    "phase":state["phase"],"scope":state["scope"],"offers":[asdict(o) for o in state["offers"]],
                    "blocked":state["blocked"],"journal_sha256":sha(canonical(self._audit(current))),
                    "metadata_sha256":sha(canonical(metadata))}

        def restoration_pin(self):
            with self.transaction():
                return self._restoration_pin_locked(self._read())

    return Managed


def restore_managed_staging(quorum,slot_subjects,source,destination,*,expected_pin,source_retired,
                            before_publish=None):
    """Explicit non-destructive abandonment of uncertified managed work.

    Preserve current consent, failed-worker blocks, monotonic scope
    IDs and all retired lifecycle records, even when a world cut rolls
    back. Previously abandoned command IDs are permanently tombstoned
    in this local lineage. Never transfer a live scope or claim.
    """
    fields(expected_pin,"source","phase","scope","offers","blocked","journal_sha256","metadata_sha256")
    require(expected_pin["phase"]=="paused","managed restoration requires released scope")
    Authority = managed_authority(quorum,slot_subjects)

    def copy_extra(original,current,restored,tick,generation,head):
        pin = original._restoration_pin_locked(current)
        require(canonical(pin)==canonical(expected_pin),"managed restoration consent or journal pin changed")
        state = original._contribution(current)
        original._require_reconciled()
        require(state["phase"]=="paused" and not state["probes"] and not state["assignment"]
                and original.db.execute("SELECT COUNT(*) FROM managed_scopes WHERE released<>1").fetchone()[0]==0,
                "managed restoration requires released scope")
        require(original.db.execute("SELECT COALESCE(MAX(scope),0),COUNT(*) FROM managed_scopes").fetchone()
                ==(state["scope"],state["scope"]),"managed scope history incomplete")
        # Keep complete historical lifecycle and scope evidence. An
        # abandoned stage remains historical evidence, never an active
        # authorization; the new quarantine consumes a higher generation.
        restored.db.execute("DELETE FROM contribution")
        for table,order,arity in (("contribution","singleton",7),("lifecycle","seq",2),
                                  ("managed_scopes","scope",8),("managed_restores","generation",2),
                                  ("abandoned_commands","request_id",2)):
            rows = original.db.execute(f"SELECT * FROM {table} ORDER BY {order}").fetchall()
            restored.db.executemany(f"INSERT INTO {table} VALUES("+",".join("?" for _ in range(arity))+")",rows)
        retained = original.db.execute("""SELECT a.* FROM command_authorizations a
                    JOIN commands c ON c.request_id=a.request_id WHERE c.tick<=? ORDER BY c.tick,c.player""",(tick,)).fetchall()
        restored.db.executemany("INSERT INTO command_authorizations VALUES(?,?,?,?,?,?)",retained)
        for scope,plan,completion in original.db.execute("SELECT * FROM managed_certifications ORDER BY scope"):
            require(completion is not None and json.loads(plan)["candidate"]["tick"]<=tick,
                    "cannot discard a reconciled managed certificate")
            restored.db.execute("INSERT INTO managed_certifications VALUES(?,?,?)",(scope,plan,completion))
        abandoned = [row[0] for row in original.db.execute("SELECT request_id FROM commands WHERE tick>? ORDER BY tick,player",(tick,))]
        record = {"source_pin":pin,"accepted_head":head["head"],"restored_tick":tick,
                  "quarantine_generation":generation,"abandoned_request_ids":abandoned}
        restored.db.execute("INSERT INTO managed_restores VALUES(?,?)",(generation,canonical(record)))
        restored.db.executemany("INSERT INTO abandoned_commands VALUES(?,?)",[(i,generation) for i in abandoned])
        current_restored = restored._read()
        restored._write(current_restored,state,"certified_restore_quarantine")
        restored._audit_authorizations()
        require(restored._contribution(current_restored)==state,"managed restoration changed current consent")
        return {"managed_consent_preserved":True,"scope_sequence":state["scope"],
                "abandoned_request_ids":abandoned,"fresh_measured_admission_required":True}

    return _restore_reference_staging(quorum,source,destination,expected_source=expected_pin["source"],
            source_retired=source_retired,before_publish=before_publish,Authority=Authority,
            restore_extra=copy_extra,allow_pending=True)


def certify_managed_checkpoint(ledger,quorum,gate,label,*,before_retry=None,before_sync=None):
    """No offline fallback; release only the audited, drained staged cut."""
    require(isinstance(gate,CertifiedClientGate) and gate.ledger is ledger and gate.quorum is quorum,
            "managed publication gate differs")
    plan = ledger.prepare_certification(label)
    # The intent is durable before any request can cast a vote.
    from managed_certification import completion_from_quorum
    def before_collect():
        pinned,done,unchanged = ledger.certification_intent(plan["proposal"]["proposal_hash"])
        require(pinned==plan and done is None and unchanged,
                "managed consent or intent changed before collection; new votes forbidden")
    accepted = quorum.certify_network_checkpoint(label,plan["candidate"],before_retry=before_retry,
                    before_sync=before_sync,expected_proposal=plan["proposal"],before_collect=before_collect)
    ledger.finish_certification(plan,completion_from_quorum(quorum,plan,accepted))
    return accepted,gate.publish(accepted)
