# Combined ownerless reference qualification (#68)

Date: 2026-11-26 (local). Classification:
`LOCAL_COMBINED_CFT_REFERENCE_SESSION_NOT_MINECRAFT_NOT_MULTI_HOST`.

## What ran

Two isolated runs on one Linux machine used the actual Rust ownerless CLI,
separate disposable application identities and signed journals, a trusted
local SQLite transaction ledger, real private TCP client processes, and
policy-pinned regional worker processes in separate network namespaces.
Linux cgroup v2 enforced process CPU and memory limits. The retained
second run strengthens the retired-receipt negative to require an exact
stale-session refusal, rather than merely rejecting the wrong receipt count.

The signed creation certificate pins the private no-OP policy and hashes
of the reference implementation and tested CLI. Each local staged tick is
replayed and committed to an exact checkpoint; the controller obtains
and checks a signed current-majority history extension before returning
the committed client acknowledgement. The staging ledger is not itself
an independent canonical quorum publisher. All role proofs are explicit
one-shot reference proofs; no Minecraft login, runtime permit or production
worker admission is granted.

| Stage | Active regional workers | Enforced per-worker ceilings | Certified tick |
|---|---:|---|---:|
| Founder present | 1 | 96 MiB / 1000 millicores | 1 |
| Founder departed; two surviving validators | 2 | 96 MiB / 1000 millicores each | 2 |
| Extra worker revoked; surviving worker reduced | 1 | 80 MiB / 750 millicores | 3 |

The founder's journal remains byte-identical after departure, and no later
operation uses that identity. The surviving two validators certify removal
of the founder's player and worker eligibility, recovery, and continued
checkpoint publication. Player 1 crosses from region 0 on worker slot 1 to
region 7 on slot 2, with inventory and view independently replayed.
The extra worker's later volunteer proof is denied because its eligibility
was revoked. Each old generation exits and releases its scope before the
next one starts. Authenticated retired receipts are refused even when padded
or truncated to the current worker count. Each worker refuses OP and vote
requests. A single remaining validator cannot assemble a recovery decision;
the exported copy matches the last certified checkpoint byte for byte and
has no write authority.

## Retained evidence and reproduction

- [Public observations and signed history](evidence/2026-11-26-combined-ownerless.json):
  13 signed decisions, 3 certified reference ticks, 4 worker processes,
  4 client receipts, and 9 role proof observations (including the denied
  withdrawn-worker proof).
- `verify_combined_ownerless_evidence.py` independently replays all three
  ticks, views, inventory, journals, assignment and resource observations,
  verifies role context and reported refusal/consumption results, and uses
  the Rust CLI to cryptographically verify the creation certificate, every
  history prefix and each exact checkpoint commitment.
- The retained proof observations do not independently re-establish the
  role signatures or attest remote process execution. The running CLI
  checked these signatures and consumed its own challenges. A trusted
  local controller supplies process, timing, acknowledgement ordering and
  cgroup observations; these are not cryptographic remote attestations.
- Mutation regressions reject false field claims, invalid signer sets,
  altered checkpoints, incorrect crossing, stale-session test dilution,
  false role grants, corrupted kernel/client observations and wrong
  minority recovery claims. All 90 distributed-reference unit and regression
  tests passed; release-version and workflow-policy checks passed.

## Certified checkpoint client boundary (follow-up)

The original controller sequencing withheld client acknowledgments
until a checkpoint was certified, but the unguarded local SQLite
interface could still return staged views or committed local results.
The reference client adapter now uses a separate `CertifiedClientGate`.
It pins the world, policy, compatibility, exact accepted head, reference
fence, local verifier and the independently mapped reference client
identities. Each state-bearing operation builds the checkpoint from
one audited SQLite transaction. Pending inputs remain valid local
staging but are excluded from the committed snapshot; all committed
commands, receipts, sealed inputs and region state are independently
replayed. Neither an uncommitted input nor a later local FULL
transaction constitutes shared publication.

The new CLI command `ownerless authority verify-reference-cut` opens
the existing signed private validator journal under its nonblocking
stable lock, verifies the entire history and durable checkpoint bytes,
requires exact independent world/head/fence/policy/compatibility pins,
rejects pending governance votes, and requires the verifier to be a
current validator and the supplied reference player to be invited.
The supplied public key is an independently mapped, already
authenticated reference connection identity, not proof of possession
or an actual Minecraft login. This adapter explicitly refuses public
reference mode. It creates no permit, vote, world or control record.
When provided, the expected parent must be the immediate predecessor
of a certified checkpoint decision at the same fence, not a recovery
or role change that happens to retain identical checkpoint bytes.

