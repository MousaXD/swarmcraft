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
