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
