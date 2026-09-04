import os
from pathlib import Path


path = Path("implementation/agent-4-network.md")
text = path.read_text()
validated_sha = os.environ["VALIDATED_PRODUCTION_SHA"]
run_id = os.environ["VALIDATION_RUN_ID"]

replacements = [
    ("STATUS: BLOCKED", "STATUS: READY FOR INTEGRATION"),
    ("READY FOR INTEGRATION: NO", "READY FOR INTEGRATION: YES"),
    (
        "INTEGRATED SHA: pending — Agent 4 is not ready to merge because FINAL-028 remains unresolved.",
        "INTEGRATED SHA: pending — Agent 4 is validated and ready for integration; this branch was not merged by Agent 4.",
    ),
    (
        "- [ ] Anchor discovery announcements to a verifiable canonical world authority/current-head proof rather than merely the announcer's self-signature. **BLOCKED on missing canonical non-omittable freshness/current-head proof.**",
        "- [x] Anchor discovery announcements to a verifier-nonce freshness proof bound to the canonical membership, authority/fence, WorldConfig, and Agent 3 canonical head, certified by the current Agent 1 quorum (joint old+new quorum while pending).",
    ),
    (
        "- [ ] Add malicious discovery provider / stale-authority / malformed-proof / wrong-history / replay-after-transition acceptance regressions. These cannot truthfully pass until the canonical proof primitive exists.",
        "- [x] Add malicious discovery provider / stale-authority / malformed-proof / wrong-history / replay-after-transition acceptance regressions for public browse and exact resolve.",
    ),
    (
        "- [ ] discovery unauthorized-signer/current-authority proof regressions — blocked on the canonical current-head proof primitive described above",
        "- [x] discovery unauthorized-signer/current-authority proof regressions, including live browse/resolve malicious-provider ordering, malformed freshness responses, durable recovery-promise fencing, and joint quorum",
    ),
    ("## Agent final statement\n\nBLOCKED", "## Agent final statement\n\nREADY FOR INTEGRATION"),
]
for old, new in replacements:
    if old in text:
        text = text.replace(old, new, 1)

post_validation_files = [
    ".github/agent4_final028_patch.py",
    ".github/agent4_final028_repair.py",
    ".github/agent4_finalize_ledger.py",
    ".github/agent4_finalize_network_test.py",
    ".github/agent4_finalize_source.py",
    ".github/agent4_finalize_source_v2.py",
    ".github/agent4_finalize_source_v3.py",
    ".github/agent4_finalize_source_v4.py",
    ".github/clippy-failure.txt",
    ".github/workflows/agent4-final028.yml",
    "implementation/agent-4-network.md",
]

