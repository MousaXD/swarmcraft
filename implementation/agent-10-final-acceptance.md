# Agent 10 — Final Whole-Product Acceptance

## Status

STATUS: IN PROGRESS

BRANCH: `finalize/audit-remediation-v1`

BASE INTEGRATION SHA: `51a79f2e60d8274f9c96a9c0a23d7409fc0adc19`

MAIN-HISTORY RECONCILIATION MERGE: `9fba9883b6aa548e60fd7145a2d630d230e4346f`

FINAL CANDIDATE SHA: pending this finalization commit

ACCEPTANCE REFRESH DATE: 2026-09-22

## Mission

Prove the remediated SwarmCraft product on one composed tree, close any genuine integration defect exposed by that proof, and leave a protected path to `main` without weakening quorum, fencing, runtime verification, provider verification, release validation, or repository governance.

Agent 10 is an integration/acceptance role. The only production change made during final acceptance is the discovery explicit-locator warmup hardening described below, which was required because the frozen composed candidate exposed a nondeterministic network regression.

## Dependency gate

OPEN.

Agents 1 through 9 are integrated into `integration/audit-remediation-v1`. The master ledger records the exact validated implementation heads, source closure heads, integration PRs, merge commits, and validation evidence.

Composed integration history includes:

- Agent 1 merge `a0e0dec659d0b1eb21f9be34c44730edc6ff3984`;
- Agent 2 merge `6e70a0774d7e021cc57681705ccef4620265ce3d`;
- Agent 3 merge `602f6f1cfed46e457a1fccbf8d6d2df79e3f1ab5`;
- Agent 4 merge `aac089a72c68f72000a8610d2288721317cf0ae4`;
- Agent 5 merge `264b74b0e62d2d5201deb91b2069372c90c754ac`;
- Agent 6 merge `ca6556fc89c29147930f7f17832e1f89c30cae77` plus follow-up `5911dce8eb13d71ad52d670129c4ddb955137e20`;
- Agent 7 merge `7babf3b941f84cbdba1a0ffd3ac0a20628358051`;
- Agent 9 merge `efde7ecb996ead8d414378e7876354b731e4a963`;
- Agent 8 merge `51a79f2e60d8274f9c96a9c0a23d7409fc0adc19`.

The two newer `main` commits add and then remove an accidental test file and therefore have zero net tree delta. Their legitimate history was merged into this finalization branch at `9fba9883b6aa548e60fd7145a2d630d230e4346f` before final validation.

## Frozen composed-candidate evidence

Agent 10 explicitly dispatched the two authoritative workflows on integration SHA `51a79f2e60d8274f9c96a9c0a23d7409fc0adc19`:

- Required Validation run `35400412573`;
- Main Desktop Installers run `35400416280`.

The frozen candidate exposed one real acceptance issue in `Core CI / Rust (ubuntu-latest)`, job `105778780344`: `discovery_network_freshness::malicious_and_stale_providers_cannot_win_browse_or_exact_resolve` could fail because an explicit hostile locator did not always participate before the query completed.

The failure did not show a hostile result being accepted. It showed that the test depended on libp2p scheduling. Inspection found a production-adjacent readiness weakness in `warm_explicit_locators`: raw outbound dial errors are consumed inside `DiscoveryNode::next_event`, so the warmup loop could spend its entire bounded window without re-driving an unauthenticated explicit locator.

## Final acceptance repair

`crates/swarm-cli/src/discovery.rs` now periodically re-drives every unauthenticated explicit locator inside the existing bounded warmup. `DiscoveryNode` still suppresses duplicate pending dials and the outer discovery timeout still makes unreachable locators bounded/nonfatal.

`crates/swarm-cli/tests/discovery_network_freshness.rs` now verifies security outcomes rather than scheduler choreography. The network regression requires the canonical live authority to win from a bounded topology containing current, voter, stale and malformed locator peers. Deterministic cryptographic tests in `discovery_freshness.rs` continue to prove malformed proofs, unrelated attackers, stale partitions, replay and wrong-context evidence fail closed.

Local finalization validation before push:

- `cargo fmt --all -- --check` — PASS;
- `cargo clippy -p swarm-cli --test discovery_network_freshness --locked -- -D warnings` — PASS;
- full `cargo test -p swarm-cli --test discovery_network_freshness --locked -- --nocapture` — PASS five consecutive rounds, 15/15 network tests green;
- `git diff --check` — PASS.

### First exact finalization head — `f7b92b37014bf54ee294b411c0031b931ed02dd1`

The first pushed finalization candidate was not accepted. Required Validation run `35684977478` failed only the macOS x86_64 Desktop package lane, and soak-enabled Main Desktop Installers run `35684985696` failed only the nested macOS Rust lane.