Example, using the already reviewed disposable profile and exact
candidate checkpoint file:

```sh
swarmcraft --data-dir /private/validator-a ownerless authority verify-reference-cut \
  --expected-world "$WORLD" --expected-head "$HEAD" \
  --expected-fence "$FENCE" --expected-policy "$POLICY" \
  --expected-compatibility "$COMPATIBILITY" --expected-parent "$PARENT" \
  --checkpoint exact-cut.bin --player-key "$PLAYER_KEY"
```

The client gate will not advance its session pin except for a directly
certified next tick with unchanged fence/policy/compatibility. A new
governance head requires a new session and a separately verified role
proof. State views and input queueing recheck the local accepted
checkpoint; read-only receipt retrieval checks exact committed request
identity and payload within that checkpoint. The actual TCP client
adapter rechecks previously committed replays immediately before
sending rather than returning a cached result or enqueuing a missing
command. On reopen, a locally advanced but uncertified SQLite database
is refused even if its own audit is valid. A certified checkpoint can
be reopened before any acknowledgment is sent, and its exact result
can be recovered; a subsequently revoked player cannot use either
the old or a freshly opened session to retrieve it.

This is a fail-closed *local reference publication check*. It does not
make the Rust journal and SQLite transaction a distributed atomic
commit, lock a distributed quorum until the eventual network send,
prove global freshness or authorize actual gameplay. The trusted
fixture still establishes connection identities and worker admission
outside this adapter. A local certificate may be superseded elsewhere;
the local verifier's result is not a reusable freshness lease. Lost
majority, unresolved votes and a staged cut without a matching
certificate do not cause rollback, vote reset or permission to resume.
Recovery of a divergent uncertified SQLite staging database from the
accepted checkpoint remains a separate operation.

Schema 2 of the combined evidence adds observed refusal of staged
state/input/receipt access and exact publication-check reports for
every active player at each certified tick. The independent evidence
checker compares those reports to the signed history, player mapping,
and independently replayed checkpoint; older schema-1 receipts
remain independently verifiable and cannot claim the new guard.
The role and publication reports are local observations, not remote
attestation.

The retained [schema-2 combined evidence](evidence/2026-11-26-combined-certified-gate.json)
has SHA-256 `320d66dcb69e915870912930dfc5d37542d06ad26e4ab131b2bca8913a5114ee`.
The actual delegated Linux cgroup/user/network namespace run exited
successfully. All three stages refused uncertified view, input and
receipt operations; four per-player exact publication reports and
four committed TCP receipts reconcile with the 13 signed decisions,
three replayed ticks, four worker processes and nine role observations.
The independent verifier checked the history and checkpoint bytes
using the actual Rust CLI. Every recorded artifact hash still matched
the tested file; all worker scopes were released. The founder was
not contacted after departure, and minority publication remained
refused. This is a new bounded local qualification, not sustained
service, live global authority or a real game session.

Additional real-CLI regressions cover a locally committed but
uncertified database reopen, exact certified receipt replay after
reopen before acknowledgment, missing/conflicting requests, stale
session and freshly reopened revoked-player refusal, and no writes
through the read-only receipt path. A separate send-boundary regression
proves that a cached committed result is not transmitted when current
verification refuses it. Rust CLI integration covers stale pins,
wrong policy/compatibility/fence, wrong parent, nonvalidator verifier,
uninvited and revoked players, pending vote, incorrect checkpoint and
unsupported public mode.

All 135 research regression tests (including the explicitly enabled
real-CLI test), 29 authority and two creation CLI integration tests,
strict all-target/all-feature CLI Clippy, formatting, workflow-policy
and release-version checks passed locally. The shared TCP client send
path also passed the existing #61 real-process fault regression:
[retained raw regression evidence](evidence/2026-11-26-transition-certified-gate-regression.json),
SHA-256 `fc6a2aaa3cb1f315a3bafce50a31a1c3ba75b0799853deb98b3f9d2401c8f8f3`.
Independent replay verifies four accepted ticks, eight unique commands,
14 client sessions, eight crossings, two actual writer SIGKILL faults,
two conflicting replay refusals and two recovered receipts.

For the real-CLI Python regression, supply the built binary explicitly:

