# SwarmCraft Audit Remediation Implementation Plan

## Purpose

This directory is the authoritative implementation and handoff ledger for fixing the findings in `audits/FINAL-AUDIT.md`.

Chat history is not authoritative. Every implementation agent must leave enough exact repository state in these Markdown files for a fresh agent session to resume safely.

Every agent must record:

- exact branch and SHA
- dependencies consumed
- findings owned
- implementation milestones completed
- files and contracts changed
- tests run and their result
- blockers
- remaining work
- exact final handoff SHA

If code was implemented but the assigned ledger does not record it, the work is not considered handed off.

## Campaign baseline

- Repository: `MousaXD/swarmcraft`
- Integration branch: `integration/audit-remediation-v1`
- Campaign base branch: `main`
- Campaign base SHA: `b4bab08562cf0eb53763674407375b023e1d0858`
- Audit source: `audits/FINAL-AUDIT.md`
- Original audited product SHA: `354be3b1066428ecab6987590b7c7dbd80fe0870`

The campaign base is the 0.5.0 metadata promotion on top of the audited product tree. The final audit established that the post-audit merge changed release/version metadata and lockfiles rather than the production paths behind the confirmed CRITICAL/HIGH findings. Therefore the remediation findings remain applicable to this campaign base.

Do not silently rebase this campaign onto a newer `main`. If a newer base is intentionally adopted, record the exact decision and SHA here first.

## Directory layout

- `implementation/README.md` — master campaign ledger
- `implementation/agent-1-consensus.md`
- `implementation/agent-2-protocol.md`
- `implementation/agent-3-storage.md`
- `implementation/agent-4-network.md`
- `implementation/agent-5-supply-chain.md`
- `implementation/agent-6-runtime.md`
- `implementation/agent-7-desktop.md`
- `implementation/agent-8-ci-release.md`
- `implementation/agent-9-recovery-wake.md`
- `implementation/agent-10-final-acceptance.md`

Each implementation agent owns exactly one ledger. Agents must not edit another agent's ledger. The integration coordinator may update this master table when consuming validated heads.

## Mandatory agent workflow

### 1. Read before editing

Every agent must read, in order:

1. this file
2. its assigned `implementation/agent-N-*.md`
3. `audits/FINAL-AUDIT.md`
4. the underlying auditor reports referenced by the assigned findings
5. every dependency ledger named in the agent file

### 2. Verify Git state

Before editing production code:

- fetch remote state
- verify repository and assigned branch
- verify the expected starting/dependency SHA
- inspect whether a previous session already pushed legitimate newer work
- never reset or discard legitimate newer work

### 3. Record start state

Update the assigned ledger with:

- `STATUS: IN PROGRESS`
- branch
- starting SHA
- current head SHA
- dependency heads
- date/time if useful

### 4. Work in milestones

Do not create one giant undocumented implementation. Break work into meaningful milestones. After every substantial milestone:

1. update the ledger
2. record behavior implemented
3. record files/contracts changed
4. record tests run
5. record remaining work/blockers
6. commit implementation plus ledger together when practical
7. push the branch

The branch and ledger must not materially disagree.

### 5. Handoff discipline

An agent may declare `READY FOR INTEGRATION` only when:

- every owned CRITICAL/HIGH requirement is implemented or explicitly resolved by an approved design decision
- required regression tests exist
- domain validation is green
- exact head SHA is recorded
- known integration conflict areas are documented
- no unrelated work is mixed into the branch

Final ledger status must be exactly one of:

- `READY FOR INTEGRATION`
- `BLOCKED`
- `NOT COMPLETE`

## Commit discipline

Implementation commits should include the assigned ledger update whenever practical.

Examples:

- `fix(consensus): commit membership generations`
- `fix(consensus): fence stale membership quorums`
- `test(consensus): cover divergent membership partition`
- `docs(progress): finalize agent 1 handoff`

Do not hide progress only in commit messages.

## Master status table

