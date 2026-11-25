"""#58 research-only admission planner for the deterministic region model.

No canonical state, Minecraft protocol, signed authority, or production
consent import. All values are bounded and decisions are deterministic.
Offers are explicit local experiment choices, not remote attestations.
"""
from dataclasses import dataclass
from itertools import combinations, permutations
from math import ceil

from region_runtime import REGIONS, WIDTH, MAX_AGENTS, integer, require

MIB = 1024 * 1024
BASE_BYTES = 32 * MIB
RESERVE_BYTES = 8 * MIB
MAX_RTT_US = 8000
DEADLINE_US = 50_000
# Additional workers must improve the estimated critical path by at
# least 20% AND 1 ms. This is not a measured performance guarantee.
GAIN_PERCENT = 20
MIN_GAIN_US = 1000
MAX_SAMPLE_AGE_NS = 10_000_000_000


@dataclass(frozen=True)
class Offer:
    slot: int
    revision: int
    process_memory_mib: int
    cpu_millicores: int

    def validate(self):
        require(integer(self.slot, 0, 3) and integer(self.revision, 1, 2**63-1),
                "invalid explicit lab consent identity")
        require((self.process_memory_mib == self.cpu_millicores == 0
                 and type(self.cpu_millicores) is int and type(self.process_memory_mib) is int)
                or (integer(self.process_memory_mib, 16, 128)
                    and integer(self.cpu_millicores, 250, 1000)),
                "invalid lab resource preference")

    @property
    def opted_in(self):
        return self.process_memory_mib > 0 and self.cpu_millicores > 0


@dataclass(frozen=True)
class Sample:
    slot: int
    revision: int
    effective_cpu_millicores: int
    usable_memory_bytes: int
    rtt_us: int
    observed_ns: int
    verified_enforcement: bool

    def validate(self):
        require(integer(self.slot, 0, 3) and integer(self.revision, 1, 2**63-1)
                and integer(self.effective_cpu_millicores, 0, 1000)
                and integer(self.usable_memory_bytes, 0, 128*MIB)
                and integer(self.rtt_us, 0, 3_000_000)
                and integer(self.observed_ns, 0, 2**63-1)
                and type(self.verified_enforcement) is bool, "invalid measured capacity")


def region_load(state):
    """Measure current region agents and boundary candidates.

    Explicit conservative cost model, not benchmark output: 200 us per
    region + 2 us/agent + 3 us/edge agent, and 2 MiB/region + 1 KiB per
    resident/possible adjacent incoming agent, over a per-worker base.
    The same model applies to all layouts; the kernel cap is independent.
    """
    require(set(state) == set(range(REGIONS)), "incomplete region state")
    counts = [len(state[r].agents) for r in range(REGIONS)]
    require(sum(counts) <= MAX_AGENTS, "excessive region load")
    left = [sum(a[1] == r*WIDTH for a in state[r].agents) for r in range(REGIONS)]
    right = [sum(a[1] == (r+1)*WIDTH-1 for a in state[r].agents) for r in range(REGIONS)]
    return [{"region": r, "agents": counts[r], "left": left[r], "right": right[r],
             "cpu_us": 200 + 2*counts[r] + 3*(left[r]+right[r]),
             "memory_bytes": 2*MIB + 1024*(counts[r]+right[(r-1)%REGIONS]+left[(r+1)%REGIONS])}
            for r in range(REGIONS)]


def assess(offer, sample, now_ns):
    offer.validate()
    if not offer.opted_in:
        return "player_only"
    if sample is None:
        return "unmeasured"
    sample.validate()
    if sample.slot != offer.slot or sample.revision != offer.revision:
        return "stale_consent"
    if not 0 <= now_ns-sample.observed_ns <= MAX_SAMPLE_AGE_NS:
        return "stale_measurement"
    if not sample.verified_enforcement:
        return "unsupported_enforcement"
    if (sample.effective_cpu_millicores > offer.cpu_millicores
            or sample.usable_memory_bytes > offer.process_memory_mib*MIB):
        return "inconsistent_measurement"
    if sample.effective_cpu_millicores < 250:
        return "insufficient_cpu"
    if sample.usable_memory_bytes < BASE_BYTES + 2*MIB:
        return "insufficient_memory"
    if sample.rtt_us > MAX_RTT_US:
        return "path_too_slow"
    return "eligible"


