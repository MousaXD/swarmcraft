# #60 — Exclusive regional writers and consistent world saves

Date: 25 November 2026 UTC / 26 November UAE time.

**Locally verified research only.** This is a single trusted, durable
local authority ledger, not distributed consensus or production
Minecraft authority. The existing single-authority Fabric product
path and regional-production NO-GO are unchanged.

## Design and global state ownership

The authority uses a private, explicitly created SQLite database with
a distinct application ID and schema, DELETE rollback journal and
FULL synchronous transactions. It owns the canonical world identity,
ruleset, fixed player roster and agent identity set, resource
conservation total, global tick, current generation, accepted
generation, runtime and world-bound cut roots, full region state, and
the exclusive assignment of every region. The region workers own
only tentative local computation; their prepare/seal/commit replies
do not publish canonical state.

Each generation assigns the eight regions exactly once, in disjoint
groups. All sessions receive fresh random session IDs and distinct
private MAC keys. The generation is claimed exactly once in a durable
transaction, even before the first tick. A failed startup or restart
requires a fresh generation rather than reusing the previous claim.
Rotation uses a compare-and-swap on the current generation, accepted
tick and runtime root and changes all region session credentials at
once, including unchanged placements. It does not change the
accepted world state. Workers bind once to their world, generation,
region scope and starting cut. They cannot rebind to another grant
or attest a snapshot before a local commit.

For each candidate tick, the existing coordinator first completes
the bounded worker barrier. The fenced adapter then requests
authenticated snapshots bound to the world, session, generation,
next tick, exact parent runtime root, candidate runtime root and
full region rows. It checks the candidate matches the barrier
before submitting to the authority. Inside one write transaction
the authority checks the generation was claimed, all current
sessions appear exactly once, all MACs and region scopes match,
parent and next tick are exact, and all receipts agree on the cut.
It independently reconstructs the complete world, checks exact
agent identities, resource conservation and the runtime root.
A unique tick record and the full new world state are committed
atomically. Refusal or transaction rollback cannot publish a new
canonical cut. A failed adapter freezes rather than retrying an
ambiguous tick or operating on tentative state.

The world-wide save is a single read transaction returning the
whole accepted cut plus the current grant map and accepted
generation. Current generation and accepted generation are
intentionally distinct after reassignment, before its first tick.
The save contains no session keys. The ledger preserves the exact
accepted tick/root and full state on reopen, and retains tick
history and public generation history. A reader during an uncommitted
write sees the previous whole cut, never a mixture.

Global commands, time/weather beyond the tick, portal behavior,
Minecraft saves and mod-specific global state are outside this
reference ruleset. There is no production network endpoint, no
reuse of actual SwarmCraft keys, no quorum certificate, no WAN
authority, no claim of resilience to a malicious local authority,
database rollback/replacement, copied signing credentials, physical
disk failure or Byzantine workers. The trusted local controller and
database are the single serialization and trust domain. Workers
receive lab-only MAC keys over private authenticated channels, not
publicly verifiable signatures. The authority must not be cloned
into competing ledgers. The ledger and its keys are temporary
synthetic lab data; no keys are included in retained receipts.

## Live evidence

- [Run 1](evidence/2026-11-26-authority-1.json)
- [Run 2](evidence/2026-11-26-authority-2.json)
- [Unfenced network regression](evidence/2026-11-26-authority-static-regression.json)

Each final run used a 256-agent, eight-region synthetic world,
a real local ledger, and four simultaneous worker processes in
distinct private network namespaces during reassignment. Separate
authority contender processes opened the same database. Both the
old and the new pair computed the identical next candidate (tick 4)
from tick 3; only the new generation could publish. The old pair
was still alive when its receipts were refused. After a complete
worker shutdown, the ledger reopened and a third generation used
new worker processes, resuming from the accepted cut without
resetting tick, world identity or inventory/resource state.

| Check | Run 1 | Run 2 |
| --- | --- | --- |
| Durable accepted ticks | 9 | 9 |
| Generations | 3 | 3 |
| Full save observations | 17 | 17 |
| Concurrent generation rotation | 1/2 accepted | 1/2 accepted |
| Concurrent tick publication | 1/2 accepted | 1/2 accepted |
| Explicit stale/overlap/replay/restart refusals | 8 | 8 |
| Process killed before durable commit | Previous cut preserved | Previous cut preserved |
| Process killed after commit, before acknowledgment | New cut preserved; replay refused | New cut preserved; replay refused |
| All scoped workers exited | Yes | Yes |

The two SIGKILL fault cases in each run kill a separate SQLite
authority writer process, not the coordinator or a Minecraft
server. The pre-commit kill occurs after the transaction updates
both history and world state; reopening/reading rolls it back.
The post-commit kill occurs after COMMIT returns and before a reply;
the new state is read from the ledger, not guessed from a missing
acknowledgment. The ledger then rejects replay. These tests exercise
process failure on one Linux filesystem, not sudden power loss.

All 18 accepted ticks across the two runs match independent
monolithic serial computation, and every full save observation
(including rotation, claim and crash boundaries) matches the
corresponding exact serial state. Both final runtime roots are
`82cb179c28f5eb6b6f879697c49d2f09da10193776f68f2663d595c65dee2690`.
World IDs and session IDs are independently randomized. The offline
verifier checks full state, roots, generation/session history,
refusals and race outcomes; it cannot independently authenticate
the MACs or attest process execution without the live experiment.
The 58-test research suite includes 14 new tests, with 18 deliberately
altered retained-evidence cases refused, malformed/forged/mixed
receipts, overlap, replay, unclaimed generation, restart before
tick one, concurrent reader consistency and adapter freeze.
The prior unfenced static network path also passed a focused
eight-tick one-/two-worker regression and both kill/partition
faults; no performance comparison is claimed for the new ledger.

Reproduce in a fresh mapped user/network namespace on Linux
with five distinct physical CPU cores available:

```sh
cd research/distributed-simulation
python3 -m unittest discover -s . -p 'test_*.py' -v
unshare --user --map-root-user --net python3 authority_lab.py --output /tmp/new-sc60.json
python3 verify_authority_evidence.py /tmp/new-sc60.json
```

## Remaining gates

#60 is locally verified for the explicitly chosen trusted-ledger
reference runtime. A genuinely distributed authority must replace
this serialization point with reviewed durable quorum-backed
generation certificates and publication/recovery rules across
independent control domains; it must address full ledger rollback,
compromised writers and production authentication. The fixed player
roster is preserved in each cut, but this experiment uses autonomous
reference movement rather than real player-client interaction.
#61 next: durable client-controlled player/entity/inventory
transitions and replay, including cross-region interaction and
restart. #62: worker withdrawal and partition recovery. Actual
Minecraft compatibility, independent physical hosts and sustained
scale tests remain unverified. Do not enable distributed gameplay.