- macOS Rust job `106609780724` timed out in `simultaneous_bidirectional_dials_converge_on_one_authenticated_connection`. The test initiated one symmetric dial race and then only consumed events. When both initial outbound attempts lost the startup race on the slower macOS runner, `DiscoveryNode::next_event` correctly cleared failed pending dials but the test never re-drove them. The regression now re-drives unauthenticated symmetric peers every 250 ms inside the existing bounded convergence window and allows 10 seconds for final duplicate-connection settlement. Strict Clippy passes; eight focused convergence rounds plus the complete network suite pass locally.
- macOS x86_64 Desktop package job `106609752530` successfully compiled the full release application, then failed inside Tauri's generated `bundle_dmg.sh`; GitHub cleanup reported an orphan `diskimages-help` process. This is a transient Apple disk-image tooling failure after successful product compilation. Both permanent macOS packaging paths now retry the DMG bundling step exactly once after clearing only transient DMG bundle state. A reproducible product/package failure still fails the second attempt.

These changes require a new exact candidate and full protected validation; `f7b92b3` is not a release candidate.

## Whole-product acceptance coverage

The permanent exact-head workflow graph covers the required journey and adversarial seams rather than relying on isolated unit tests:

- clean data directory and identity/world creation;
- managed Java/Minecraft/Fabric installation from official sources;
- explicit EULA refusal then acceptance;
- real authenticated Minecraft/Fabric launch, save, stop, restart and canonical snapshot advancement;
- invite/join and immediate signed snapshot replication;
- canonical provider/package contracts and server-mod readiness;
- real import-lock and runtime lifecycle fencing;
- three- and five-peer consensus partition regressions;
- old-authority/majority recovery behavior and no unsafe automatic Solo fallback;
- previous-generation/current-authority/history rejection;
- cross-process recovery-promise persistence/non-equivocation;
- authenticated transport replay/admission/privacy hardening;
- traversal/credential/origin/metadata provider hardening;
- three-daemon hard-kill authority recovery;
- abandoned-successor higher-round recovery;
- sleep-record-bound multi-member quorum wake with exact snapshot lineage;
- stale authority fencing after recovery;
- corrupt sleep state fail-closed behavior;
- Desktop module/render/keyboard/player-journey tests;
- RustSec audits, fuzz smoke and release identity checks;
- impaired QUIC resume and authenticated admission-backoff regression;
- native Linux, Windows, macOS ARM64 and macOS x86_64 Desktop packaging;
- soak-enabled interrupted multi-gigabyte QUIC transfer with durable identity/restart/resume semantics.

## Product boundary preserved

The accepted product contract does not invent availability that the protocol cannot safely provide.

- A two-voter Alice/Bob world remains `BlockedByQuorum` after one voter disappears. One-of-two crash failover is intentionally unsupported.
- Multi-member wake is implemented only with a surviving canonical quorum bound to the signed sleep generation and exact sleeping snapshot. No first-click-wins wake exists.
- Automatic safe successor runtime startup is implemented after a valid authority transition.
- Seamless automatic Minecraft client redirection/reconnection is not claimed complete.
- Representative universal NAT/carrier certification is not claimed complete.
- SwarmCraft remains an advanced technical preview, not a claim of universal production readiness.

## Repository governance

Agent 8 closed the governance gap before integration:

- ruleset `23573424` strictly requires `Required validation gate` on `main` and `integration/audit-remediation-v1`;
- ruleset `23678402` requires the protected PR/safety path on `main`;
- neither ruleset has a bypass actor;
- release publication consumes exact-SHA required validation rather than rebuilding an unrelated mutable ref;
- the soak-enabled Main Desktop Installers workflow is the release-path proof for large interrupted transfer plus platform packages.

## Final gate still required

Before changing this ledger to `GOAL REACHED`, the literal finalization head must have:

- aggregate `Required validation gate` — SUCCESS;
- soak-enabled `Main Desktop Installers` — SUCCESS;
- no failed required job on that exact SHA;
- clean source tree and exact remote head identity.

After that exact head is merged into `integration/audit-remediation-v1`, the protected integration-to-`main` PR must itself pass the required validation policy before merge. Post-merge `main` CI must then be checked for a clean terminal result.

## Remaining work

1. Commit and push this finalization tree.
2. Run exact-head Required Validation and Main Desktop Installers.
3. If both are green, record their exact run IDs and change this ledger to `GOAL REACHED` in the final documentation-only closure commit.
4. Revalidate that closure commit under the required gate.
5. Merge the finalization PR into `integration/audit-remediation-v1`.
6. Open the protected integration-to-`main` PR, pass its required checks, merge, and verify post-merge `main` CI.

## Agent final statement

GOAL NOT REACHED