```sh
SWARMCRAFT_TEST_CLI="$PWD/target/debug/swarmcraft" \
  python3 -m unittest discover -s research/distributed-simulation -p 'test_*.py' -v
```

## Encrypted checkpoint certification follow-up

Schema 3 connects the guarded reference client boundary to the
existing authenticated Noise vote collection and durable checkpoint
sync protocol. The three checkpoint decisions no longer use offline
vote files or offline acceptance. The trusted local fixture still
uses offline creation, role changes, and recovery decisions; it owns
all of the disposable validator identities. This experiment does not
constitute independent operator review or global consensus availability.

For each checkpoint, the controller obtains each selected validator's
exact retained base via its own signed journal and exports that
validator's verified history. Independently replay the next state
from that prior checkpoint and the new sealed input batch: preserve
the earlier journal and genesis exactly, enforce command identity,
parent and tick, recompute every agent transition and exact root,
and compare the resulting command receipts. No worker receipt or
coordinator state is used as the review's simulation input. The
controller derives and pins the same proposal from each validator's
own history, and starts an explicitly reviewed one-shot vote listener.
The collector is the selected surviving validator (slot 1), not an
organizer or a privileged founder. The actual CLI persists and
collects the required votes over mutually authenticated encrypted
loopback TCP and verifies and accepts the exact certificate locally.

Publication remains withheld after the collector's local commit.
All other active profiles receive the same certified cut through
encrypted `serve-sync` / `push-sync`, and the sender verifies their
exact signed durable acceptance receipts. Only after these operations
finish and all active journals match may the client gate advance its
pin and perform its usual independent exact-cut check. A failed
operation never falls back to offline assembly or acceptance. The
surviving-quorum stage additionally attempts collection against an
unavailable private endpoint before starting the voter; the collector
must keep its exact pending vote and accepted base, the remote
validator must remain unchanged, and the client must still be denied
the staged cut. Only the same reviewed candidate is retried.

Workers complete and retire before the slower governance exchange.
Their resource scopes are released and their runtime instance is
frozen; no idle worker timeout or generation ownership check is
relaxed. The first attempt at this experiment exceeded the previous
ten-second client and twelve-second worker receive windows. The
corrected experiment gives only the combined reference clients an
explicit bounded 60-second acknowledgment wait (ordinary reference
clients retain ten seconds). This is a stop-and-certify research
session, not a sustained 20 Hz service measurement. The failure
does not justify extending worker authorization or granting a lease.

Schema 3 records the local replay observations, exact listener pins,
signed sync receipts, failed minority collection status, pre-sync
publication refusal and early worker retirement. The independent
verifier also verifies the actual Ed25519 sync signatures using the
version-1 receipt encoding and OpenSSL 3 (`pkeyutl`); signed creation,
history and checkpoint commitments still use the existing Rust
read-only verifier. Previous schema 1 and 2 observations remain
supported without attributing networked certification to them.
Use the same delegated reproduction command above with a new output
path. Neither local certification nor these historical receipts
are proof of current global quorum, actual Minecraft account
authentication, or canonical runtime write authority.

The retained [schema-3 evidence](evidence/2026-11-26-combined-networked-cut.json)
has SHA-256 `34169bcaa750c9cb259d0c97233fa8265353db241170bf922462b1a1ecc1e8d0`.
The corrected delegated namespace experiment exited 0, with all
worker scopes released and exact artifact hashes matching. Independent
verification checked 13 signed decisions, three encrypted checkpoint
collections, six local validator replay observations, four exact signed
durable sync receipts, four client receipts, four worker processes and
nine role observations. The intermediate majority failure preserved the
collector's pending target and the remote's unchanged accepted base,
and no staged or pre-sync client publication was permitted. The
departed founder was not contacted after the first tick. This does not
qualify sustained service, independent ownership or real Minecraft. All 137 research regression tests passed, including real-CLI
journal checks and the new evidence/receipt mutation tests. Workflow,
release-version and diff checks passed. The shared ordinary client path
also passed the existing #61 actual crash/replay experiment with its
unchanged ten-second default: [retained regression evidence](evidence/2026-11-26-transition-networked-cut-regression.json),
SHA-256 `536b1b65bca7f8e9719bdcda622137f964f30d7c265a722c91937222b709da25`.
Four accepted ticks, eight unique commands, 14 client sessions, eight
crossings, two real writer SIGKILL faults, two replay refusals and two
recovered receipts independently verify. No Rust implementation was
changed; the network experiment used the previously verified binary.

## Non-destructive certified staging restoration