def evaluate(owner, load, samples):
    """Return estimated service critical path or None if any bound fails."""
    used = sorted(set(owner))
    costs = {}
    for slot in used:
        rows = [row for row, s in zip(load, owner) if s == slot]
        sample = samples[slot]
        memory = BASE_BYTES + sum(row["memory_bytes"] for row in rows)
        if memory > sample.usable_memory_bytes:
            return None
        compute = ceil(sum(row["cpu_us"] for row in rows)*1000/sample.effective_cpu_millicores)
        costs[slot] = {"regions": [r["region"] for r in rows], "estimated_cpu_us": compute,
                       "estimated_memory_bytes": memory}
    # Three synchronous exchanges each tick, plus a conservative
    # per-crossing serialization allowance. No invented bandwidth rate.
    crossings = sum(load[r]["right"] + load[(r+1)%REGIONS]["left"] + 1
                    for r in range(REGIONS) if owner[r] != owner[(r+1)%REGIONS])
    score = max(c["estimated_cpu_us"] for c in costs.values()) + 3*max(
        samples[s].rtt_us for s in used) + 2*crossings
    if score > DEADLINE_US:
        return None
    return {"owner_by_region": list(owner), "score_us": score,
            "estimated_boundary_events": crossings,
            "allocations": {str(slot): costs[slot] for slot in used}}


def validate_load(load):
    require(len(load) == REGIONS and [r["region"] for r in load] == list(range(REGIONS)),
            "invalid region load")
    for row in load:
        require(set(row) == {"region", "agents", "left", "right", "cpu_us", "memory_bytes"}
                and integer(row["agents"], 0, MAX_AGENTS)
                and integer(row["left"], 0, row["agents"])
                and integer(row["right"], 0, row["agents"])
                and row["left"]+row["right"] <= row["agents"]
                and integer(row["cpu_us"], 1, 1_000_000)
                and integer(row["memory_bytes"], 1, 128*MIB), "invalid load estimate")
    require(sum(r["agents"] for r in load) <= MAX_AGENTS, "excessive total load")
    for i,row in enumerate(load):
        require(row["cpu_us"] == 200+2*row["agents"]+3*(row["left"]+row["right"])
                and row["memory_bytes"] == 2*MIB+1024*(row["agents"]
                    +load[(i-1)%REGIONS]["right"]+load[(i+1)%REGIONS]["left"]),
                "inconsistent cost model")


class ConsentLedger:
    """Ephemeral experiment-generation choices, never production consent.

    A changed offer must advance its revision; omission is not withdrawal.
    Call only when the previous generation has fully exited.
    """
    def __init__(self):
        self.previous = {}

    def accept(self, offers):
        require(len(offers) == 4 and {o.slot for o in offers} == set(range(4)),
                "each device needs an explicit offer or player-only choice")
        for o in offers:
            o.validate()
            prior = self.previous.get(o.slot)
            require(prior is None or o == prior or o.revision > prior.revision,
                    "stale or reused consent revision")
        self.previous = {o.slot:o for o in offers}


def choose(offers, samples, load, now_ns):
    """Small exhaustive contiguous-placement search. No partial worlds."""
    require(integer(now_ns, 0, 2**63-1), "invalid observation clock")
    require(1 <= len(offers) <= 4 and len({o.slot for o in offers}) == len(offers),
            "duplicate or excessive offers")
    require(set(samples) <= {o.slot for o in offers}, "unknown sample")
    validate_load(load)
    reasons = {o.slot: assess(o, samples.get(o.slot), now_ns) for o in offers}
    eligible = sorted(s for s, reason in reasons.items() if reason == "eligible")
    best = {}
    for n in range(1, len(eligible)+1):
        for cuts in combinations(range(1, REGIONS), n-1):
            bounds = (0,) + cuts + (REGIONS,)
            for slots in permutations(eligible, n):
                owner = tuple(slot for i, slot in enumerate(slots)
                              for _ in range(bounds[i+1]-bounds[i]))
                result = evaluate(owner, load, samples)
                if result is not None:
                    key = result["score_us"], result["estimated_boundary_events"], owner
                    if n not in best or key < best[n][0]:
                        best[n] = key, result
    chosen = None
    for n in sorted(best):
        candidate = best[n][1]
        if chosen is None or (
            chosen["score_us"]-candidate["score_us"] >= MIN_GAIN_US
            and candidate["score_us"]*100 <= chosen["score_us"]*(100-GAIN_PERCENT)
        ):
            chosen = candidate
    for slot in eligible:
        reasons[slot] = ("admitted_for_experiment" if chosen and slot in chosen["owner_by_region"]
                         else "no_useful_feasible_placement" if chosen else "no_complete_feasible_placement")
    return {"plan": chosen, "reasons": {str(k): v for k,v in sorted(reasons.items())},
            "best_scores_by_worker_count": {str(n): pair[1]["score_us"] for n,pair in best.items()}}
