"""#62 durable contribution lifecycle for the trusted local reference world.

Consent and publication are serialized by the SAME SQLite transaction lock.
The lab controller is trusted to supply real measurements and cleanup proof;
this is not production enrollment or distributed consensus.
"""
from dataclasses import asdict
import json
import time

from region_runtime import REGIONS, canonical, integer, require
from regional_authority import MAX_GENERATION, new_grants, public_grants
from transition_runtime import TransitionAuthority
from worker_placement import Offer, Sample, choose, region_load

SLOTS = tuple(range(4))
PHASES = ("paused", "probing", "ready", "running", "draining")
FAULTS = ("worker_loss", "partition", "policy", "startup", "stop")


def checked_offers(offers):
    require(len(offers) == 4 and [o.slot for o in offers] == list(SLOTS), "explicit ordered choices required")
    for o in offers:
        o.validate()
    return offers


def active_slots(offers, blocked):
    return [o.slot for o in offers if o.opted_in and o.slot not in blocked]


class RecoveryAuthority(TransitionAuthority):
    """Separate database ID so old authorities cannot bypass lifecycle gates."""
    app_id = 0x53433632

    @classmethod
    def _create_extra(cls, db, state):
        super()._create_extra(db, state)
        db.execute("""CREATE TABLE contribution(singleton INTEGER PRIMARY KEY CHECK(singleton=1),
            phase TEXT NOT NULL, scope INTEGER NOT NULL, offers BLOB NOT NULL, blocked BLOB NOT NULL,
            probes BLOB NOT NULL, assignment BLOB NOT NULL)""")
        db.execute("CREATE TABLE lifecycle(seq INTEGER PRIMARY KEY, event BLOB NOT NULL)")
        db.execute("INSERT INTO contribution VALUES(1,?,?,?,?,?,?)", (
            "paused", 0, canonical([asdict(Offer(s, 1, 0, 0)) for s in SLOTS]),
            b"[]", b"[]", b"[]"))

    def __init__(self, path, *, read_only=False):
        super().__init__(path, read_only=read_only)
        try:
            self.status()
        except BaseException:
            self.close()
            raise

    def _contribution(self, cur):
        phase, scope, offers_raw, blocked_raw, probes_raw, assignment_raw = self.db.execute(
            "SELECT phase,scope,offers,blocked,probes,assignment FROM contribution WHERE singleton=1").fetchone()
        offers = checked_offers([Offer(**o) for o in json.loads(offers_raw)])
        blocked, probes, assignment = map(json.loads, (blocked_raw, probes_raw, assignment_raw))
        require(phase in PHASES and integer(scope, 0, 2**63-1) and all(isinstance(v, list) for v in (blocked, probes, assignment))
                and all(v == sorted(set(v)) and all(integer(s, 0, 3) for s in v)
                        for v in (blocked, probes)), "invalid contribution state")
        require(set(blocked) <= {o.slot for o in offers if o.opted_in}, "invalid unavailable slots")
        require(all(isinstance(a, list) and len(a) == 2 and integer(a[0], 0, 3)
                    and integer(a[1], 1, 2**63-1) for a in assignment), "invalid assigned revision")
        slots = [a[0] for a in assignment]
        require(slots == sorted(set(slots)) and set(slots) <= set(probes), "invalid active assignment")
        if phase == "paused":
            require(not probes and not assignment, "paused scope still assigned")
        if phase == "probing":
            require(probes == active_slots(offers, blocked) and probes and not assignment, "invalid probe scope")
        if phase in ("ready", "running"):
            require(probes == active_slots(offers, blocked) and len(slots) == len(cur["grants"])
                    and all(offers[s].opted_in and offers[s].revision == rev for s, rev in assignment)
                    and (phase == "running") == cur["claimed"], "invalid active contribution grant")
        return {"phase": phase, "scope": scope, "offers": offers, "blocked": blocked, "probes": probes, "assignment": assignment}

    def _write(self, cur, state, event):
        self.db.execute("UPDATE contribution SET phase=?,scope=?,offers=?,blocked=?,probes=?,assignment=? WHERE singleton=1",
                        (state["phase"], state["scope"], canonical([asdict(o) for o in state["offers"]]),
                         canonical(state["blocked"]), canonical(state["probes"]), canonical(state["assignment"])))
        self.db.execute("INSERT INTO lifecycle(event) VALUES(?)", (canonical({
            "event": event, "phase": state["phase"], "scope": state["scope"], "generation": cur["generation"],
            "tick": cur["tick"], "root": cur["runtime_root"],
            "offers": [asdict(o) for o in state["offers"]], "blocked": state["blocked"],
            "probes": state["probes"], "assignment": state["assignment"]}),))

    def status(self):
        with self.transaction():
            cur = self._read()
            state = self._contribution(cur)
            return {**state, "offers": [asdict(o) for o in state["offers"]],
                    "generation": cur["generation"], "tick": cur["tick"], "root": cur["runtime_root"]}

    def events(self):
        with self.transaction():
            self._contribution(self._read())
            return [json.loads(r[0]) for r in self.db.execute("SELECT event FROM lifecycle ORDER BY seq")]

    def configure(self, offers):
        """Changed consent fences active or probing work immediately."""
        checked_offers(offers)
        with self.transaction(write=True):
            cur = self._read()
            state = self._contribution(cur)
            old = state["offers"]
            require(all(o == prior or o.revision > prior.revision for o, prior in zip(offers, old)),
                    "stale or reused consent revision")
            if offers == old:
                return
            state["blocked"] = [s for s in state["blocked"] if offers[s] == old[s]]
            state["offers"] = offers
            if state["phase"] != "paused":
                state["phase"] = "draining"
            self._write(cur, state, "consent_changed")

    def _before_probe(self, current, state, slots):
        pass

    def _before_admit(self, current, state, samples, now_ns):
        pass

    def begin_probe(self):
        """Durably reserve the exact allowed local slots before any spawn."""
        with self.transaction(write=True):
            cur = self._read()
            state = self._contribution(cur)
            require(state["phase"] == "paused", "previous scope not released")
            slots = active_slots(state["offers"], state["blocked"])
            if not slots:
                return []
            require(state["scope"] < 2**63-1, "scope sequence exhausted")
            state.update(phase="probing", scope=state["scope"]+1, probes=slots)
            self._before_probe(cur, state, slots)
            self._write(cur, state, "begin_probe")
            return [state["offers"][s] for s in slots]

    def admit(self, expected_scope, samples, retired_slots, now_ns=None):
        """Select measured complete placement, retire unselected probes first.

        The local controller supplies the observation and retirement evidence.
        The ledger recomputes the deterministic plan under its write lock.
        """
        now_ns = time.monotonic_ns() if now_ns is None else now_ns
        with self.transaction(write=True):
            cur = self._read()
            state = self._contribution(cur)
            require(state["phase"] == "probing" and integer(expected_scope, 1, 2**63-1)
                    and expected_scope == state["scope"], "probe scope retired")
            require(set(samples) == set(state["probes"]) and all(isinstance(v, Sample) for v in samples.values()),
                    "missing probe measurements")
            self._before_admit(cur, state, samples, now_ns)
            decision = choose(state["offers"], samples, region_load(cur["state"]), now_ns)
            plan = decision["plan"]
            selected = sorted(set(plan["owner_by_region"])) if plan else []
            require(retired_slots == sorted(set(state["probes"]) - set(selected)),
                    "unselected probe not retired")
            if not plan:
                state["phase"] = "draining"
                self._write(cur, state, "no_feasible_placement")
                return decision
            groups = [[r for r in range(REGIONS) if plan["owner_by_region"][r] == s] for s in selected]
            generation = cur["generation"] + 1
            require(generation <= MAX_GENERATION, "generation exhausted")
            grants = new_grants(cur["world"], generation, cur["tick"], cur["runtime_root"], groups)
            self.db.execute("UPDATE world SET generation=?,grants=?,claimed=0 WHERE singleton=1",
                            (generation, canonical(grants)))
            self.db.execute("INSERT INTO epochs VALUES(?,?,?,?,0)",
                            (generation, cur["tick"], cur["runtime_root"], canonical(public_grants(grants))))
            cur["generation"] = generation
            state.update(phase="ready", assignment=[[s, state["offers"][s].revision] for s in selected])
            self._write(cur, state, "admitted")
            return decision

    def rotate(self, *args, **kwargs):
        require(False, "use measured contribution admission")

    def _before_claim(self, current):
        state = self._contribution(current)
        require(state["phase"] == "ready", "no admitted contribution scope")
        state["phase"] = "running"
        self._write(current, state, "claimed")

    def _running(self, current):
        state = self._contribution(current)
        require(state["phase"] == "running", "contribution scope fenced")
        return state

    def _before_seal(self, current):
        self._running(current)

    def _record_candidate(self, current, state, candidate, cut):
        self._running(current)
        super()._record_candidate(current, state, candidate, cut)

    def fence(self, expected_scope, expected_generation, reason, unavailable=()):
        """Stop publication before draining. Faulted slots need new consent revision."""
        require(reason in FAULTS and all(integer(s, 0, 3) for s in unavailable)
                and len(set(unavailable)) == len(unavailable), "invalid fence reason")
        with self.transaction(write=True):
            cur = self._read()
            state = self._contribution(cur)
            require(integer(expected_scope, 1, 2**63-1) and expected_scope == state["scope"]
                    and integer(expected_generation, 1, MAX_GENERATION) and
                    expected_generation == cur["generation"] and state["phase"] != "paused",
                    "stale scope fence")
            require(set(unavailable) <= set(state["probes"]), "unknown failed worker")
            state["blocked"] = sorted(set(state["blocked"]) |
                                      {s for s in unavailable if state["offers"][s].opted_in})
            state["phase"] = "draining"
            self._write(cur, state, reason)

    def release(self, expected_scope, expected_generation, retired_slots):
        """Trusted local cleanup attestation, NOT a remote termination proof."""
        with self.transaction(write=True):
            cur = self._read()
            state = self._contribution(cur)
            require(integer(expected_scope, 1, 2**63-1) and expected_scope == state["scope"]
                    and integer(expected_generation, 1, MAX_GENERATION) and expected_generation == cur["generation"]
                    and state["phase"] == "draining" and retired_slots == state["probes"],
                    "scope cleanup incomplete or stale")
            state.update(phase="paused", probes=[], assignment=[])
            self._write(cur, state, "released")