The recovery adapter now restores an exact certified prefix from an
audited, locally advanced staging database, rather than rolling the
source back in place. The caller explicitly selects an existing signed
private authority head, the exact source world/generation/tick/root/cut,
and a new destination, and must have retired the source runtime.
The retirement assertion is a trusted-controller precondition, not
remote process attestation. The actual isolated run below checks
worker process exit and releases its kernel scope before restoration.

The adapter checks the selected current validator's signed journal,
exact world, policy, compatibility, fence and head, requires no pending
vote, obtains the retained bytes with the existing recovery-copy
operation, and verifies their cryptographic commitment. It opens the
source in SQLite read-only mode with query-only enforcement, audits
the complete local state, and holds a consistent read transaction
through restoration. It independently replays the exact certified
prefix from genesis, including the committed input batch and receipts.
The resulting canonical checkpoint must equal the retained signed
bytes. A missing, corrupt, behind, or divergent source cannot be
silently repaired. In particular, no worker receipt hash or epoch
history is fabricated from a checkpoint that lacks those fields.

Only the actual local commit rows, command results, batches and
historical public epoch records through the certified cut are copied.
The abandoned suffix and pending commands are excluded; no old private
worker session key is copied. A newly created private database reserves
a consumed quarantine generation strictly above the original source
generation. A worker runtime cannot claim it. An explicit later
rotation and separately reviewed role/placement checks are required
to resume execution. This is a trusted local isolation boundary, not
an automatically installed distributed fence over other copies.

The new SQLite file is constructed with FULL durability under a
private temporary directory, independently audited, closed, reopened
read-only and compared to the exact certified checkpoint. The
validator pin is checked again immediately before publication. A
same-filesystem hard link publishes the complete file without
replacement, followed by destination-directory fsync. Existing
destination files, symlinks or SQLite sidecars cause refusal. A real
SIGKILL before publication leaves the source intact and no destination;
a private scratch directory may remain for manual cleanup. A failure
after the link is created may leave a complete destination; it must
be inspected rather than overwritten. The adapter never switches
runtime paths or automatically rolls back a signed journal. A race
after the last local authority check is not ruled out by a global
lock; the client gate still rechecks every state-bearing access.

### Actual isolated restoration run

[Retained evidence](evidence/2026-11-26-certified-staging-restore.json),
SHA-256 `d411dc34460d7d1680e7f94391813214c354e484e2b15f123d163cb05d073b87`.

The separate `certified_restore_lab.py` run exercised actual
authenticated reference TCP clients, policy-bound worker processes and
Linux cgroup caps in isolated namespaces:

| Stage | Local generation | Tick | Client outcome |
| --- | ---: | ---: | --- |
| First certified execution | 1 | 1 | Committed receipt |
| Founder departure and abandoned advance | 2 | 2 | No acknowledgment |
| Separate certified restore | 3 (quarantine) | 1 | Exact prior receipt recovered |
| Fresh measured worker and new execution | 4 | 2 | Replacement certified receipt |

The original database's before/after SHA-256 matched exactly. The
restored history retained its original genesis and tick-one commit
rows and public epoch record, not the abandoned tick-two row. Its
quarantine generation could not be claimed, old worker receipts were
refused and the discarded command had no certified receipt. A fresh
reference client recovered the exact tick-one result over TCP; a
different command subsequently committed at tick two after encrypted
collection and signed checkpoint synchronization. The departed
founder's journal remained unchanged. The independent verifier
replayed all three local candidate transitions, verified five signed
governance decisions and three sync signatures, and checked three
worker processes, four TCP client sessions and seven reference role
observations. All scopes exited, and recorded source and binary
artifact hashes matched. These are local observations, not independent
physical-host attestation or real Minecraft recovery.

Additional real-CLI regressions exercise a certified nonzero prefix,
genesis restore after a process kill, exact receipt recovery,
uncertified suffix and pending input removal, explicit rotation,
source-pin and no-clobber checks, divergent history refusal, a
governance advance during restoration, and a pending vote that cannot
be reset. The original database remains untouched in each case. All 143 research regression tests passed, including the explicitly enabled signed-journal restoration tests, retained evidence mutations and read-only authority checks. Workflow-policy, release-version and diff checks passed locally. No Rust code was changed; the actual network experiment used the previously verified CLI binary.

Reproduce with a new output path and the previously built CLI:

