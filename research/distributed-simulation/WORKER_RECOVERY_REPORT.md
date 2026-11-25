# #62 — Durable contribution withdrawal, scale-in and worker recovery

Date: 25 November 2026 UTC / 26 November UAE time.

**Locally verified research only.** The custom transaction reference
runtime still relies on one trusted local SQLite authority. This is
not live Minecraft migration, production consent, distributed quorum
or independent physical-host certification.

## Lifecycle and fencing

The recovery authority uses its own SQLite application ID so neither
the earlier regional authority nor the transaction-only authority can
open the database and bypass lifecycle checks. It durably records all
four explicit local slot choices and revisions, unavailable slots,
probe scope identity, selected slot/revision pairs, lifecycle phase
and the current cut. The lifecycle is:

- Paused: no active assignment. Zero contribution or no available
  slot starts no worker.
- Probing: reserve the exact eligible slot set before spawning
  capped probe processes. Only bounded read-only echo/status messages
  may precede their worker grant.
- Ready: compute complete measured placement with the #58 planner;
  retire unselected probes before assigning regions. Rotate all
  regional grants to a fresh generation at the exact durable cut.
- Running: claim that generation once, and admit only its authenticated
  worker receipts and the immutable #61 command batch.
- Draining: fence publication before closing worker sessions.
  Verify process exit, empty worker leaves and removal of those
  leaves and private network interfaces before releasing the scope.

Changed consent, including withdrawal, immediately enters draining
when a scope exists. The publication gate checks this phase inside
the same SQLite write transaction as the accepted world and
command receipt. Even complete receipts from the current generation
cannot commit after withdrawal, before any new generation is created.
A change ordered after an accepted tick does not roll back that
tick. The next tick's immutable queued/sealed commands are preserved.

The local controller checks actual kernel CPU/memory policy and
shared headroom before each attempted tick and again after receipt
collection, before durable publication; it also rechecks measured
placement feasibility and observation age. A failed worker or
partition freezes the runtime and is recorded as unavailable,
without representing the fault as user consent withdrawal. That
slot remains unavailable after database reopen until an explicit
new revision changes its choice. Unknown or stale scope fences and
cleanup receipts cannot release a different scope. A no-fit decision
creates no new regional generation.

The recovery policy is intentionally conservative: stop the entire
reference generation, verify cleanup, then start newly admitted
workers from the last accepted durable checkpoint. It is not a
live partial-region handoff or seamless player connection transfer.
Cleanup attestation is supplied by the trusted local controller
after verifying the actual Linux worker scope, not a remote
termination proof. Unexpected lab failures poison the scope;
there is no automatic retry without verified cleanup and fresh
admission. The kernel checks are observations, not a distributed
atomic proof of resource enforcement.

## Repeated real-process experiments

- [Run 1 raw receipt](evidence/2026-11-26-recovery-1.json)
- [Run 2 raw receipt](evidence/2026-11-26-recovery-2.json)

Each run uses the same 256-agent world, two active scripted clients,
three registered players and six sequential disposable worker scopes.
There are eight real worker process instances per run. They use
separate private network namespaces, private authenticated TCP,
distinct physical CPU cores and real delegated Linux cgroup-v2
resource limits. Client sessions use real TCP in the private
coordinator namespace. No host network or production keys are used.

| Stage | Actual action | Result |
| --- | --- | --- |
| Two workers | Two 56 MiB, 1000 millicore offers | Tick 1 commits; tick 2 staged |
| Withdrawal | Slot 1 opts out; slot 0 explicitly increases to 96 MiB | Old tick-2 candidate denied while both workers are alive; both exit cleanly |
| Scale-in | One admitted worker with all eight regions | Pending tick 2 commits once; then all contributions set to zero |
| All player-only | Explicit zero choices | No worker spawned, no tick advanced |
| Worker loss | Two new 56 MiB workers; slot 1 SIGKILL after prepare for tick 4 | Tick 3 retained; sealed tick-4 inputs remain pending |
| Insufficient capacity | Slot 1 unavailable; one 56 MiB probe | Complete placement denied; no generation or tick advanced; probe exits |
| Partition | Slot 0 explicitly raises its limit to 96 MiB; recovers tick 4 | Real veth link down after seal for tick 5; tick 4 retained |
| All workers unavailable | Both failed slots blocked | No worker spawned, no tick advanced |
| Recovery | Slot 1 gives fresh 96 MiB offer | Ticks 5 and 6 commit; final scope released |