| Agent | Domain | Status | Branch | Exact Head | Integrated | Notes |
|---|---|---|---|---|---|---|
| 1 | Consensus configuration safety | INTEGRATED | `fix/agent-1-consensus` | `67493374544d91ad7bbb36be17e9312adb5654f6` | Yes | Exact-head run `33619420045` SUCCESS; merge `a0e0dec659d0b1eb21f9be34c44730edc6ff3984` |
| 2 | Protocol authorization/history | NOT STARTED — UNBLOCKED | `fix/agent-2-protocol` | - | No | Wave 2; branch from newest integration head |
| 3 | Storage transactional integrity | NOT STARTED | `fix/agent-3-storage` | - | No | Wave 1 |
| 4 | Network authentication/privacy | NOT STARTED | `fix/agent-4-network` | - | No | Wave 1 |
| 5 | Package/provider security | NOT STARTED | `fix/agent-5-supply-chain` | - | No | Wave 1 |
| 6 | Minecraft/runtime lifecycle | NOT STARTED | `fix/agent-6-runtime` | - | No | Wave 1 |
| 7 | Desktop player journey | NOT STARTED | `fix/agent-7-desktop` | - | No | Wave 1 |
| 8 | CI/release governance | NOT STARTED | `fix/agent-8-ci-release` | - | No | Wave 1 |
| 9 | Recovery/wake completion | BLOCKED ON AGENTS 1 + 6 | `fix/agent-9-recovery-wake` | - | No | Wave 2 |
| 10 | Final acceptance | BLOCKED | `integration/audit-remediation-v1` | - | No | After 1-9 integration |

## Agent allocation

### Agent 1 — Consensus configuration safety

Owns `FINAL-001`, `FINAL-002`, `FINAL-006`, `FINAL-039`, `FINAL-045`.

Core goals: committed membership generations/joint consensus, prevent stale/new voter sets from forming independent quorums, eliminate unsafe automatic Solo versus majority recovery split brain, value-preserving higher recovery rounds, strict counter exhaustion handling, and adversarial partition coverage.

### Agent 2 — Protocol authorization and history

Owns `FINAL-004`, `FINAL-005`, `FINAL-025`, `FINAL-026`.

Core goals: current-authority binding, stale membership rejection, direct-parent and exact-sequence rules, snapshot history continuity, protocol-version fail-closed behavior, and canonical collection representation.

### Agent 3 — Storage transactional integrity

Owns `FINAL-008`, `FINAL-009`, `FINAL-010`, `FINAL-011`, `FINAL-027`, `FINAL-041`.

Core goals: durable canonical head, immutable snapshot slots, cross-process control locking, generation-fenced snapshot commits, portable path identity, transactional restore, and durability consistency.

### Agent 4 — Network authentication and privacy

Owns `FINAL-012`, `FINAL-013`, `FINAL-028`, `FINAL-029`, `FINAL-030`, `FINAL-040`.

Core goals: live connection-bound proof of possession, exhaustive request authorization, private-world confidentiality, authenticated discovery authority, admission/rate limits, presence privacy, and address policy hardening.

### Agent 5 — Package/provider security

Owns `FINAL-003`, `FINAL-017`, `FINAL-018`, `FINAL-019`, `FINAL-034`.

Core goals: server-owned staging, filename traversal prevention, credential-safe CurseForge HTTP policy, canonical retrieval consistency, bounded metadata, and redirect/host allowlists.

### Agent 6 — Minecraft/runtime lifecycle

Owns `FINAL-014`, `FINAL-015`, `FINAL-016`, `FINAL-032`.

Core goals: import quiescence, authoritative adapter support matrix, supervisor/controller liveness fencing, orphan Java handling, and retained runtime diagnostics.

### Agent 7 — Desktop player journey

Owns `FINAL-020`, `FINAL-031`, `FINAL-042`, and coordinates UX for `FINAL-023`/`FINAL-024` after backend semantics exist.

Core goals: fix launcher initialization, Import Tauri contracts, canonical Create/provider/discovery wiring, partial-success handling, browser module smoke, exact-size render and keyboard/focus coverage.

### Agent 8 — CI/release governance

Owns `FINAL-021`, `FINAL-022`, `FINAL-035`, `FINAL-036`, `FINAL-038`, `FINAL-044`, `FINAL-046`.