```sh
systemd-run --user --scope --quiet \
  --unit=swarmcraft-memory-cap-test-combined-68-restore-$$ \
  -p 'Delegate=memory cpu' -p RuntimeMaxSec=240 \
  env SWARMCRAFT_TEST_CPU_DELEGATION=1 \
  bash scripts/e2e/linux_cgroup_memory_gate.sh \
  unshare --user --map-root-user --net \
  python3 research/distributed-simulation/certified_restore_lab.py \
  --cli "$PWD/target/debug/swarmcraft" --output /tmp/sc68-restore-new.json
python3 research/distributed-simulation/verify_certified_restore_evidence.py \
  /tmp/sc68-restore-new.json --cli target/debug/swarmcraft
```

The operation still requires an intact local provenance prefix, a
current verified private reference journal and explicit source
retirement. It does not recover a missing local database from opaque
checkpoint bytes alone, resolve pending split votes, prove current
global quorum, or authorize canonical Minecraft state restoration.

## Durable managed contribution and consent withdrawal

The #62 contribution lifecycle is now composed with the #68 policy-
bound worker and certified client boundary in a distinct SQLite
application/ruleset. The managed database cannot be opened through the
ordinary combined or contribution authority. The fixed worker-slot
identity mapping is checked against the durable configuration on reopen.
Every positive offer is an explicit local consent revision and cap.
Before reserving a probe scope, the controller checks the exact private
accepted checkpoint from its local signed validator journal, requires
no unresolved vote, and obtains a fresh one-use signed volunteer role
proof for every selected identity. Zero consent starts no scope;
positive local consent alone cannot admit an ineligible identity.
The existing role proof offers 512 MiB of reference volunteer memory,
and the actual smaller local process cap must fit that budget. The
CPU cap is matched exactly. This is not production resource consent
or live remote attestation.

The existing measured-placement planner chooses only a complete
feasible assignment. The same durable local write serialization
protects consent revisions, probe scope, generation admission,
one-time claim, input sealing and the worker candidate transaction.
The new scope records retain the reviewed head, exact base cut,
original offers, consumed role observations and measured samples.
Only the original in-process controller can admit or use that scope;
a restart has no reusable resource-guard callback or controller token.
It must fence and prove cleanup before starting a new scope. No
indefinite service installation or remote cleanup inference is made.

At claim, seal and candidate commit the managed authority verifies
the pinned local signed head and exact local base, unchanged consent,
fresh resource samples and a trusted current resource guard. The
guard checks actual process ownership, affinity, CPU/memory controls,
headroom and the feasibility of the exact current region placement.
A resource/authority fault rolls back the candidate transaction and
invalidates the controller before durably fencing the affected scope.
A stale independent contender cannot attach a new guard or claim the
prior controller's scope. A rejected stale receipt alone does not
fence a valid new scope. Local consent withdrawal serializes against
the same SQLite transaction, fences before cleanup, and cannot reset
an accepted tick or remove an already sealed input.

Each admitted scope may commit **at most one** staged tick. Its FULL
SQLite transaction also records the exact staged cut and transitions
to draining. No subsequent tick may use those grants. Cleanup is
explicit: the actual harness verifies worker exit, empty/removal of
all worker cgroups and removal of private network links. Only then
does it release the durable scope. The managed certification adapter
refuses an uncleaned scope, missing staged result, changed offer,
changed governance pin or different staged cut. A released exact
candidate uses the existing encrypted collection and sync protocol
with no offline fallback, then the exact client publication check.
No additional compute occurs during this slower certificate exchange.

### Actual isolated managed run

[Retained evidence](evidence/2026-11-26-managed-ownerless-contribution.json),
SHA-256 `39479cdcb84197d3965821ff66a9dc607a37d3295048a967365fd00a6396506e`.

| Stage | Scope | Generation | Workers | Outcome |
| --- | ---: | ---: | ---: | --- |
| First certified execution | 1 | 2 | 2 | Tick 1 acknowledged |
| Local consent withdrawal after candidate | 2 | 3 | 2 | Commit refused; tick 1 retained |
| Restart and scale-in | 3 | 4 | 1 | Same sealed command certified at tick 2 |

The founder departed before the first runtime stage. The experiment
used actual policy-authenticated TCP workers, delegated Linux kernel
caps, fresh signed volunteer and player-role observations, and
independent reference TCP clients. The withdrawing worker's prepared
receipts could not promote the tentative state. The client received
no acknowledgment, its sealed input was preserved, new overlapping
probes were refused, and a second independent connection had
durably recorded the new consent revision. All five worker processes
exited and all three scopes were released. Reopening retained
identical state, command journal, contribution status and event
history. A newly attempted probe of the now signed-revoked worker
was refused. A new authorized single worker at a reduced cap
replayed the same pending command, refused old worker receipts,
and certified the next exact tick without the founder.

