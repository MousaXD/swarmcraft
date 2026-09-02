# Agent 10 — Final Whole-Product Acceptance

## Status

STATUS: BLOCKED ON AGENTS 1-9

BRANCH: `integration/audit-remediation-v1`

STARTING SHA: exact integration head after Agents 1-9 are integrated

CURRENT HEAD SHA: pending

## Mission

Prove whether the remediated SwarmCraft product actually satisfies the intended launcher/player journey and adversarial safety requirements on one exact integrated SHA.

Agent 10 is an integration/acceptance agent, not a normal feature-development agent. Make production fixes only for genuine integration defects that prevent acceptance and record every such change here.

## Dependencies

Required before starting:

- Agents 1-9 integrated into `integration/audit-remediation-v1`
- every Agent 1-9 ledger says `READY FOR INTEGRATION` before integration and records exact source SHA
- integration README contains each consumed head

## Required acceptance journey

- [ ] Verify exact integration SHA and clean working tree/remote branch truth.
- [ ] Clean Player A data directory and identity creation.
- [ ] Choose a supported authoritative Minecraft version.
- [ ] Choose compatible Fabric loader/adapter tuple.
- [ ] Select provider mods using Modrinth/CurseForge or deterministic permitted fixtures representing exact provider contracts.
- [ ] Create canonical world through the intended Desktop/CLI contract.
- [ ] Install managed runtime.
- [ ] Explicit EULA refusal then acceptance behavior.
- [ ] Launch real Minecraft/Fabric runtime.
- [ ] Mutate real world state.
- [ ] Safe save/checkpoint/stop.
- [ ] Generate invite without manual bootstrap requirement for the normal path.
- [ ] Clean Player B identity/state.
- [ ] Consume invite and join canonical membership.
- [ ] Replicate exact signed snapshot/state.
- [ ] Reacquire exact permitted provider artifacts on Player B.
- [ ] Install compatible Player B runtime.
- [ ] Exercise authority failure under the supported voter topology.
- [ ] Recover exactly one safe successor.
- [ ] Continue/checkpoint on successor.
- [ ] Stop all hosts into safe sleeping state.
- [ ] Restart backends.
- [ ] Run safe multi-member wake bound to durable sleep state.
- [ ] Restore exact canonical snapshot.
- [ ] Relaunch real Minecraft/Fabric runtime.
- [ ] Prove canonical world identity/history remains valid.

## Mandatory adversarial regressions

- [ ] Divergent membership partition cannot create two valid writable quorums.
- [ ] Old-authority minority cannot enter writable Solo while majority recovers.
- [ ] Previous-epoch authority membership replay is rejected.
- [ ] Non-authority WorldConfig write is rejected.
- [ ] Snapshot wrong-parent/jump/same-sequence conflict is rejected.
- [ ] Duplicate daemon/shared-data-root recovery promise equivocation is prevented.
- [ ] Captured PeerHello replay on another transport identity is rejected.
- [ ] Stranger/removed/banned peer cannot read private world metadata.
- [ ] Provider filename traversal cannot escape staging.
- [ ] CurseForge API credential cannot cross origin or appear in child argv.
- [ ] Oversized provider metadata fails within explicit bounds.
- [ ] Import while a real Minecraft process owns the source is rejected.
- [ ] Runtime supervisor hard death cannot leave an unowned writable Java authority runtime.
- [ ] Desktop exact module graph initializes with zero uncaught exceptions.
- [ ] Provider/discovery/canonical Create UI paths invoke intended Tauri commands.
- [ ] Release publication is blocked while same-SHA required validation is running or failing.

## Evidence requirements

For every acceptance item record:

- exact SHA
- exact command/workflow/test
- result
- relevant logs/artifacts/run IDs
- whether evidence is real process/live runtime versus mock/unit

Do not treat isolated unit tests as proof of the full journey.

## Work completed

None yet.

## Tests run

| Test / Journey Step | Result | Exact SHA | Evidence |
|---|---|---|---|
| None yet | - | - | - |

## Blockers

Agents 1-9 are not yet integrated.

## Remaining work

All required journey and adversarial items.

## Handoff / final verdict

After completing acceptance, update `implementation/README.md` with the exact accepted or failed integration SHA and evidence summary.

Then finish this ledger and your final response with exactly one of:

`GOAL REACHED`

or

`GOAL NOT REACHED`

If `GOAL REACHED`, freeze the exact candidate SHA and trigger re-audit by Auditors 0-10 plus the Final Audit Integrator against that exact SHA.

## Agent final statement

GOAL NOT REACHED