Core goals: same-SHA release gating, immutable Action pins/minimum token permissions, direct Desktop/provider gates, tag/version/signing policy, branch rules, and cleanup of obsolete validation machinery only after replacement.

### Agent 9 — Recovery/wake product completion

Owns `FINAL-007`, `FINAL-023`, `FINAL-024`.

Core goals: supported voter topology, host-ready recovery candidacy, sleep-record-bound quorum wake, explicit two-voter behavior, and safe restore/relaunch after multi-member sleeping state.

### Agent 10 — Final acceptance

Runs only after Agents 1-9 are integrated. It proves the release-blocking whole-product journey and the adversarial regressions named in its ledger. It does not redesign normal product features.

## Execution waves

### Wave 1

Run in parallel from campaign base `b4bab08562cf0eb53763674407375b023e1d0858`:

- Agent 1 Consensus
- Agent 3 Storage
- Agent 4 Network
- Agent 5 Supply Chain
- Agent 6 Runtime
- Agent 7 Desktop
- Agent 8 CI/Release

### Wave 2

After Agent 1 is integrated, Agent 2 branches from the newest exact integration head.

After Agents 1 and 6 are integrated, Agent 9 branches from the newest exact integration head.

Do not start Agents 2 or 9 from the original baseline if their dependency heads are already integrated.

## Integration policy

All validated heads are integrated into `integration/audit-remediation-v1`.

Do not merge implementation branches directly into `main`.

Before consuming a branch:

- ledger says `READY FOR INTEGRATION`
- exact source SHA is recorded
- required tests are green
- branch contains no unrelated work

After every integration, update this README with:

- agent
- integrated source SHA
- integration commit SHA
- resulting integration head SHA
- validation run/results
- conflicts resolved

### Integration history

#### Agent 1 — Consensus configuration safety

- Integrated source production SHA: `67493374544d91ad7bbb36be17e9312adb5654f6`
- Source ledger head: `8bcbef2bb24478c5a9938872643c7103ba8a4573` (docs-only after validated production SHA)
- Integration PR: `#63`
- Integration commit/resulting production integration head: `a0e0dec659d0b1eb21f9be34c44730edc6ff3984`
- Validation: Agent 1 exact-head regression run `33619420045` — SUCCESS on `67493374544d91ad7bbb36be17e9312adb5654f6`
- Merge-tree proof: comparing Agent 1 ledger head `8bcbef2...` to integration commit `a0e0dec...` changes only `implementation/agent-10-final-acceptance.md`; no Agent 1 production path differs from the validated branch tree.
- Conflicts resolved: none. The integration branch's only divergence since the common campaign-plan base was the Agent 10 dependency-gate ledger, so GitHub produced a clean two-parent merge without production conflict.
- Deferred composed proof: Agent 3 cross-process recovery-promise/non-equivocation and production transport/restart composition remains required when Agent 3 is integrated. This is not unfinished Agent 1-owned work and does not block Agent 2 from starting from the newest integration head.

## Final acceptance and re-audit

After Agents 1-9 are integrated, Agent 10 runs the complete release-blocking acceptance on one exact integration SHA.

Agent 10 must finish with exactly one of:

- `GOAL REACHED`
- `GOAL NOT REACHED`

Only after `GOAL REACHED`, freeze one exact candidate SHA and rerun Auditors 0-10 plus the Final Audit Integrator against exactly that SHA.

No release/publication should occur before the same fixed SHA passes the full required gate and final re-audit.

## Standard short invocation

Use this pattern to launch a fresh agent:

> You are Agent N. Read `implementation/README.md`, your assigned `implementation/agent-N-*.md`, `audits/FINAL-AUDIT.md`, and every dependency/audit file referenced by your ledger. Continue from the latest legitimate remote head of your assigned branch. Implement everything still marked incomplete. After every meaningful milestone, update your ledger with exact work completed, files/contracts changed, tests run, blockers, remaining work, and current head SHA, then commit and push the implementation plus ledger. Do not edit another agent's ledger. Do not claim READY until the exact-head validation required by your ledger is green. Finish with `READY FOR INTEGRATION`, `BLOCKED`, or `NOT COMPLETE`.

## Core rule

Chat is temporary. The repository ledger is permanent.
