# Agent 5 — Package / Provider Security

## Status

STATUS: IN PROGRESS

BRANCH: `fix/agent-5-supply-chain`

STARTING SHA: `b4bab08562cf0eb53763674407375b023e1d0858`

CURRENT HEAD SHA: `a9736b159d9e9618a3ed8515c20e93f92c1453cb` (branch creation head; campaign ledger commit only on top of production baseline)

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

- [ ] Stop accepting arbitrary provider download destination paths from the frontend.
- [ ] Make backend construct provider staging paths from server-owned root plus opaque provider identity/session.
- [ ] Validate provider filename as exactly one safe normal path component.
- [ ] Reject absolute paths, prefixes, separators, `.`/`..`, UNC/drive forms and cross-platform separator tricks.
- [ ] Verify normalized containment under provider staging root before publication.
- [ ] Separate authenticated CurseForge API client from artifact download client.
- [ ] Disable cross-origin redirects for authenticated API requests or allow only exact approved origin.
- [ ] Ensure `SWARMCRAFT_CURSEFORGE_API_KEY` is not passed in child argv and cannot reach another origin.
- [ ] Make canonical `ProviderDownload` rules consistent with runtime reacquisition for MD5-only CurseForge files.
- [ ] Choose and enforce one coherent strong-hash/runtime-hash/manual-required policy.
- [ ] Bound provider API response bytes before JSON parsing for Modrinth and CurseForge.
- [ ] Bound relevant header/cardinality/string metadata.
- [ ] Define redirect host allowlists for provider API and artifact downloads.
- [ ] Add traversal fixture matrix across Unix/Windows path forms.
- [ ] Add two-origin HTTPS redirect credential-leak tests.
- [ ] Add oversized metadata response tests.
- [ ] Add MD5-only canonicalization/reacquisition contract test.

## Work completed

- Campaign start verified from `integration/audit-remediation-v1` plan commit `a9736b159d9e9618a3ed8515c20e93f92c1453cb`, whose parent is the required production baseline `b4bab08562cf0eb53763674407375b023e1d0858`.
- Assigned branch `fix/agent-5-supply-chain` created from the campaign plan commit. No production dependency gate applies.
- Read the full Agent 5 ledger plus the required final audit, Auditor 6 package/supply-chain report, and Auditor 7 security report. The audit files live on their audit branches and are not copied into the campaign branch.

## Tests run

| Test | Result | Commit/SHA | Notes |
|---|---|---|---|
| Git/audit start-state verification | PASS | `a9736b159d9e9618a3ed8515c20e93f92c1453cb` | Branch absent before start; created from campaign plan commit whose production parent is required baseline. |

## Required validation before handoff

- [ ] format
- [ ] clippy/lint for provider/Desktop Rust
- [ ] provider unit tests
- [ ] Desktop provider tests
- [ ] traversal matrix
- [ ] cross-origin secret redirect fixture
- [ ] metadata size-bound tests
- [ ] MD5/manual/provider-download consistency tests
- [ ] exact-head CI/dedicated provider validation

## Blockers

- Local terminal connector is currently unavailable due a conversation-identity guard. Repository reads/writes are being performed through the GitHub connector; exact-head CI will be used for executable validation unless local execution becomes available.

## Handoff

READY FOR INTEGRATION: NO

Exact final head: pending

Known conflict areas: `launcher-controller.js`, Tauri CurseForge commands, runtime provider reacquisition.

## Agent final statement

NOT COMPLETE
