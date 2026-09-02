# Agent 5 — Package / Provider Security

## Status

STATUS: IN PROGRESS

BRANCH: `fix/agent-5-supply-chain`

STARTING SHA: `b4bab08562cf0eb53763674407375b023e1d0858`

CURRENT IMPLEMENTATION SHA: `640e35783549f12de3997606a8a396b077eaaca1` (Milestone 1 production head; this ledger-only progress commit advances the branch ref without changing production behavior)

INTEGRATED SHA: pending

## Mission

Make provider-controlled input incapable of escaping filesystem staging, leaking credentials, bypassing host trust boundaries, exhausting metadata handling, or promising reproducibility the runtime cannot actually deliver.

## Findings owned

- FINAL-003 — CurseForge provider filename traversal / staging escape
- FINAL-017 — CurseForge API credential redirect/handling weakness
- FINAL-018 — MD5-only/provider-download reproducibility mismatch
- FINAL-019 — unbounded provider metadata responses
- FINAL-034 — provider redirect/host trust boundary hardening

Read `audits/FINAL-AUDIT.md`, Auditor 6 Package/Supply Chain, and Auditor 7 Security before editing.

## Dependencies

Required before starting: none.

## Ownership boundaries

Primary ownership:

- `crates/swarm-provider`
- Modrinth provider implementation
- Desktop CurseForge provider/Tauri commands
- `crates/swarm-cli/src/provider_runtime.rs`
- launcher provider staging commands/UI contract as required for safe native boundary

Coordinate with Agent 7 when changing Tauri/provider frontend payloads.

## Implementation checklist

- [x] Stop accepting arbitrary provider download destination paths from the frontend.
- [x] Make backend construct provider staging paths from server-owned root plus opaque provider identity/session.
- [x] Validate provider filename as exactly one safe normal path component.
- [x] Reject absolute paths, prefixes, separators, `.`/`..`, UNC/drive forms and cross-platform separator tricks.
- [x] Verify containment under provider staging root by deriving every publication path exclusively from a validated opaque session plus validated provider identity/file components; staging sessions reject symlink roots.
- [ ] Separate authenticated CurseForge API client from artifact download client.
- [ ] Disable cross-origin redirects for authenticated API requests or allow only exact approved origin.
- [ ] Ensure `SWARMCRAFT_CURSEFORGE_API_KEY` is not passed in child argv and cannot reach another origin.
- [x] Make canonical `ProviderDownload` rules consistent with runtime reacquisition for MD5-only CurseForge files.
- [x] Enforce the strong-hash/manual-required policy: automatic `ProviderDownload` requires SHA-1/SHA-256/SHA-512; MD5-only provenance is `ManualRequired`.
- [ ] Bound provider API response bytes before JSON parsing for Modrinth and CurseForge.
- [ ] Bound relevant header/cardinality/string metadata.
- [ ] Define redirect host allowlists for provider API and artifact downloads.
- [x] Add traversal fixture matrix across Unix/Windows path forms.
- [ ] Add two-origin HTTPS redirect credential-leak tests/policy regression coverage.
- [ ] Add oversized metadata response tests.
- [x] Add MD5-only canonicalization/reacquisition contract test.

## Work completed

- Campaign start verified from `integration/audit-remediation-v1` plan commit `a9736b159d9e9618a3ed8515c20e93f92c1453cb`, whose parent is the required production baseline `b4bab08562cf0eb53763674407375b023e1d0858`.
- Assigned branch `fix/agent-5-supply-chain` created from the campaign plan commit. No production dependency gate applies.
- Read the full Agent 5 ledger plus the required final audit, Auditor 6 package/supply-chain report, and Auditor 7 security report. The audit files live on their audit branches and are not copied into the campaign branch.
- Milestone 1 committed at `640e35783549f12de3997606a8a396b077eaaca1`:
  - `provider_staging_dir` now returns an opaque session token rather than exposing a filesystem path to the webview.
  - Desktop Modrinth and CurseForge commands derive destinations server-side from the validated staging session and provider identities.
  - Modrinth path components and CurseForge JAR filenames reject portable traversal/prefix/separator/device-name forms, including Windows drive/UNC/backslash cases on non-Windows test hosts.
  - staging-session resolution rejects malformed tokens and symlink session roots.
  - launcher frontend sends opaque staging sessions, never native destination paths.
  - canonical provider provenance now rejects `ProviderDownload` without SHA-1/SHA-256/SHA-512 proof; MD5-only CurseForge artifacts are recorded as `manual_required`.
  - Desktop canonicalization independently enforces the same strong-hash rule.
- Remaining implementation is limited to authenticated CurseForge HTTP isolation, redirect trust boundaries, metadata resource limits, their regression tests, and final lint/exact-head validation.

## Tests run

| Test | Result | Commit/SHA | Notes |
|---|---|---|---|
| Git/audit start-state verification | PASS | `a9736b159d9e9618a3ed8515c20e93f92c1453cb` | Branch created from campaign plan commit whose production parent is required baseline. |
| Agent 5 Milestone 1 workflow | PASS | run `33582718560`, source commit `640e35783549f12de3997606a8a396b077eaaca1` | Protocol tests green; validation-only Tauri sidecars staged; Desktop check/tests green; launcher-controller Node tests green; workflow committed source and self-cleaned helpers. |
| `cargo test -p swarm-protocol --locked` | PASS | run `33582718560` | Includes `md5_only_provider_download_is_not_a_valid_reacquisition_contract`. |
| Desktop `cargo check` + `cargo test --locked` | PASS | run `33582718560` | Includes hostile provider filename traversal matrix, staging-session fencing, Modrinth identity path-component test, and existing provider tests. |
| `node --test apps/desktop/tests/launcher-controller.test.mjs` | PASS | run `33582718560` | Includes MD5-only CurseForge mapping to `manual_required` and provider canonical mapping tests. |

## Required validation before handoff

- [x] format for Milestone 1 touched Rust
- [ ] clippy/lint for provider/Desktop Rust
- [ ] complete Modrinth deterministic provider suite after HTTP/metadata hardening
- [x] Desktop provider tests for Milestone 1
- [x] traversal matrix
- [ ] cross-origin secret redirect policy fixture
- [ ] metadata size-bound tests
- [x] MD5/manual/provider-download consistency tests
- [ ] canonical provider provenance round-trip / clean-peer reacquisition contract validation
- [ ] exact-head dedicated provider validation

## Blockers

- No product blocker. Local terminal connector remains unavailable due a conversation-identity guard, so executable validation is being performed on GitHub Actions runners. This does not currently prevent implementation or test execution.

## Handoff

READY FOR INTEGRATION: NO

Exact final head: pending

Known conflict areas: `apps/desktop/src/launcher-controller.js`, Tauri provider commands, `crates/swarm-cli/src/provider_runtime.rs`, shared Modrinth provider code. Agent 7/integration must preserve the opaque staging-session payload contract.

## Agent final statement

NOT COMPLETE