Independent verification checked seven signed decisions, two signed
sync receipts, eight role observations, measured complete placements,
eight guard observations, both certified ticks, and the refused
withdrawal attempt. The first experimental attempt stopped after
the expected commit refusal because its assertion expected a
contribution-fence error for a subsequent seal; the existing
retired-cut guard refused first. The corrected run passed and
all recorded source/binary artifact hashes matched. This is not
a sustained-performance result or independent physical-host proof. All 149 research regression tests passed, including real signed-journal consent, resource-fault fencing, restart and revoked-role scenarios. The strengthened lifecycle-event evidence verifier and its two mutation/replay tests passed again after the full suite reached them. Workflow-policy, release-version and diff checks passed. No Rust implementation changed.

Reproduce using a new output path and the previously built CLI:

```sh
systemd-run --user --scope --quiet \
  --unit=swarmcraft-memory-cap-test-combined-68-managed-$$ \
  -p 'Delegate=memory cpu' -p RuntimeMaxSec=240 \
  env SWARMCRAFT_TEST_CPU_DELEGATION=1 \
  bash scripts/e2e/linux_cgroup_memory_gate.sh \
  unshare --user --map-root-user --net \
  python3 research/distributed-simulation/managed_ownerless_lab.py \
  --cli "$PWD/target/debug/swarmcraft" --output /tmp/sc68-managed-new.json
python3 research/distributed-simulation/verify_managed_ownerless_evidence.py \
  /tmp/sc68-managed-new.json --cli target/debug/swarmcraft
```

The controller still owns the disposable validator/worker identities
and supplies trusted measurements and cleanup proof. The local SQLite
guard and Rust journal check are not a distributed atomic transaction:
another governance decision may supersede the local check, and local
consent changes during an already initiated network decision cannot
undo a signed vote or accepted certificate. The scope is already
retired before voting. Current player access is checked by the
existing client gate; independently revalidating already sealed
inputs against a changed player invitation set remains a separate
fail-closed integration requirement. The preceding non-destructive
restore operation also still targets the original combined database
type, not this new managed lifecycle schema. Restart-safe
certification/reconciliation of a locally staged but uncertified
managed cut and consent-preserving separate restore require further
integration. No live authority or actual Minecraft gameplay is enabled.

Run on Linux with mapped user/network namespace support and a delegated
user systemd scope (from the repository root, with the tested CLI built):

```sh
systemd-run --user --scope --quiet \
  --unit=swarmcraft-memory-cap-test-combined-68-local-$$ \
  -p 'Delegate=memory cpu' -p RuntimeMaxSec=240 \
  env SWARMCRAFT_TEST_CPU_DELEGATION=1 \
  bash scripts/e2e/linux_cgroup_memory_gate.sh \
  unshare --user --map-root-user --net \
  python3 research/distributed-simulation/combined_ownerless_lab.py \
  --cli "$PWD/target/debug/swarmcraft" --output /tmp/new-combined-ownerless.json

python3 research/distributed-simulation/verify_combined_ownerless_evidence.py \
  /tmp/new-combined-ownerless.json --cli target/debug/swarmcraft
python3 -m unittest discover -s research/distributed-simulation -p 'test_*.py'
```

## Acceptance boundary

**Local combined reference qualification passed; #68 field acceptance
remains unverified.** The experiment runs on one physical machine with
disposable identities controlled by one trusted orchestrator. It uses a
small custom deterministic simulation, not Minecraft Java/Fabric
distributed gameplay. The three ticks do not establish sustained 20 Hz
throughput; the #63 full durable service target remains failed.
The separate #65 Fabric permission evidence does not turn this combined
run into an authenticated Minecraft login or live ownerless game session.

Before production activation, implement and verify independent live
quorum transport and canonical publication, current signed resource
consent and actual admission/permit integration, real Minecraft account
mapping and a supported region-aware runtime. Then repeat combined field
qualification across independently controlled physical hosts, including
actual founder exit, cross-region movement, worker withdrawal, quorum
loss, partition/recovery, stale-session fencing, and sustained service
at the required target. Do not enable ownerless or distributed Minecraft
gameplay on the basis of this reference result.