The 56 MiB budget yields 48 MiB usable under the planner's reserve;
the complete eight-region fixture cannot fit the conservative
single-worker memory estimate at that limit. The explicit 96 MiB
offer supplies enough modeled capacity. This is a chosen reference
fixture and conservative placement heuristic, not a benchmark
of real machine or Minecraft memory requirements.

For the graceful withdrawal, the next tick was fully computed
tentatively and its complete authenticated receipts were available.
Consent changed before publication, and both direct publication and
further sealing were refused. Starting another scope before cleanup
and releasing only a subset of the old scope were also refused.
Both workers were still alive during these negative checks, then
exited normally with their capped leaves empty and removed.

For worker loss and partition, the failed tick is never published,
no client receives a committed reply, and the exact sealed commands
are retried after the ledger is reopened and workers are freshly
admitted. The killed worker exited with SIGKILL; the partitioned
worker was terminated during scoped cleanup. A graceful stop is
verified separately from these fault exits. Every accepted tick
and complete saved state is checked against serial reference
simulation. The final runtime root in both runs is
`3d8bef2bd7973f1fdb953fa45c422a8b3182a09c9083414e01319738d895b564`.

| Verification | Run 1 | Run 2 |
| --- | ---: | ---: |
| Accepted durable ticks | 6 | 6 |
| Unique committed player commands | 12 | 12 |
| Real TCP client sessions | 18 | 18 |
| Worker process instances | 8 | 8 |
| Fully released worker scopes | 6 | 6 |
| Actual worker-loss/partition faults | 2 | 2 |
| No-capacity pauses (including one probe-only rejection) | 3 | 3 |

There is no duplicate agent, inventory action, resource or world
tick during replay. The verifier checks each client response, sealed
journal, full save, generation/slot grant, placement decision from
recorded observations, lifecycle boundary, kernel-budget observation,
process exit and cleanup evidence. It cannot independently attest
historical process execution, measurements or MACs from the public
receipt alone.

## Local verification

80 research tests passed, including nine new tests for default
zero contribution, old-schema bypass denial, stale consent and
scope checks, withdrawal immediately before publication via another
SQLite connection, old receipt refusal before rotation, durable
unavailability, unselected probe retirement, reopen while draining,
and evidence alterations. Twenty deliberate alterations across the
two retained receipts were refused. The previous #56–61 unit and
retained-evidence regression suite passed. Workflow policy and
release metadata checks passed.

Reproduce with a new disposable scope and receipt path (Linux,
systemd user CPU/memory delegation, mapped user/network namespaces,
iproute2 and five distinct available physical CPU cores):

```sh
systemd-run --user --scope --quiet \
  --unit=swarmcraft-memory-cap-test-recovery-62-local-$$ \
  -p 'Delegate=memory cpu' -p RuntimeMaxSec=100 \
  env SWARMCRAFT_TEST_CPU_DELEGATION=1 \
  bash scripts/e2e/linux_cgroup_memory_gate.sh \
  unshare --user --map-root-user --net \
  python3 research/distributed-simulation/recovery_lab.py \
  --output /tmp/new-sc62.json
cd research/distributed-simulation
python3 verify_recovery_evidence.py /tmp/new-sc62.json
python3 -m unittest discover -s . -p 'test_*.py' -v
```

## Remaining gates

The tested scope is one trusted local authority on one workstation,
not independent device failure domains or actual Minecraft. The
controller cannot prove termination of an uncooperative remote
worker, and the test does not certify machine power loss, corrupted
or rolled-back authority storage, a compromised controller, native
device consent, automatic coordinator restart, or seamless live
migration. No sustained throughput, WAN capacity or actual
Minecraft gameplay result is established. Production distributed
hosting remains disabled. #63 next: publish honest comparative
sustained scale evidence for the supported experimental runtime and
retain the independent-host and actual Minecraft qualification
requirements.
