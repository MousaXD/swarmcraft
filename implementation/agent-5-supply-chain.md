# Agent 5 — Package / Provider Security

## Status

STATUS: READY FOR INTEGRATION

READY FOR INTEGRATION: YES

BRANCH: `fix/agent-5-supply-chain`

LIVE STARTING HEAD FOR CLOSURE: `a6267844d04499debcca5290d46f49ef4357cc5e`

CURRENT AUTHORITATIVE INTEGRATION TIP: `39decc9e5eef7d2b28a0e103686aa79ebb744017`

MERGE BASE WITH CURRENT INTEGRATION: `a9736b159d9e9618a3ed8515c20e93f92c1453cb`

FINAL PRODUCTION / PERMANENT-TEST SHA: `51cd18cb5c937838ae7b4bcc2cb67f6027b1d6d6`

EXACT-HEAD VALIDATION SHA: `191df6eaa69a6b38e2202e6a2e3ef061417b345d`

EXACT-HEAD VALIDATION RUN: `34070673328` — SUCCESS

POST-VALIDATION HELPER CLEANUP SHA: `61e44078f32912fdc899aa7cba07036114a80df1`

INTEGRATED SHA: pending — Agent 5 must not merge itself.

## Mission

Make provider-controlled input incapable of escaping filesystem staging, leaking credentials, bypassing host trust boundaries, exhausting metadata handling, or promising reproducibility the runtime cannot actually deliver.

## Findings owned

- FINAL-003 — CLOSED: portable provider filename and staging containment hardening, opaque sessions, and symlink fencing are implemented and permanently tested.
- FINAL-017 — CLOSED: authenticated CurseForge API traffic is isolated from artifact traffic and credentials are exact-origin fenced and absent from subprocess/artifact paths.
- FINAL-018 — CLOSED: automatic `ProviderDownload` requires SHA-1/SHA-256/SHA-512; MD5-only CurseForge provenance remains `ManualRequired`.
- FINAL-019 — CLOSED: provider metadata bodies, headers, strings, depth, arrays, objects, URLs/IDs/filenames and relevant cardinalities are bounded before/around parsing.
- FINAL-034 — CLOSED: authenticated API redirect and artifact-host redirect trust boundaries are explicit and regression tested.

## Reconciled implementation checklist

The previous ledger was stale at closure start. The live `a6267844...` branch already contained the later provider-security implementation validated by run `33614873216`; the unchecked items below were therefore audited against current source rather than reimplemented blindly.

- [x] Stop accepting arbitrary provider download destination paths from the frontend.
- [x] Make backend construct provider staging paths from server-owned root plus opaque provider identity/session.
- [x] Validate provider filename as exactly one safe normal path component.
- [x] Reject absolute paths, prefixes, separators, `.`/`..`, UNC/drive forms, Windows backslash tricks, reserved device names, trailing-dot/space forms, and cross-platform separator tricks.
- [x] Derive publication paths only from validated opaque sessions plus validated provider/file components.
- [x] Reject malformed staging-session tokens and symlinked staging-session roots.
- [x] Separate the authenticated CurseForge API client from the artifact-download client.
- [x] Fence authenticated API redirects to the exact approved CurseForge API origin.
- [x] Prevent `SWARMCRAFT_CURSEFORGE_API_KEY` from entering child-process argv.
- [x] Prevent the CurseForge API key from being attached to artifact-download requests.
- [x] Keep the API credential out of frontend payload contracts and provider logging paths.
- [x] Define explicit CurseForge artifact host/origin acceptance for `forgecdn.net` and approved subdomains and reject unrelated/private/unapproved hosts.
- [x] Bound Modrinth response bodies before JSON parsing.
- [x] Bound CurseForge response bodies before JSON parsing.
- [x] Bound relevant headers, JSON depth, string length, array/object cardinality, URLs, filenames and IDs.
- [x] Keep canonical provider reacquisition semantics aligned with runtime behavior.
- [x] Require SHA-1/SHA-256/SHA-512 proof for automatic `ProviderDownload`.
- [x] Keep MD5-only CurseForge files `ManualRequired`.
- [x] Preserve server-owned staging and portable filename protections.

## Milestones and validation history

### Milestone 1

Production SHA: `640e35783549f12de3997606a8a396b077eaaca1`

Validation run: `33582718560` — SUCCESS

Implemented and proved:

- backend-owned opaque provider staging sessions;
- frontend no longer chooses arbitrary native download destinations;
- portable traversal/prefix/separator/device-name rejection;
- staging symlink-root rejection in production logic;
- server-side Modrinth/CurseForge destination construction;
- strong-hash requirement;
- MD5-only CurseForge provenance canonicalizes to `ManualRequired`;
- provider contract and launcher tests.

### Provider HTTP / metadata completion already present at closure start

Production SHA after run: `a6267844d04499debcca5290d46f49ef4357cc5e`

Validation run: `33614873216` — SUCCESS

This later milestone, which the stale ledger had not reconciled, already implemented:

- separate authenticated CurseForge API and artifact clients;
- exact-origin API redirect policy;
- explicit artifact-host policy;
- in-process provider HTTP without credential-bearing `curl`/child argv;
- bounded provider response bodies and metadata shapes;
- header/string/cardinality limits;
- Modrinth and Desktop provider compilation/tests and strict clippy.

The successful run included `swarm-cli` check, provider-runtime tests, Modrinth deterministic tests, strict CLI clippy, Desktop check/tests, strict Desktop clippy, and static secret/subprocess/implicit-redirect proofs.

### Permanent redirect / reacquisition regressions

Permanent-test milestone: `8e004c269b5ff88f96df4fac6b3aaf9a76a63c21`

Validation run: `34068137515` — SUCCESS

Added permanent coverage for:

- real local two-origin redirect behavior: the authenticated origin receives the API key and a cross-origin redirect is rejected before the attacker origin receives a request;
- production HTTPS CurseForge API-origin validation rejects attacker origins;
- artifact redirect policy: approved redirect target accepted, unapproved target rejected, and artifact requests carry no API key;
- CurseForge oversized body rejection before JSON parsing;
- CurseForge header/array/object cardinality limits and existing string-size limits;
- Modrinth oversized response rejection before JSON parsing;
- API-key absence from provider child-process paths and artifact-request paths;
- MD5-only `ProviderDownload` rejection with valid `ManualRequired` fallback;
- strong-hash canonical provider provenance surviving runtime-compatibility round trip for a clean peer.

### Permanent filesystem symlink regression

Final production/permanent-test SHA: `51cd18cb5c937838ae7b4bcc2cb67f6027b1d6d6`

Validation run: `34070434020` — SUCCESS

Added a real Unix filesystem regression which creates an opaque provider staging session, replaces the private session directory with a symlink, and proves `resolve_provider_staging_session` fails closed with `Provider staging session is not a private directory`.

An earlier attempt, run `34068478902`, failed only because the test expected a different error string; production already rejected the symlink correctly. The expectation was corrected and the permanent regression passed without changing production behavior.

## Permanent regression coverage

- [x] two-origin credential redirect rejection and attacker non-receipt
- [x] API credential present only on authenticated API request
- [x] API credential absent from artifact requests
- [x] API-key child-process argv absence regression
- [x] approved artifact redirect accepted
- [x] unapproved artifact redirect rejected before target receives a request
- [x] oversized Modrinth metadata rejected before JSON parsing
- [x] oversized CurseForge metadata rejected before JSON parsing
- [x] metadata string/cardinality/header bounds
- [x] Unix traversal forms
- [x] Windows drive/backslash/UNC traversal forms
- [x] reserved device/trailing-dot/trailing-space filename forms
- [x] opaque staging-session fencing
- [x] real symlink staging-session rejection
- [x] MD5-only provider file remains manual-required
- [x] strong-hash canonical provider clean-peer round trip

## Final exact-head acceptance

Validation SHA: `191df6eaa69a6b38e2202e6a2e3ef061417b345d`

Run: `34070673328` — SUCCESS

The validation SHA differs from final production SHA `51cd18cb...` only by the temporary read-only acceptance workflow. GitHub compare showed exactly one changed file between those SHAs: `.github/workflows/agent5-exact-head-acceptance.yml`.

The acceptance workflow had `contents: read`, did not rewrite source, did not commit, and verified the exact remote SHA plus a clean tree at both start and end.

Passed acceptance commands/checks:

- [x] `cargo fmt --all -- --check`
- [x] Desktop `cargo fmt ... -- --check`
- [x] `cargo check --workspace --locked`
- [x] `cargo check -p swarm-cli --locked`
- [x] `cargo clippy -p swarm-cli --all-targets --locked -- -D warnings`
- [x] `cargo test -p swarm-cli --lib --locked provider_runtime`
- [x] `cargo test -p swarm-cli --test modrinth_provider --locked`
- [x] `cargo test -p swarm-cli --test agent5_supply_chain_acceptance --locked`
- [x] `cargo test -p swarm-protocol --locked agent5_supply_chain_tests`
- [x] Desktop locked check
- [x] Desktop strict all-target clippy with `-D warnings`
- [x] traversal matrix regression
- [x] opaque staging-session regression
- [x] symlinked staging-session regression
- [x] two-origin authenticated redirect regression
- [x] artifact redirect allow/reject regression
- [x] API-key child-process/artifact-path regression
- [x] CurseForge metadata body/header/cardinality regressions
- [x] full Desktop tests
- [x] launcher-controller provider contract tests
- [x] exact SHA + clean tree at start
- [x] exact SHA + clean tree at end

The current repository does not expose provider-runtime coverage as an integration-test target named `--test provider_runtime`; those tests live in the `swarm-cli` library module and were run explicitly with `cargo test -p swarm-cli --lib --locked provider_runtime`.

## Post-validation cleanup contract

After exact-head run `34070673328` turned green, no provider/Desktop production file and no permanent test file changed.

Post-validation changes are limited to:

1. deletion of temporary `.github/workflows/agent5-exact-head-acceptance.yml` at cleanup SHA `61e44078f32912fdc899aa7cba07036114a80df1`;
2. this Agent 5 ledger reconciliation / handoff update.

No validation-only Tauri sidecar binary is committed. Earlier temporary Agent 5 helper workflows/materializers self-cleaned from their successful production milestones.

## Handoff

READY FOR INTEGRATION: YES

Exact production/permanent-test SHA: `51cd18cb5c937838ae7b4bcc2cb67f6027b1d6d6`

Exact validated helper SHA: `191df6eaa69a6b38e2202e6a2e3ef061417b345d`

Exact validation run: `34070673328` — SUCCESS

Post-validation cleanup head before this ledger-only handoff commit: `61e44078f32912fdc899aa7cba07036114a80df1`

Final branch head: this ledger-only handoff commit on `fix/agent-5-supply-chain`; its exact SHA must be taken from the branch ref after this commit because a Git commit cannot contain its own SHA in its contents.

Known integration conflict areas: `apps/desktop/src/launcher-controller.js`, Tauri provider commands, `crates/swarm-cli/src/provider_runtime.rs`, shared Modrinth provider code, canonical provider semantics. Integration must preserve opaque staging sessions, strong-hash/manual semantics, HTTP client separation, redirect policies, and metadata limits.

Do not merge Agent 5 from this branch automatically; hand it to the integration owner.

## Agent final statement

READY FOR INTEGRATION