closure = f'''\n\n## FINAL-028 exact-head closure (2026-09-04)\n\nThis section supersedes the earlier historical FINAL-028 blocker analysis above.\n\n### Pre-closure failure evidence\n\n- `33870722585` — FAILURE on `4de788abdce185fc79417140f54a0e1121e2b524`. Pristine bootstrap, one-shot materialization, formatting, allowlist, and workspace all-target check passed. Strict clippy stopped only on the 10-argument permanent network-test helper; no production compiler defect was present. The helper was restructured as `AnnouncementFixture` without dropping any security-bound input.\n- `33876484031` — FAILURE on `c178c4a2513a6db6ebd74bef0b6c25c91a0de117`. PASS: pristine bootstrap assertion, source materialization, changed-file allowlist, `cargo fmt --all -- --check`, `cargo check --workspace --all-targets --locked`, strict clippy with `-D warnings`, protocol freshness, core freshness, Agent 1 joint membership quorum tests, permanent FINAL-028 verifier suite `6/6`, and `durable_recovery_promise_fences_stale_freshness_and_current_majority_recovers`. FAIL: `discovery_network_freshness`, `malicious_and_stale_providers_cannot_win_browse_or_exact_resolve`, approximately line 397, with `discovery response channel closed` after about 12.6 seconds. No production milestone was committed because the workflow correctly gates that commit on focused network success.\n- Root cause of the channel-close failure: `DiscoveryNode::next_event()` propagated closed request-response channels from authentication acknowledgement sends (`HelloChallengeAccepted` / `HelloAccepted`) with `?`. A peer connection/request replacement could therefore turn a peer-local handshake race into a fatal error for the whole browse/resolve caller. The repair keeps authentication verification unchanged but treats a closed acknowledgement channel as peer/request-local and continues the discovery operation.\n- The live regression was also changed from wall-clock convergence sleeps to observed Kademlia provider-set readiness and now surfaces spawned peer task termination through inspected `JoinHandle` results.\n\n### Accepted production candidate\n\n- Integration ancestor: `c9252820a560e6ed4d30bb77227e3a494c6ce869`.\n- Final production milestone: `{validated_sha}`.\n- Exact validated production SHA: `{validated_sha}`.\n- Exact-head validation run: `{run_id}` — SUCCESS.\n- Linux exact-head gate: PASS.\n- Windows freshness portability: PASS.\n- macOS freshness portability: PASS.\n- Public browse malicious/stale-first provider regression: PASS; stale/malicious candidate rejected and current fresh-quorum candidate accepted.\n- Exact resolve malicious/stale-first provider regression: PASS; resolver did not use first-self-valid semantics and current fresh-quorum candidate accepted.\n- Malformed freshness response handling: PASS.\n- Discovery service resilience: PASS; peer-local response-channel closure/provider failure does not terminate whole browse/resolve availability.\n- Durable three-peer stale-authority/recovery-promise transition: PASS; newer durable promise fenced stale freshness signing, stale side could not form accepted quorum, current side recovered with legitimate quorum.\n- Joint old+new quorum: PASS for both majorities; insufficient old, insufficient new, and stale-old-only cases rejected.\n- Recovery-promise fencing: PASS.\n- Agent 1 3-peer / 5-peer partition safety, Solo-loss safety, live membership replication, automatic invite join, three-daemon recovery, and recovery-successor crash/resume: PASS.\n- Agent 2 direct-history/current-authority regressions: PASS.\n- Agent 3 canonical-head integrity, missing-head rollback, stale fenced commit, and cross-process promise non-equivocation regressions: PASS.\n- Impaired QUIC lost-ACK/restart recovery: PASS.\n- All workspace and CLI integration targets required by the closure workflow compiled/passed.\n- Exact SHA equality and clean-tree assertions passed at both start and end of the Linux exact-head job.\n\n### FINAL-028 invariants preserved\n\nFreshness remains bound to verifier identity + nonce, world ID, exact announcement hash, membership sequence/hash, pending joint membership identity, current authority, authority epoch, fencing token/generation, WorldConfig sequence/hash, Agent 3 canonical snapshot number + manifest/head hash + head epoch + head sequence. Signers remain bounded, unique, canonical, and keyed to canonical member public keys. Steady state uses current Agent 1 majority; pending membership requires both old and new majorities. A voter with a newer durable recovery promise/state refuses stale authority freshness signing. Checked exhaustion/bounds remain fail-closed.\n\n### Post-validation diff\n\nOnly temporary Agent 4 remediation machinery and this ledger are changed after `{validated_sha}`. Cleanup paths:\n'''
closure += "\n".join(f"- `{p}`" for p in post_validation_files)
closure += f'''\n\nNo Rust, permanent test, Cargo metadata, or production workflow behavior changed after the accepted production SHA.\n\nSTATUS: READY FOR INTEGRATION\n\nREADY FOR INTEGRATION: YES\n\nExact SHA to integrate: `{validated_sha}`\n\nREADY FOR INTEGRATION\n'''

if "## FINAL-028 exact-head closure (2026-09-04)" in text:
    raise SystemExit("FINAL-028 exact-head closure already recorded")
path.write_text(text.rstrip() + closure)
