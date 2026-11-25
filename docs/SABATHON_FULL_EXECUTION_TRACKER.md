# Sabathon full execution tracker

Baseline: `2d45f97b5e31118a7074361e141a90b9296eccbd` (2026-11-25).

All 68 original backlog IDs retained. Acceptance text is copied from the two roadmap documents. Historical partial work is not counted as complete. NOT STARTED means not yet started in this execution ledger, not necessarily absent from source. IN PROGRESS includes existing partial work. LOCALLY VERIFIED here is scoped to implemented automated components, not full acceptance. CI VERIFIED requires the current source and complete item-specific evidence; FIELD VERIFIED requires field evidence. No item is marked fully accepted at initialization.

Current CI: [45524776685](https://github.com/MousaXD/swarmcraft/actions/runs/45524776685) completed successfully on `e1f7bce` (certified successor offers). Earlier `bfca9f5` run 45523386525 was cancelled; `d6bf294` run 45521764635 completed successfully with 20 selected jobs passed. Newer checkpoints below have local evidence and still require exact-SHA CI. See [live integration status](SABATHON_P0_LIVE_INTEGRATION_STATUS.md) for scope and residuals.

## Status accounting

Implementation completion: count only items whose full implementation criteria are satisfied, excluding partial work. Verified acceptance: count only items meeting all applicable acceptance criteria. Mixed #26 counted once overall and split by phase. Initial assessment is conservative and requires item-specific source review; do not infer completion from a packaging job.

## Execution ledger

### #1 — Sabathon P0 · Partial
- Work: Make one private flow connect Create/Import, Invite, Join, Verify save, and Play in the shipping UI. Reuse the Rust membership and installer paths
- Acceptance: Two fresh devices join one world by invite, without terminal steps or manual world-folder copying
- State: **IN PROGRESS**
- Source: apps/desktop/
- Evidence: Live integration status; scoped automated evidence, exact-SHA CI pending.
- Remaining / next action: Run complete installed invite-to-play journey
- External dependency: Not yet established; assess during execution.
- Commit: 2d45f97 baseline; see live status for individual commits

### #2 — Sabathon P0 · Missing
- Work: Define and implement an authenticated route offer and freshness verifier for the currently authorized Minecraft host
- Acceptance: Resolver proves exact world/generation, fresh authority, membership, and runtime readiness before admitting a route. Expired, retired, conflicting, and wrong-world routes are rejected
- State: **LOCALLY VERIFIED**
- Source: crates/swarm-cli/src/live_authority.rs
- Evidence: Live integration status; scoped automated evidence, exact-SHA CI pending.
- Remaining / next action: Confirm exact-SHA CI; audit unknown-successor discovery
- External dependency: Not yet established; assess during execution.
- Commit: 2d45f97 baseline; see live status for individual commits

### #3 — Sabathon P0 · Missing
- Work: Ship a local loopback TCP gateway (`127.0.0.1:<stable-port>`) and verified **Join current host / Reconnect** action. Separate server ready from game route verified
- Acceptance: Players add one permanent loopback address in Minecraft. On host change or fencing, the gateway closes the old stream immediately; after verified handoff, manual reconnect to the same loopback address lands only on the verified successor and sees the saved marker. Errors distinguish route, transport reachability, permission, and compatibility
- State: **IN PROGRESS**
- Source: crates/swarm-network/
- Evidence: Live integration status; scoped automated evidence, exact-SHA CI pending.
- Remaining / next action: Run installed reconnect with saved marker and retired-host refusal
- External dependency: Not yet established; assess during execution.
- Commit: 2d45f97 baseline; see live status for individual commits

### #4 — Sabathon P0 · Partial
- Work: Automate the signed staged transfer exchange behind the existing wizard by letting `daemon.rs` advance `WireRequest::AuthorityTransfer` reactively instead of manual hex copy-paste (`transfer_commands.rs`)
- Acceptance: Both sides persist transfer ID/phase and resume cleanly after app or `run_cli` restart without duplicate transfer. The successor sees a durable checkpoint. The source releases hosting and returns as a replica
- State: **IN PROGRESS**
- Source: crates/swarm-cli/src/daemon.rs; crates/swarm-storage/src/stale_publication.rs
- Evidence: Live integration status; scoped automated evidence, exact-SHA CI pending.
- Remaining / next action: Exact transfer phases and included recovery/membership votes are consumed only after certified replay publication. Planned-retirement re-entry now holds the existing runtime exclusion and exact committed-transfer guard. Crash-recovery retirement and exact recovery certificate/independently signed sleep and reservation reconciliation are implemented; Targeted certified successor offers now support previously unknown, subsequently admitted authorities on connected authenticated transport; pending membership intents and solo conflicts remain blocking.
- External dependency: Not yet established; assess during execution.
- Commit: 2d45f97 baseline; see live status for individual commits

### #5 — Sabathon P0 · Partial
- Work: Surface Host Readiness, replica freshness, and shutdown outcome in one player flow
- Acceptance: Stale replica, unreachable voter, missing mods, zero budget, and active safe successor produce distinct messages. Green requires fresh backend proof
- State: **IN PROGRESS**
- Source: crates/swarm-cli/src/host_readiness.rs
- Evidence: Live integration status; scoped automated evidence, exact-SHA CI pending.
- Remaining / next action: Exercise installed negative and positive readiness states
- External dependency: Not yet established; assess during execution.
- Commit: 2d45f97 baseline; see live status for individual commits

### #6 — Sabathon P0 · Partial
- Work: Make sleep/wake reachable through ordinary UI and verify the supported quorum path with real Minecraft
- Acceptance: All peers stop safely. Original host stays off. A supported surviving majority wakes the exact saved world with one active authority
- State: **IN PROGRESS**
- Source: crates/swarm-cli/src/migration.rs
- Evidence: Live integration status; scoped automated evidence, exact-SHA CI pending.
- Remaining / next action: Exercise real Minecraft sleep/wake after original host leaves
- External dependency: Not yet established; assess during execution.
- Commit: 2d45f97 baseline; see live status for individual commits

### #7 — P1 · Missing
- Work: Add an optional first-party Fabric **client** auto-reconnect mod on top of the P0 loopback gateway
- Acceptance: On disconnect, the gateway closes the old game stream; with explicit opt-in, the client mod starts a fresh Minecraft connection to the same loopback entry only after a new trusted route is verified. Retry is bounded and cancelable
- State: **NOT STARTED**
- Source: Pending item-specific source mapping; see original roadmap links.
- Evidence: Historical roadmap baseline only; current item-specific evidence pending.
- Remaining / next action: Inspect current source against acceptance; implement missing dependencies and test.
- External dependency: Not yet established; assess during execution.
- Commit: Pending item-specific attribution

### #8 — P1 · Proposal (after #9)
- Work: Design an explicit two-device **Pass hosting** policy with one writable holder, explicitly specifying online-overlap vs. async-dropbox handoff requirements
- Acceptance: Both devices can transfer the right to play without a third voter when they overlap online (or via an explicit async dropbox adapter if one is offline). Partition tests prove there is never a second writer
- State: **IN PROGRESS**
- Source: docs/rfcs/two-device-exclusive-turn.md; crates/swarm-consensus/src/exclusive_turn_model.rs (test only)
- Evidence: Reviewed-in-source proposal selects online overlap, durable irreversible release, exact checkpoint receipts, no timeout takeover, and new-world recovery for lost grants. Five test-only model regressions and bounded search of 3,165 independent two-journal states passed. Full consensus suite 51 tests and strict Clippy passed.
- Remaining / next action: Independent design/threat-model review; real versioned signed records, atomic journal/outbox, process/route fencing and creation-time opt-in. Keep default quorum and old peers unchanged. This model is not an active runtime policy.
- External dependency: Independent review and two-device actual partition/gameplay acceptance; no safe automatic crash takeover without an independent anchor.
- Commit: See 25 November exclusive-turn design checkpoint.

### #9 — P1 · Proposal (recommended first)
- Work: Provide a supported headless zero-Java third-replica profile (`swarmcraft daemon`) for small private worlds
- Acceptance: A small third device (Raspberry Pi, NAS, or low-cost VPS) runs and restarts without Java, acknowledges current saves, and votes under the existing quorum rules — solving both crash failover and async offline handoff with zero new consensus machinery
- State: **LOCALLY VERIFIED**
- Source: crates/swarm-cli/src/{main,hosting_budget,daemon,live_authority}.rs; docs/HEADLESS_REPLICA.md; deploy/systemd/swarmcraft-replica.service
- Evidence: 138 CLI library tests, 4 public CLI hosting tests, and real three-daemon recovery/wake with a replica-only voter passed. The local process test checks exact signed recovery votes, restart, no runtime, and no permit. Dedicated profile and deployment example implemented.
- Remaining / next action: Field-run a low-memory VPS/Pi/NAS with independent transport, verify snapshot/ack across restart, measure storage/RSS, and validate real Minecraft offline handoff. Systemd example not installed on a live device.
- External dependency: Dedicated third device/network and real gameplay for field acceptance.
- Commit: See 25 November headless replica checkpoint.

### #10 — P1 · Partial
- Work: Make invite expiry, reuse, membership approval, pending state, and revocation legible. Review a recipient-bound or one-use invite option
- Acceptance: An expired or revoked grant cannot admit a new device. A removed device cannot fetch newer private snapshots. Existing downloaded copies have clearly documented retention limits
- State: **LOCALLY VERIFIED**
- Source: crates/swarm-cli/src/{invite,main,daemon,daemon_protocol_tests}.rs; crates/swarm-storage/src/{membership,world}.rs; apps/desktop/src/invite-lifecycle.js; native Desktop inspect/revoke and Svelte parity; crates/swarm-cli/tests/automatic_invite_join.rs; docs/INVITE_LIFECYCLE.md
- Evidence: CLI issuer revocation, authenticated denial and removed-member replication, exact pinned Desktop inspect/revoke, pending-state refresh, safe fresh-token replacement, and recipient-bound/one-use policy review. 51 daemon/CLI tests, 67 storage tests (one optional soak ignored), real authenticated join/leave/replication regression, strict CLI/storage Clippy, native Tauri check, 140 Desktop tests (including real ChromeDriver at three sizes), 82 Svelte unit tests and 29 source-parity tests pass. Prior issuer-revocation exact-SHA CI passed.
- Remaining / next action: Exact-SHA CI and installed Desktop field acceptance. V1 remains reusable; recipient-bound/one-use grants require a separately reviewed versioned protocol. Existing durable prepares are never silently canceled.
- External dependency: Installed Desktop/independent network acceptance; distributed revocation/rollback protection is not claimed.
- Commit: See 25 November invite lifecycle checkpoint.

### #11 — P1 · Partial
- Work: Expose a clear recovery path when a sole valid copy survives but quorum is gone. Reuse export/import and add a labeled recovered-world workflow
- Acceptance: The original bytes are preserved. A new recovered world uses a new identity unless a valid original authority transition exists. UI clearly identifies the old and new worlds
- State: **LOCALLY VERIFIED**
- Source: crates/swarm-cli/src/{world_import,historical_authority,main}.rs; apps/desktop/src/{app,recovery-copy,backend-adapter}.js; native Tauri recover_copy and Svelte parity; docs/RECOVERED_COPY.md
- Evidence: Exact selected hash and complete retained signed history verified offline; independent private genesis, identical file state root, atomic staging, no inherited consent, and unchanged original signed records. Corruption, wrong selection, interrupted publication, and missing confirmation regressions pass; real multi-daemon offline former-replica recovery via CLI passes. 140 CLI library tests, 2 invite/recovery integration tests, strict CLI Clippy, native Tauri check, 137 Desktop tests including ChromeDriver confirmation/cancel/reopen/duplicate-click checks at three sizes, 29 Svelte source tests, and 82 Svelte unit tests pass.
- Remaining / next action: Exact-SHA CI; installed native recovery journey on representative retained world. Unsupported incomplete/pruned/conflicting history stays denied; manual verified export is separate.
- External dependency: Installed native UI field acceptance, including a real Minecraft save.
- Commit: See 25 November recovered-copy checkpoint.

### #12 — Sabathon P0 · Missing
- Work: Build authenticated game-traffic transport as a dedicated libp2p stream protocol (reusing existing QUIC/TCP/Noise/DCUtR/relay) wired directly into the P0 loopback gateway (#3), isolated from `request-response` snapshot sync
- Acceptance: Remote players complete real Minecraft login and play without manual router port-forwarding. Game streams authenticate both ends against world membership/authority, apply bounded queues/backpressure, and never starve or corrupt authority/snapshot validation
- State: **LOCALLY VERIFIED**
- Source: crates/swarm-network/
- Evidence: Live integration status; scoped automated evidence, exact-SHA CI pending.
- Remaining / next action: Confirm current hosted live Minecraft journey and independent network acceptance
- External dependency: Not yet established; assess during execution.
- Commit: 2d45f97 baseline; see live status for individual commits

### #13 — Sabathon P0 · Unverified
- Work: Execute the three-device field run for #85 and #120, including opt-out
- Acceptance: Alice, Bob, and Carol play on independent networks through the loopback gateway. The original host is stopped or isolated with approved controls. The correct successor restores the marker, and the original host cannot resume stale writes
- State: **EXTERNALLY BLOCKED**
- Source: scripts/e2e/three_machine/
- Evidence: Live integration status; scoped automated evidence, exact-SHA CI pending.
- Remaining / next action: Execute runbook on three independent networks when available
- External dependency: Independent physical devices/networks
- Commit: 2d45f97 baseline; see live status for individual commits

### #14 — P1 · Partial
- Work: Finish discovery and connection diagnosis for ordinary players. Show whether directory, peer link, host proof, or Minecraft game stream failed
- Acceptance: Per-path results are fresh and actionable. Failure has retry/copy options and no endless loading or false Ready
- State: **LOCALLY VERIFIED**
- Source: [Connection diagnosis](CONNECTION_DIAGNOSIS.md); crates/swarm-cli/src/{game_route_api,daemon,daemon_game_gateway}.rs; crates/swarm-network/src/game_transport.rs; apps/desktop/src/{game-route-status,discovery-diagnostics}.js; both Desktop views.
- Evidence: strict core Clippy; 115 network, 140 CLI library and 51 daemon tests; real game-stream and resolver integration; local negative gateway smoke (unconsented mock, not installed Minecraft); 144 Desktop tests including ChromeDriver at three viewport sizes; 82 Svelte tests, 29 source parity tests, typecheck/build. Four native Desktop route-decoder tests passed. Exact-SHA CI tracked separately.
- Remaining / next action: Confirm exact-SHA CI; installed multi-network directory/peer/host/game error journey, including real Minecraft login. Typed host-proof failure does not speculate about which required proof failed.
- External dependency: Installed devices and representative networks for field acceptance.
- Commit: Connection-diagnosis checkpoint following ad99fd8; see git history.

### #15 — P1 · Unverified
- Work: Certify representative LAN, WAN, home NAT, CGNAT, IPv6, mobile, blocked UDP, and packet-loss scenarios
- Acceptance: Matrix reports network class, selected control and game routes, success/failure, and reason. Any unsupported path is named accurately
- State: **IN PROGRESS**
- Source: [Network validation](NETWORK_VALIDATION.md); [field template](network-validation/FIELD_RECORD_TEMPLATE.md); crates/swarm-network/tests/{game_stream_transport,game_stream_relay}.rs
- Evidence: Local authenticated control and dedicated game-stream echo pass on IPv4 QUIC, TCP/Noise, IPv6 QUIC, and production TCP relay. Explicit relay loss and replacement pass. Existing CI netem tests measure snapshot resume, not Minecraft latency.
- Remaining / next action: Record actual control/game paths, successful Minecraft login, and failure reasons on all representative physical networks. No field row promoted from loopback.
- External dependency: Independent LAN/WAN, NAT/CGNAT, mobile, routed IPv6, UDP-blocked and measured-loss environments.
- Commit: See private infrastructure checkpoint in git history.

### #16 — P1 · Partial
- Work: Publish deployable discovery bootstrap, private rendezvous, and game relay configurations with documented boundaries
- Acceptance: A new group can bring up its own compatible node, swap relays, and recover from relay outage. Relay and directory services never grant world authority
- State: **LOCALLY VERIFIED**
- Source: crates/swarm-network/src/infrastructure.rs; crates/swarm-cli/src/{infra_main,runtime_main,discovery}.rs; deploy/infrastructure/; deploy/systemd/swarmcraft-{locator,relay}.service; [deployment runbook](PRIVATE_INFRASTRUCTURE.md)
- Evidence: Production allowlisted relay transfers authenticated control/game bytes, refuses unknown transport, drops the old path on outage and reconnects via replacement without changing peer identities. Independent local locator and signed publisher subprocesses resolve and browse only through DHT with mDNS disabled; private world absent, duplicate ownership denied, relay identity stable on restart. DHT-discovered already-authenticated providers now participate in freshness proof; private visibility change invalidates cached announcement and voting. Strict Clippy, 3 infrastructure unit tests, 5 discovery network regressions and process smoke passed.
- Remaining / next action: Run exact-SHA CI; deploy to an independent group VPN/firewall; validate actual daemon restart/replacement with installed Minecraft. Standalone discovery publishers require a reachable direct/VPN address; no cryptographic DHT isolation or automatic Desktop relay swap claimed.
- External dependency: Independent operators/networks and real gameplay field test.
- Commit: See private infrastructure checkpoint in git history.

### #17 — P1 · Proposal
- Work: Define relay connection, bandwidth, duration, auth, and resource policies before shared hosting
- Acceptance: Fair-use and disconnect behavior are measured with multiple worlds. Invites and private route details are absent from public logs
- State: **IN PROGRESS**
- Source: crates/swarm-network/src/infrastructure.rs; crates/swarm-network/tests/game_stream_relay.rs; [resource policy](PRIVATE_INFRASTRUCTURE.md)
- Evidence: Explicit transport allowlist, connection/reservation/circuit/byte/duration limits and retained libp2p rate limiters. Deployment examples bound memory/tasks/fds. Local four-peer test enforces 256 KiB encrypted circuit cap; 124 KiB acknowledged payload with echo before disconnect and auth revocation; independent circuit remains live. Operator services do not log remote peer events, invite or route payloads.
- Remaining / next action: Measure duration exhaustion, sustained throughput and fairness with real concurrent worlds and limited uplinks; validate operator resource policies on target service host. Current caps are not per-world bandwidth guarantees.
- External dependency: Multiple actual Minecraft worlds and representative shared-relay deployment.
- Commit: See private infrastructure checkpoint in git history.

### #18 — P2 · Proposal
- Work: Add adaptive replication throttling so background saves do not ruin game latency
- Acceptance: On a limited uplink, measured game responsiveness stays within a published target while the replica continues to make bounded progress
- State: **LOCALLY VERIFIED**
- Source: crates/swarm-cli/src/replication_pacing.rs; daemon.rs; daemon_game_gateway.rs; crates/swarm-network/src/game_transport.rs; tests/paced_replication_game_latency.rs; [pacing policy and target](REPLICATION_PACING.md)
- Evidence: Three pacer unit tests, 51 daemon tests, live join/replication and authenticated gateway lifecycle passed. Actual 131,084 encoded blob bytes replicated and verified in 4.17s with 41 overlapping synthetic game echo samples; local p95 1.03ms. Added 1 Mbit/s + 12ms loopback CI gate with p95 <=400ms and bounded replica completion <=30s.
- Remaining / next action: Exact-SHA constrained-link CI evidence; representative actual Minecraft and multi-peer WAN workload measurements. No hard network QoS guarantee or field-certified latency claim.
- External dependency: Actual representative Minecraft runtime and physical network.
- Commit: See replication pacing checkpoint in git history.

### #19 — Sabathon P0 · Partial
- Work: Prove per-world player-only default, opt-in persistence, and withdrawal through CLI, native IPC, and live hosting; make `authority.permit` updates atomic (temp + rename or loopback IPC)
- Acceptance: Zero-budget peer can join and replicate with no JVM. Consent stays scoped to one world and persists across restart. Every launch path rejects missing or withdrawn consent, and concurrent permit reads on Windows/POSIX never see a truncated permit file
- State: **LOCALLY VERIFIED**
- Source: crates/swarm-cli/src/{hosting_budget,authority_permit,migration}.rs; tests/hosting_consent_cli.rs; native contribution commands
- Evidence: Four real CLI consent tests pass (player-only default, persistence, invalid changes, restart of replica-only daemon without Java); live mock Fabric withdrawal, safe-stop and transfer gates pass. Existing atomic permit reader/writer and durable heartbeat-floor protections retained.
- Remaining / next action: Exact-SHA Windows/POSIX CI and installed Minecraft consent journey.
- External dependency: Installed clients and actual host runtime.
- Commit: See hosting process ownership checkpoint in git history.

### #20 — Sabathon P0 · Partial
- Work: Clarify withdrawal lifecycle and handle a hung JVM without silently losing the current save
- Acceptance: UI shows **Stopping at checkpoint** until proof of checkpoint/fence/exit, or a truthful error with recovery choices. Stale readiness and active gateway routes are cleared immediately
- State: **LOCALLY VERIFIED**
- Source: crates/swarm-cli/src/{migration,hosting_budget,runtime_process_guard}.rs; tests/migration_core.rs; native hosting contribution DTO and panel; runtime_archive.rs; tests/hosting_consent_cli.rs; [recovery contract](UNCONFIRMED_SHUTDOWN_RECOVERY.md)
- Evidence: Real local mock-Fabric subprocess exercised full 30-second hung barrier (32.66s, passed), stopping/blocked reporting, immediate stale readiness retirement, live process retention, unchanged canonical snapshot, no signed sleep, and no relaunch/reset after subsequent clean exit without acknowledgement and renewed consent. Durable pending-checkpoint marker retained until verified fenced publication. Earlier successful safe-stop, transfer and withdrawal suite passed. Rust library 151/151 regular tests and strict Clippy; frontend check/tests passed. Required CI gate forbids treating insufficient-memory skip as positive evidence.
- Remaining / next action: Explicit Linux/macOS local CLI archival implemented; Linux tests verify kernel no-replace rename, actual process/session-lock denial, interruption after rename and durable receipt, exact save preservation and unchanged signed history. macOS implementation requires exact-SHA CI; Windows archival remains explicitly unsupported and blocked. Installed Minecraft/Fabric withdrawal and physical-machine certification remain.
- External dependency: Adequately provisioned OS runner and installed Minecraft/Fabric.
- Commit: See unconfirmed shutdown checkpoint in git history.

### #21 — P1 · Partial
- Work: Improve process identity and coordinate a safe resource budget across data roots where supported
- Acceptance: Local concurrency/orphan tests cover process identity, restart, and no second host when capacity or ownership is ambiguous
- State: **LOCALLY VERIFIED**
- Source: crates/swarm-cli/src/{machine_hosting,runtime_process_guard,hosting_budget,migration}.rs; [process ownership](HOSTING_PROCESS_OWNERSHIP.md)
- Evidence: 11 ownership regressions pass: account-wide lock independent of data root, separate controller hard kill with real surviving process fixture, second-root denial, PID birth/boot/namespace checks, pending/corrupt/symlink denial and verified exit cleanup. Durable atomic Linux process identity; Windows process-handle liveness probe. Host-process, 10 migration, four consent CLI and strict Clippy gates passed.
- Remaining / next action: Exact-SHA CI; Windows/macOS retain conservative per-data-root coordination and PID-based records. Cross-account and isolated-container coordination not claimed.
- External dependency: Representative OS installations and local persistent Linux account home.
- Commit: See hosting process ownership checkpoint in git history.

### #22 — P1 · Implemented
- Work: Add whole-process limits only on platforms that can enforce them. Evaluate delegated Linux cgroups, Windows Job Objects, and macOS capability separately
- Acceptance: Measurement includes native memory and children. Unsupported enforcement is labeled heap-only. Existing Java heap and memory headroom rules stay in force
- State: **LOCALLY VERIFIED**
- Source: crates/swarm-cli/src/{host_memory_cap,hosting_budget,runtime_process_guard,migration}.rs; native hosting contribution DTO and panel; [enforcement contract](HOST_MEMORY_ENFORCEMENT.md); scripts/e2e/linux_cgroup_memory_gate.sh
- Evidence: Real delegated Linux cgroup test measured parent and descendant together (~33 MiB), verified the configured high threshold, and enforced a separate 64 MiB kernel hard-limit/group-OOM experiment; live residual descendant blocks runtime reset and account-wide readmission after the direct child exits. Actual host launcher fixture verified 1536 MiB kernel cap for 512 MiB heap before exec, and restored its saved marker. Core library 151/151 regular tests, host/consent/migration gates, strict Clippy, native DTO 11/11, frontend 82/82 and Svelte check pass.
- Remaining / next action: Exact-SHA CI (new dedicated transient systemd kernel/launcher gates), representative installed Minecraft/Fabric memory workload, Windows Job Object design and macOS platform evaluation if stronger enforcement is required. Windows/macOS and unconfigured Linux are explicitly heap-only; Linux cgroup charging is not a whole-process RSS guarantee.
- External dependency: Linux delegation and adequate physical memory; actual installed runtimes/OS devices for field certification.
- Commit: See managed JVM memory enforcement checkpoint in git history.

### #23 — P1 · Partial
- Work: Reconcile V2 transcript, signed consent, voter proof, and admission work from #122/#125. Plan durable migration from V1 separately
- Acceptance: Exact source history, signed approvals, stable and joint voter majorities, replay bounds, no legacy stale writes, interrupted migration, and rollback behavior are reviewed and tested
- State: **LOCALLY VERIFIED** (inert cryptographic and read-only preflight components; not activated)
- Source: PR #122 `56b50274e7e173cd6573e678df6cd0a41963b2c0`; PR #125 `c722ece5b86a41433644013bf1dabccd23142a13`; [reconciliation and cutover plan](rfcs/v2-migration-reconciliation.md).
- Evidence: One bounded protocol verifier plus independently pinned real-Ed25519 consensus adapter, corrected V1 sequence 0 and historical config signer/current epoch authority separation; protocol contracts 12/12, crypto stable/joint and wrong-source tests 7/7, read-only actual canonical-storage review rejects unsafe source, preserves signed history on reopen and refuses V1 metadata at distinct V2 identity. Network transcript tests 10/10, V2 challenge/action tests 15/15 (including overlapping transcript tests), full network library 133/133 before extra invalidation regression, CLI library 155 passed/2 ignored; strict Clippy. Three interruption/rollback specification tests pass (model only).
- Remaining / next action: Exact-SHA CI; durable freeze voting, atomic accepted-root publication, actual crash injection and V2-only live negotiation and authority transitions are separate implementation gates. The reviewed proposal, verified vote set, and authenticated session cannot activate V2. #24 can improve V1 role presentation without enabling V2 permissions.
- External dependency: Independent security review and installed mixed-version/real-device acceptance before activation. Old V1 binaries may still operate on their own separate V1 identity.
- Commit: See V2 migration reconciliation checkpoint in git history.

### #24 — P1 · Partial
- Work: Split member, player, replica, voter, administrator, and host-candidate permissions in UI and policy
- Acceptance: A non-hosting replica can safely vote. A player or viewer can’t become host from UI preference alone. Membership changes preserve durable quorum rules
- State: **LOCALLY VERIFIED** (V1 read-only role projection; independent policy roles remain pending)
- Source: `crates/swarm-cli/src/member_roles.rs`, `world status`, existing signed membership/consent checks; both desktop frontends; [V1 role boundaries](V1_ROLE_PERMISSIONS.md).
- Evidence: Signed real-storage test verifies non-hosting member can sign and persist a joint vote, conflicting durable promise rejected after reopen, local positive consent does not promote role, unsigned descriptor mismatch rejected, and proposed/nonmember cannot inherit membership. Public CLI 5/5; core library 14/14; Svelte 82/82, source parity 30/30, type check and strict Clippy pass.
- Remaining / next action: Exact-SHA CI; installed GUI acceptance. Independent viewer/player/administrator roles need reviewed canonical policy semantics and safe activation, not a V1 UI grant.
- External dependency: Durable V2 migration and canonical policy review before independent grants are enabled.
- Commit: See V1 role projection checkpoint in git history.

### #25 — P1 · Proposal
- Work: Add supported invite/device lifecycle, including contact removal, replacement devices, and recovery of a lost identity
- Acceptance: Losing a PC has a documented repair path. A removed device cannot regain access through stale presence or an obsolete invite. Private recovery material is only exported with explicit user action
- State: **LOCALLY VERIFIED** (explicit V1 lost-device retirement and local contact CLI; full identity backup/restore and Desktop flows remain pending)
- Source: `crates/swarm-cli/src/device_retirement.rs`, `world retire-device`, `contact add/list/remove`, signed membership promise/certificate storage, [device recovery](DEVICE_RECOVERY.md), [invite lifecycle](INVITE_LIFECYCLE.md), [recovered copy](RECOVERED_COPY.md).
- Evidence: Actual local daemon regression proves staged retirement requires surviving old/new quorum, persists across restart, rejects uncertified membership publication and conflicting epoch/sleep consumption, preserves checkpoint/epoch, denies an authenticated old identity's obsolete signed invite and private status request, and admits a new device without erasing the banned identity. Public CLI contact removal leaves signed membership unchanged; concurrent contact mutations preserve other entries, removed contacts cannot be probed, providers check live contact store before signing presence. Protocol/consensus/storage suites, strict Clippy, CLI library 158 passed/2 ignored, CLI binary 51/51, all four existing invite/join integration tests and the updated retirement/contact scenario pass locally.
- Remaining / next action: Desktop operator/contact flow; independently reviewed identity backup/restore preserving durable anti-rollback state; installed mixed-version acceptance. A signed contact or invitation does not grant authority; no unsafe private-key export was added. Existing previously issued presence observations expire normally.
- External dependency: Genuine surviving quorum; no original-identity recovery without safe durable-state restoration. Retired legacy/offline devices may still retain previous bytes; no remote erase guarantee.
- Commit: See locally verified lost-device retirement and contact milestone in git history.

### #26 — Sabathon P0 (baseline) / P1 (full identity mapping) · Missing
- Work: Manage deterministic `server.properties` at Fabric launch (`server-ip=127.0.0.1`, per-world loopback port, explicit `online-mode`, `enable-rcon=false`, baseline allowlist); in P1 connect private SwarmCraft membership to full Minecraft login/session allowlist policy
- Acceptance: **P0:** Launched servers never bind `0.0.0.0:25565`, never collide across worlds on one PC, and enforce explicit `online-mode` so Mojang UUIDs/inventories survive host transfer. **P1:** Swarm membership and Minecraft game identity stay consistently enforced so unauthorized game sessions are rejected
- State: **LOCALLY VERIFIED** (deterministic baseline and Linux live socket ownership; full host-process and P1 account/session enforcement remain pending)
- Source: `crates/swarm-cli/src/managed_server_properties.rs`, `crates/swarm-cli/src/migration.rs`, `crates/swarm-cli/tests/host_process.rs`, [Minecraft session policy](MINECRAFT_SESSION_POLICY.md).
- Evidence: Stable unique per-world loopback ports, exclusive properties creation, explicit online mode, RCON/query disabled. Linux readiness now requires the supervised process to own the exact loopback listener, both at startup and on live Fabric challenge. Actual child-process/socket regressions reject foreign ownership and wildcard binds even when a TCP connect succeeds. Strict Clippy and CLI library 161 passed/2 ignored locally. Full host-process and migration fixtures returned early due to the unchanged RAM admission threshold; they are not counted as completed live scenarios.
- Remaining / next action: Run the full host and migration fixtures with sufficient available memory; implement equivalent native socket-ownership checks on Windows/macOS; implement and verify the P1 authenticated Minecraft account/device binding protocol. No account allowlist or direct-local-login enforcement is claimed.
- External dependency: At least 4543 MiB available RAM for the selected 512 MiB heap on this fixture; Java 25 and Gradle for real Fabric builds; native OS and authenticated Minecraft login/transfer acceptance.
- Commit: 2d45f97 baseline; see locally verified listener-hardening milestone in git history.

### #27 — P2 · Proposal
- Work: Define public/unlisted/private visibility behavior and moderation roles before public rollout
- Acceptance: Visibility never bypasses membership. Admin actions and role changes can be reviewed, and private data does not leak to discovery
- State: **LOCALLY VERIFIED**
- Source: crates/swarm-cli/src/{discovery,historical_authority,main}.rs; crates/swarm-storage/src/state.rs; crates/swarm-cli/tests/{discovery_freshness,discovery_network_freshness,automatic_invite_join}.rs; scripts/e2e/private_infrastructure_smoke.py; docs/WORLD_GOVERNANCE.md
- Evidence: Exact signed config/roster/epoch/fence/checkpoint rechecked before discovery; fresh announcing-authority vote required in addition to current/joint majority. Invite-only admission remains independent of visibility. Durable local listing pause/resume, malformed-marker denial, signed read-only governance history, role diffs and staged-versus-committed retirement evidence. 161 CLI library tests (2 ignored), 7 discovery freshness tests, 5 discovery network tests, authenticated outsider denial for all three visibility modes, retirement audit, strict Clippy, CLI builds, and real localhost locator/publisher/relay smoke pass. Storage regressions pass. Connection-proof retry timing repaired without bypassing authentication.
- Remaining / next action: Canonical signed visibility editing requires bounded configuration-history catch-up for offline replicas and explicit publication confirmation. Separate moderator/owner grants, abuse-report workflow, native account/session policy, mixed-version and installed cross-platform/public-network acceptance remain pending. Local unlisting is not a global policy change or retroactive erasure of prior disclosures.
- External dependency: Independent governance/privacy review and actual public-rollout field validation.
- Commit: See 25 November governance/discovery privacy checkpoint.

### #28 — Sabathon P0 · Partial
- Work: Show accepted checkpoint, last replica acknowledgment, size, lag, and progress since the last recoverable save
- Acceptance: A player can tell how much progress is at risk. A locally created but unreplicated save has different status from a verified replica
- State: **LOCALLY VERIFIED**
- Source: apps/desktop/
- Evidence: Live integration status; scoped automated evidence, exact-SHA CI pending.
- Remaining / next action: Measure actual lost gameplay window; confirm current Desktop checks
- External dependency: Not yet established; assess during execution.
- Commit: 2d45f97 baseline; see live status for individual commits

### #29 — P1 · Partial
- Work: Add backup retention, disk quota, and space recovery controls on top of current storage primitives
- Acceptance: The newest recoverable checkpoint and in-use roots survive cleanup. Low disk space pauses with a usable remediation path
- State: **LOCALLY VERIFIED**
- Source: `crates/swarm-storage/src/{space,retention,replica,streaming,scheduler,stale_replay,stale_publication}.rs`; `crates/swarm-cli/src/{main,daemon,migration,historical_authority}.rs`; `crates/swarm-storage/tests/storage_space.rs`; `crates/swarm-cli/tests/automatic_invite_join.rs`; `docs/STORAGE_RETENTION_AND_QUOTAS.md`.
- Evidence: Persistent per-world payload quota and filesystem reserve; authenticated STORAGE_PAUSED retry preserves partial offsets; explicit verified cleanup retains all committed checkpoints and in-use roots, and refuses incomplete history or pinned replay. Local storage library 67 passed (1 ignored), all storage integration suites passed including 7/7 new storage tests; CLI library 161 passed (2 ignored), CLI binary 51 passed. Real two-daemon quota/cleanup/resume test restored exact bytes; existing multi-peer invite/bootstrap and historical successor replay regression passed. Strict all-target/all-feature Clippy and formatting passed.
- Remaining / next action: Desktop controls, cross-platform actual disk-exhaustion field verification, safe signed-history compaction, and proof-based reclamation of legacy crash-persisted pins remain. Payload admission is not a hard global OS quota; preserve all signed checkpoints rather than unsafe count-based pruning.
- External dependency: Installed-platform/real disk-exhaustion certification and exact-SHA CI remain outstanding.
- Commit: See 25 November storage retention/quota checkpoint.

### #30 — P1 · Partial
- Work: Finish manual checkpoint selection, verified restore, and conflict inspection UX
- Acceptance: User can inspect/export both divergent histories, choose an explicit recovery outcome, and keep a recoverable original copy
- State: **IN PROGRESS**
- Source: Pending item-specific source mapping; see original roadmap links.
- Evidence: Historical roadmap baseline only; current item-specific evidence pending.
- Remaining / next action: Inspect current source against acceptance; implement missing dependencies and test.
- External dependency: Not yet established; assess during execution.
- Commit: Pending item-specific attribution

### #31 — P1 · Proposal
- Work: Offer an encrypted checkpoint export/import bundle for backup and deliberate transport. Treat Drive or object storage as optional transport adapters
- Acceptance: An interrupted upload never becomes a current save. The recipient validates immutable manifest/blob hashes and signature before restore. Backup cannot grant host authority
- State: **NOT STARTED**
- Source: Pending item-specific source mapping; see original roadmap links.
- Evidence: Historical roadmap baseline only; current item-specific evidence pending.
- Remaining / next action: Inspect current source against acceptance; implement missing dependencies and test.
- External dependency: Not yet established; assess during execution.
- Commit: Pending item-specific attribution

### #32 — P2 · Partial
- Work: Download unrelated missing blobs in parallel from multiple authorized replicas, with bounded scheduling and resume
- Acceptance: Compare time to verified restore against source fallback. Failure or corruption from one source does not restart or invalidate safe work from others
- State: **IN PROGRESS**
- Source: Pending item-specific source mapping; see original roadmap links.
- Evidence: Historical roadmap baseline only; current item-specific evidence pending.
- Remaining / next action: Inspect current source against acceptance; implement missing dependencies and test.
- External dependency: Not yet established; assess during execution.
- Commit: Pending item-specific attribution

### #33 — P2 · Missing
- Work: Evaluate incremental region/chunk checkpoints and snapshot scheduling
- Acceptance: Demonstrate lower bytes and shorter save window on a controlled dataset. A known full restore still works and is cross-checked
- State: **NOT STARTED**
- Source: Pending item-specific source mapping; see original roadmap links.
- Evidence: Historical roadmap baseline only; current item-specific evidence pending.
- Remaining / next action: Inspect current source against acceptance; implement missing dependencies and test.
- External dependency: Not yet established; assess during execution.
- Commit: Pending item-specific attribution

### #34 — P2 · Research
- Work: Evaluate operation journaling and erasure coding independently
- Acceptance: Proven improvement against an existing snapshot baseline; fault tolerance, restore complexity, and storage cost are measured before new live formats
- State: **NOT STARTED**
- Source: Pending item-specific source mapping; see original roadmap links.
- Evidence: Historical roadmap baseline only; current item-specific evidence pending.
- Remaining / next action: Inspect current source against acceptance; implement missing dependencies and test.
- External dependency: Not yet established; assess during execution.
- Commit: Pending item-specific attribution

### #35 — P2 · Unverified
- Work: Publish capacity with separate player, voter, replica, world-size, and relay tests
- Acceptance: Real game-server measurements report player counts, TPS, and resource ceilings. Replica count is never presented as extra simulation capacity
- State: **NOT STARTED**
- Source: Pending item-specific source mapping; see original roadmap links.
- Evidence: Historical roadmap baseline only; current item-specific evidence pending.
- Remaining / next action: Inspect current source against acceptance; implement missing dependencies and test.
- External dependency: Not yet established; assess during execution.
- Commit: Pending item-specific attribution

### #36 — P2 · Proposal
- Work: Add background replica scheduling, quotas, bandwidth windows, and replica placement controls
- Acceptance: A user can choose what to cache. Hot worlds retain the configured recoverability target under normal churn without overrunning disk or bandwidth
- State: **NOT STARTED**
- Source: Pending item-specific source mapping; see original roadmap links.
- Evidence: Historical roadmap baseline only; current item-specific evidence pending.
- Remaining / next action: Inspect current source against acceptance; implement missing dependencies and test.
- External dependency: Not yet established; assess during execution.
- Commit: Pending item-specific attribution

### #37 — Release-Gate P0 · Unverified
- Work: Implement installed Windows/macOS native GUI QA for #141 using platform-valid automation
- Acceptance: Exercise actual WebView2 (`msedgedriver`) on Windows and native WKWebView automation (Accessibility/XCUITest or isolated QA bridge, not `tauri-driver`) on Apple Silicon & Intel macOS with real sidecars, real IPC, restart, host consent, and an intentional failure in a separate QA build
- State: **NOT STARTED**
- Source: Pending item-specific source mapping; see original roadmap links.
- Evidence: Historical roadmap baseline only; current item-specific evidence pending.
- Remaining / next action: Inspect current source against acceptance; implement missing dependencies and test.
- External dependency: Not yet established; assess during execution.
- Commit: Pending item-specific attribution

### #38 — P1 · Partial
- Work: Decide Svelte cutover after shipping parity review (post-sabathon)
- Acceptance: The candidate covers all legacy journeys, including private invite, host transfer, loopback reconnect, safe stop/wake, mod setup, and recovery. Installed builds and rollback prove the packaging switch
- State: **IN PROGRESS**
- Source: Pending item-specific source mapping; see original roadmap links.
- Evidence: Historical roadmap baseline only; current item-specific evidence pending.
- Remaining / next action: Inspect current source against acceptance; implement missing dependencies and test.
- External dependency: Not yet established; assess during execution.
- Commit: Pending item-specific attribution

### #39 — P1 · Partial
- Work: Simplify first run and import. Add a native folder picker when supported and show source/runtime requirements before commit
- Acceptance: A fresh user can import a supported world, finish explicit EULA, see capacity and mod blockers, and recover an interrupted install
- State: **IN PROGRESS**
- Source: Pending item-specific source mapping; see original roadmap links.
- Evidence: Historical roadmap baseline only; current item-specific evidence pending.
- Remaining / next action: Inspect current source against acceptance; implement missing dependencies and test.
- External dependency: Not yet established; assess during execution.
- Commit: Pending item-specific attribution

### #40 — P1 · Partial
- Work: Deliver compatible game-client preparation and handoff to the chosen launcher. Preserve the user's account and settings
- Acceptance: Matching Java/Fabric/mod set launches or is clearly directed. Existing Mojang/Microsoft sign-in and entitlement remain intact. No credentials travel through invites
- State: **IN PROGRESS**
- Source: Pending item-specific source mapping; see original roadmap links.
- Evidence: Historical roadmap baseline only; current item-specific evidence pending.
- Remaining / next action: Inspect current source against acceptance; implement missing dependencies and test.
- External dependency: Not yet established; assess during execution.
- Commit: Pending item-specific attribution

### #41 — P1 · Partial
- Work: Finish modpack dependency acquisition for each supported provider and manual-artifact fallback
- Acceptance: A pristine target acquires allowed artifacts from their authorized source, verifies exact bytes, and shows unavailable or incompatible components. No silent redistribution
- State: **IN PROGRESS**
- Source: Pending item-specific source mapping; see original roadmap links.
- Evidence: Historical roadmap baseline only; current item-specific evidence pending.
- Remaining / next action: Inspect current source against acceptance; implement missing dependencies and test.
- External dependency: Not yet established; assess during execution.
- Commit: Pending item-specific attribution

### #42 — P2 · Partial
- Work: Add a compatibility matrix for Minecraft versions and Fabric builds. Develop and test an adapter before widening support
- Acceptance: Every offered tuple passes a real server launch, save, restore, and game-client connection. Unsupported versions remain visible with accurate messaging or filtering
- State: **IN PROGRESS**
- Source: Pending item-specific source mapping; see original roadmap links.
- Evidence: Historical roadmap baseline only; current item-specific evidence pending.
- Remaining / next action: Inspect current source against acceptance; implement missing dependencies and test.
- External dependency: Not yet established; assess during execution.
- Commit: Pending item-specific attribution

### #43 — P2 · Proposal
- Work: Improve accessibility and everyday polish across main actions
- Acceptance: Both supported window sizes, keyboard-only flow, screen-reader names/live state, scaling, long names and IDs, and interrupted progress pass on installed builds
- State: **NOT STARTED**
- Source: Pending item-specific source mapping; see original roadmap links.
- Evidence: Historical roadmap baseline only; current item-specific evidence pending.
- Remaining / next action: Inspect current source against acceptance; implement missing dependencies and test.
- External dependency: Not yet established; assess during execution.
- Commit: Pending item-specific attribution

### #44 — P2 · Research
- Work: Evaluate additional loaders, Bedrock clients, and cross-play separately
- Acceptance: Work only from a proven Fabric baseline. Each new adapter documents worlds, mods, game protocol, lifecycle limits, and cross-platform evidence
- State: **NOT STARTED**
- Source: Pending item-specific source mapping; see original roadmap links.
- Evidence: Historical roadmap baseline only; current item-specific evidence pending.
- Remaining / next action: Inspect current source against acceptance; implement missing dependencies and test.
- External dependency: Not yet established; assess during execution.
- Commit: Pending item-specific attribution

### #45 — P2 · Partial
- Work: Build the normal-player world lobby on signed discovery and provider results
- Acceptance: Public search and exact unlisted resolve have useful empty states, bounded paging, freshness display, and a separate private Join action
- State: **IN PROGRESS**
- Source: Pending item-specific source mapping; see original roadmap links.
- Evidence: Historical roadmap baseline only; current item-specific evidence pending.
- Remaining / next action: Inspect current source against acceptance; implement missing dependencies and test.
- External dependency: Not yet established; assess during execution.
- Commit: Pending item-specific attribution

### #46 — P2 · Partial
- Work: Connect friend contact/presence primitives to explicit consent, hide/offline mode, and useful invitations
- Acceptance: Friends see only the metadata a user agreed to share. Removing a contact stops future private presence access. Offline has honest freshness
- State: **IN PROGRESS**
- Source: Pending item-specific source mapping; see original roadmap links.
- Evidence: Historical roadmap baseline only; current item-specific evidence pending.
- Remaining / next action: Inspect current source against acceptance; implement missing dependencies and test.
- External dependency: Not yet established; assess during execution.
- Commit: Pending item-specific attribution

### #47 — P2 · Proposal
- Work: Add public-world governance and abuse handling once private play is stable
- Acceptance: Listing visibility, moderation, region/language metadata, report flow, and local filtering work without granting registry veto power over world history
- State: **NOT STARTED**
- Source: Pending item-specific source mapping; see original roadmap links.
- Evidence: Historical roadmap baseline only; current item-specific evidence pending.
- Remaining / next action: Inspect current source against acceptance; implement missing dependencies and test.
- External dependency: Not yet established; assess during execution.
- Commit: Pending item-specific attribution

### #48 — P2 · Proposal
- Work: Add opt-in, redacted diagnostics export and a useful local history of world actions
- Acceptance: Users can export time, commit, path class, and event summaries while excluding private invites, credentials, raw private addresses, and world data
- State: **NOT STARTED**
- Source: Pending item-specific source mapping; see original roadmap links.
- Evidence: Historical roadmap baseline only; current item-specific evidence pending.
- Remaining / next action: Inspect current source against acceptance; implement missing dependencies and test.
- External dependency: Not yet established; assess during execution.
- Commit: Pending item-specific attribution

### #49 — Sabathon P0 · Partial
- Work: Reconcile open issues, PRs, and capability claims against main
- Acceptance: Every stale ticket has a landed commit or exact residual; open acceptance blockers remain linked. README, roadmap, and install instructions agree
- State: **EXTERNALLY BLOCKED**
- Source: docs/SABATHON_IMPLEMENTATION_PLAN.md
- Evidence: Live integration status; scoped automated evidence, exact-SHA CI pending.
- Remaining / next action: Compare integration changes to issue ledger; defer main-only reconciliation until authorized merge
- External dependency: Authorized main merge for main-only closure
- Commit: 2d45f97 baseline; see live status for individual commits

### #50 — Release-Gate P0 · Partial
- Work: Verify an immutable main snapshot and rollback, then establish production release evidence
- Acceptance: Preview retains previous assets and validates source/version/hash. Tagged release independently verifies Windows signing and Mac notarization; mobile and unsupported packages are clearly scoped out
- State: **IN PROGRESS**
- Source: Pending item-specific source mapping; see original roadmap links.
- Evidence: Historical roadmap baseline only; current item-specific evidence pending.
- Remaining / next action: Inspect current source against acceptance; implement missing dependencies and test.
- External dependency: Not yet established; assess during execution.
- Commit: Pending item-specific attribution

### #51 — P1 · Partial
- Work: Extend release resilience for download outages, executable provenance, shared runtime cache publication, and dependency health
- Acceptance: Corrupt input fails validation. Same-runtime installs from two worlds preserve the last known-good cache. Every generated or downloaded executable has explicit provenance and verification status
- State: **IN PROGRESS**
- Source: Pending item-specific source mapping; see original roadmap links.
- Evidence: Historical roadmap baseline only; current item-specific evidence pending.
- Remaining / next action: Inspect current source against acceptance; implement missing dependencies and test.
- External dependency: Not yet established; assess during execution.
- Commit: Pending item-specific attribution

### #52 — P1 · Partial
- Work: Establish upgrade, downgrade, schema, and interrupted migration acceptance from the supported older preview
- Acceptance: One real saved world and consent history survive upgrade and restart. Unsupported downgrade stops clearly, with verified export/rollback guidance
- State: **IN PROGRESS**
- Source: Pending item-specific source mapping; see original roadmap links.
- Evidence: Historical roadmap baseline only; current item-specific evidence pending.
- Remaining / next action: Inspect current source against acceptance; implement missing dependencies and test.
- External dependency: Not yet established; assess during execution.
- Commit: Pending item-specific attribution

### #53 — P1 · Partial
- Work: Extend crash, disk-full, retention, platform resource, and network campaigns beyond current automated gates
- Acceptance: Published stress matrix includes repetitions, actual resource limits, last accepted checkpoint, and explicit failure causes. Saved canonical data remains recoverable
- State: **IN PROGRESS**
- Source: Pending item-specific source mapping; see original roadmap links.
- Evidence: Historical roadmap baseline only; current item-specific evidence pending.
- Remaining / next action: Inspect current source against acceptance; implement missing dependencies and test.
- External dependency: Not yet established; assess during execution.
- Commit: Pending item-specific attribution

### #54 — P2 · Proposal
- Work: Add signed update delivery and platform installation hygiene after release provenance is stable
- Acceptance: Client verifies manifest and package signature, separates preview/stable channels, supports opt-out, and rejects an incompatible or untrusted update
- State: **NOT STARTED**
- Source: Pending item-specific source mapping; see original roadmap links.
- Evidence: Historical roadmap baseline only; current item-specific evidence pending.
- Remaining / next action: Inspect current source against acceptance; implement missing dependencies and test.
- External dependency: Not yet established; assess during execution.
- Commit: Pending item-specific attribution

### #55 — P2 · Missing
- Work: Define production metrics, support runbooks, and per-version compatibility policy
- Acceptance: Field team can diagnose boot, replication, lease, game connect, restore, and publication with bounded and redacted evidence
- State: **NOT STARTED**
- Source: Pending item-specific source mapping; see original roadmap links.
- Evidence: Historical roadmap baseline only; current item-specific evidence pending.
- Remaining / next action: Inspect current source against acceptance; implement missing dependencies and test.
- External dependency: Not yet established; assess during execution.
- Commit: Pending item-specific attribution

### #56 — Expansion · Design prerequisite
- Work: Reopen the distributed region/tick feasibility decision for the expanded product scope (revisiting the current Fabric NO-GO). Choose the experimental runtime before implementation
- Acceptance: Compare the existing single-host baseline with actual multi-host simulation. Document cross-region correctness, compatibility limits, and measured performance before adopting the runtime
- State: **NOT STARTED**
- Source: Pending item-specific source mapping; see original roadmap links.
- Evidence: Historical roadmap baseline only; current item-specific evidence pending.
- Remaining / next action: Inspect current source against acceptance; implement missing dependencies and test.
- External dependency: Not yet established; assess during execution.
- Commit: Pending item-specific attribution

### #57 — Expansion · Missing
- Work: Add per-device CPU and memory contribution, with desired limits, measured capacity, and a player-only choice
- Acceptance: Joining grants no compute access automatically. A player can select 4 GiB and a CPU limit, change or withdraw it, and see requested, admitted, and active resources. Zero preserves play where available
- State: **NOT STARTED**
- Source: Pending item-specific source mapping; see original roadmap links.
- Evidence: Historical roadmap baseline only; current item-specific evidence pending.
- Remaining / next action: Inspect current source against acceptance; implement missing dependencies and test.
- External dependency: Not yet established; assess during execution.
- Commit: Pending item-specific attribution

### #58 — Expansion · Missing
- Work: Build opt-in worker admission and placement based on usable memory, CPU, network path, and region load
- Acceptance: A newly admitted machine takes real simulation work only when useful. Inadequate capacity scales down safely. Platform limits and heap versus process memory are visible
- State: **NOT STARTED**
- Source: Pending item-specific source mapping; see original roadmap links.
- Evidence: Historical roadmap baseline only; current item-specific evidence pending.
- Remaining / next action: Inspect current source against acceptance; implement missing dependencies and test.
- External dependency: Not yet established; assess during execution.
- Commit: Pending item-specific attribution

### #59 — Expansion · Missing
- Work: Choose or build a region-aware runtime. Keep one world identity as different machines simulate different regions
- Acceptance: Players traverse a continuous world while independent machines tick different regions. The supported Minecraft/protocol/mod matrix is documented and verified
- State: **NOT STARTED**
- Source: Pending item-specific source mapping; see original roadmap links.
- Evidence: Historical roadmap baseline only; current item-specific evidence pending.
- Remaining / next action: Inspect current source against acceptance; implement missing dependencies and test.
- External dependency: Not yet established; assess during execution.
- Commit: Pending item-specific attribution

### #60 — Expansion · Missing
- Work: Bind every region to a single authorized writer and generation. Define the ownership of global game state
- Acceptance: Region reassignment, overlap, and worker restart never produce two canonical writers for the same region. World-wide saves include a consistent accepted state
- State: **NOT STARTED**
- Source: Pending item-specific source mapping; see original roadmap links.
- Evidence: Historical roadmap baseline only; current item-specific evidence pending.
- Remaining / next action: Inspect current source against acceptance; implement missing dependencies and test.
- External dependency: Not yet established; assess during execution.
- Commit: Pending item-specific attribution

### #61 — Expansion · Missing
- Work: Make player, entity, and inventory transitions durable across regions, including block and global interactions in the supported ruleset
- Acceptance: Players walk or use a supported portal across workers and preserve identity, state, and inventory through restart. Supported cross-region mechanics pass replay and correctness tests
- State: **NOT STARTED**
- Source: Pending item-specific source mapping; see original roadmap links.
- Evidence: Historical roadmap baseline only; current item-specific evidence pending.
- Remaining / next action: Inspect current source against acceptance; implement missing dependencies and test.
- External dependency: Not yet established; assess during execution.
- Commit: Pending item-specific attribution

### #62 — Expansion · Missing
- Work: Handle scale-in, loss, and partition. Drain and reassign live work when volunteers leave
- Acceptance: A graceful exit drains eligible work and releases the device budget. A crash freezes or fails over affected regions from accepted checkpoints without duplicating state
- State: **NOT STARTED**
- Source: Pending item-specific source mapping; see original roadmap links.
- Evidence: Historical roadmap baseline only; current item-specific evidence pending.
- Remaining / next action: Inspect current source against acceptance; implement missing dependencies and test.
- External dependency: Not yet established; assess during execution.
- Commit: Pending item-specific attribution

### #63 — Expansion · Unverified
- Work: Run real distributed gameplay and publish the scale envelope
- Acceptance: Compare the same world, load, and network with one worker and multiple independent workers. Show increased sustainable active regions or player capacity at the published service target. Publish clustered-load and degraded-network limits
- State: **NOT STARTED**
- Source: Pending item-specific source mapping; see original roadmap links.
- Evidence: Historical roadmap baseline only; current item-specific evidence pending.
- Remaining / next action: Inspect current source against acceptance; implement missing dependencies and test.
- External dependency: Not yet established; assess during execution.
- Commit: Pending item-specific attribution

### #64 — Expansion · Missing
- Work: Add a creation-time **Ownerless / no OP** world option with a signed governance policy
- Acceptance: The founding player receives no permanent creator privileges, player OP, unilateral rule changes, or unilateral control over recovery. Turning on more compute grants no governance privilege
- State: **NOT STARTED**
- Source: Pending item-specific source mapping; see original roadmap links.
- Evidence: Historical roadmap baseline only; current item-specific evidence pending.
- Remaining / next action: Inspect current source against acceptance; implement missing dependencies and test.
- External dependency: Not yet established; assess during execution.
- Commit: Pending item-specific attribution

### #65 — Expansion · Missing
- Work: Bind enforced permission rules to the canonical runtime and each regional worker, with explicit CFT vs. BFT trust boundaries
- Acceptance: On unmodified runtimes, the founder and successor hosts have ordinary in-game permissions, and `/op`, RCON, and privileged management interfaces are disabled. Documentation explicitly separates software-enforced governance from OS-level host tampering
- State: **NOT STARTED**
- Source: Pending item-specific source mapping; see original roadmap links.
- Evidence: Historical roadmap baseline only; current item-specific evidence pending.
- Remaining / next action: Inspect current source against acceptance; implement missing dependencies and test.
- External dependency: Not yet established; assess during execution.
- Commit: Pending item-specific attribution

### #66 — Expansion · Missing
- Work: Define shared authority, validator admission, and loss recovery for ownerless worlds
- Acceptance: A verified majority can keep the world available after the founder leaves permanently. No single key can publish a unilateral policy change or rewrite accepted state. A lost majority leaves verified recovery copies
- State: **NOT STARTED**
- Source: Pending item-specific source mapping; see original roadmap links.
- Evidence: Historical roadmap baseline only; current item-specific evidence pending.
- Remaining / next action: Inspect current source against acceptance; implement missing dependencies and test.
- External dependency: Not yet established; assess during execution.
- Commit: Pending item-specific attribution

### #67 — Expansion · Missing
- Work: Separate world governance, public/private participation, and volunteer worker trust
- Acceptance: Ownerless can be invite-only or public. Joining a public world grants neither a vote nor write authority. Anonymous game access and eligibility to run or certify workers are separate policies
- State: **NOT STARTED**
- Source: Pending item-specific source mapping; see original roadmap links.
- Evidence: Historical roadmap baseline only; current item-specific evidence pending.
- Remaining / next action: Inspect current source against acceptance; implement missing dependencies and test.
- External dependency: Not yet established; assess during execution.
- Commit: Pending item-specific attribution

### #68 — Expansion · Unverified
- Work: Certify ownerless and horizontally scaled operation together
- Acceptance: In one real game session, the founder exits permanently, other peers keep the same world available with no player OP, opt-in worker resources change, and players continue across reassigned regions when the required quorum and capacity survive
- State: **NOT STARTED**
- Source: Pending item-specific source mapping; see original roadmap links.
- Evidence: Historical roadmap baseline only; current item-specific evidence pending.
- Remaining / next action: Inspect current source against acceptance; implement missing dependencies and test.
- External dependency: Not yet established; assess during execution.
- Commit: Pending item-specific attribution

## Next checkpoint

Continue #4: planned and crash-recovered authority re-entry, exact certified voting fences, and durable crash-retirement replay implemented. Targeted successor proof offers and certified retargeting after membership changes are implemented. Pending join/leave and solo conflicts remain conservatively blocked; independent private WAN reachability remains unverified. Item #9 explicit headless profile is locally verified; field certification remains open. Item #8 online-overlap RFC and isolated state model implemented, production activation remains gated. Continue #11 recovered-copy workflow and other actionable roadmap items. Keep publication and runtime fail-closed. #9 precedes #8; #23 precedes activation of V2-dependent permissions; #56 precedes distributed-runtime adoption; #64–68 require signed governance and independent acceptance.

Initial full-item implementation/acceptance totals: not yet established by item-specific audit; zero items asserted fully accepted by this ledger.

### 25 November: exact certified fence reconciliation

- #4: `stale_publication.rs` now accepts only byte-exact transfer phases and recovery/membership promises represented in the verified history; consumption occurs after all canonical records are written and verified, while the intent still fences ordinary access. Other fences remain blocking.
- Evidence: expanded signed replay regression passed, storage library 67 passed/1 optional soak ignored, strict storage/CLI all-target/all-feature Clippy passed, formatting and diff checks passed. No complete backlog item is newly claimed.
- Next: prove retired-authority re-entry using the runtime ownership guard, fail-closed permit/lease and route behavior, and orphan handling; do not merely remove the authority exclusion.

### 25 November: planned-retirement re-entry

- Previous fence reconciliation committed as `0ef8d21`. New daemon flow allows a retired local authority to stage/download exact successor history only when it retains its signed committed outbound transfer, and only if the full anchored proof contains that exact immediate transition.
- Before publication, the caller acquires the same world runtime lock as hosting, rejects ambiguous or live persistent JVM records, clears the old permit and invalidates runtime verification. Storage repeats the exact retirement check under its transaction lock. The intent remains the visibility fence and no local authority or consent is created.
- A retired authority without the matching committed transfer remains blocked; crash-recovery retirement requires a separate durable denial fence rather than simply allowing old authorities to replay. No divergent or pruned history is overwritten.
- Strict storage/CLI Clippy and all 135 CLI library tests passed. The real process-level invite/replay regression passed both extended runs, including strengthened direct orphan-record and held-lock denial assertions (final run: 46.32 seconds).

### 25 November: crash-recovery retirement

- Required validation 45521764635 on d6bf294 completed successfully: 20 selected jobs passed, optional sustained soak skipped.
- #4: a recovered-away former authority now verifies the anchored recovery quorum and entire history, obtains exclusive runtime ownership, denies live/ambiguous JVM records, clears old permits/readiness, and installs an exact durable retirement envelope before isolated downloads. Ordinary authority/replica writes, reads and game routing remain fenced; only the pinned successor can send historical bytes. Retargeting must extend the pinned signed history. The marker survives restart and is consumed only under the publication intent after exact canonical installation.
- Evidence: strict storage/CLI Clippy, formatting, 67 storage library tests passed (1 optional soak ignored), 136 CLI library tests passed, and real authenticated multi-daemon invite/recovery/restart test passed (65.65 seconds). The test uses signed synthetic recovery fixtures rather than a physical Minecraft JVM crash. No complete roadmap item is newly claimed.
- Next: reconcile exact pending recovery certificates, signed sleep and reservation decisions; investigate authenticated discovery of successors admitted after the local anchor.

### 25 November: certified boundary reconciliation

- #4: stale publication now accepts either recovery-certificate slot only when the complete canonical record appears in the verified history; an accepted recovery anchor still requires its exact proof. Pending certificates are cleared only after archival and canonical installation. Signed sleep records must match a certified wake/recovery boundary, and signed reservations must bind a strictly superseded epoch in the proof. CLI verifies both local signatures independently. Both records are archived before removal under the publication intent. Final/newer reservations, unrelated sleep, malformed records and unresolved decisions remain blocking.
- Evidence: all 7 historical-authority regressions, all 136 CLI library tests, 67 storage library tests (1 optional soak ignored), formatting and strict all-target/all-feature storage/CLI Clippy passed. Includes forged signatures, conflicting and trailing-byte certificates, exact pending slot, signed boundary preservation and restart. No complete roadmap item is newly claimed.
- Next: authenticate and certify successors admitted after the stale anchor, without broadcasting private world IDs or trusting unauthenticated hints.

### 25 November: previously unknown certified successor

- #4: the accepted authority offers a complete, bounded signed history only to an authenticated current authorized member. A receiver must already possess an exact canonical anchor or committed admission; the offer itself grants neither membership nor live authority. Full history, joint admission certificates, transfer/recovery proof, exact sender, and local membership are independently verified. Only negotiated historical manifests/chunks may bypass the obsolete local roster; all ordinary RPCs remain fenced. The receipt reveals no local world existence or verification outcome. Retargeting requires certified membership advancement, not merely another checkpoint. Existing pull compatibility remains.
- Evidence: strict all-target/all-feature Clippy, 136 CLI/114 network/67 storage library tests (1 optional soak ignored), both real multi-daemon invite/replay process tests (154.93s), formatting and diff checks. The offline first-contact player resumed a partial download from a newly admitted successor absent from its original membership, and published only the final epoch. The crash-retired unit regression separately verifies a subsequently admitted successor, missing joint certificate, wrong sender, and checkpoint-only target denial. The process fixtures use signed synthetic transitions, not real Minecraft gameplay.
- Remaining: private reachability across independent networks, stale-replica unknown-successor transport field validation, pending membership/solo conflict resolution, and the installed gameplay acceptance. No full backlog item newly claimed.

### 25 November: headless third voting replica

- #9: implemented explicit `swarmcraft daemon --replica-only`. The profile holds exclusive local memory/profile locks and refuses active or ambiguous prior runtime ownership. No migration supervisor is launched, stale permits/readiness are invalidated, and host candidacy/transfer activation/local permit renewal and positive opt-in are denied while the profile is active. Existing consent revisions are preserved; explicit revocation remains available. Ordinary signed replication and voting are unchanged.
- Evidence: 138 CLI library tests (including positive prior consent, shared/exclusive profile lock and ambiguous runtime negatives), 4 real public CLI consent tests (including two replica-only process starts, stale permit removal and opt-in denial without a JVM), strict CLI all-target/all-feature Clippy, and the real three-daemon recovery/wake process test (31.52s), including signed recovery votes by the replica-only peer, no local runtime/permit, and restart. The host side uses a mock JVM, not field Minecraft.
- Added dedicated deployment guide and example systemd service; these were not installed on the user's machine. Dedicated low-memory/independent-network operation and real offline handoff remain field acceptance. Next: #8 reviewed exclusive-turn design (no weakening default quorum), and additional locally testable release/UX/replication work.

### 25 November: exclusive-turn design and reference model

- #8: chose explicit online-overlap handoff for a separate, fixed two-member policy, no automatic crash recovery, immutable release after old runtime is quiesced, exact recipient receipt/checkpoint and strictly advancing generation. The design explicitly distinguishes world-data rollback from undetectable complete control-journal rollback and retains new-world recovery for lost grants. Async dropbox is a future transport-only option.
- Five isolated model regressions passed; bounded exploration checked 3,165 unique independent-journal states with partition, restart, checkpoint loss, canceled attempts, reverse transfer and delayed/duplicate records. All 51 consensus unit/integration tests and strict all-target/all-feature Clippy passed.
- No live policy, production signing/journal, UI, permit or route changes activated. Independent review and actual two-device field tests remain open. Continue #11 and other locally actionable work without loosening the existing quorum rules.

### 25 November: recovered private world copy

- #11: explicit CLI and Desktop recovery copy pins the selected manifest hash, verifies the complete retained signed history and blob bytes without manufacturing an original-authority replay envelope, and publishes a newly signed private world only after staging and verification. Its independent genesis preserves the exact file state root and compatibility contract. Original data/control records remain unchanged; source memberships, runtime, consent, invitations and recovery authority are not inherited. Incomplete, pruned, conflicting and unsupported legacy histories remain fail-closed.
- Local evidence: strict CLI Clippy; 140 CLI library tests; both actual multi-daemon invite/recovery tests, including an offline replica whose original authority is different (147.69s); native Tauri cargo check; 137 Desktop tests with real headless Chrome at 980x760, 720x560, 1280x900, using a temporary version-matched ChromeDriver and mocked IPC for UI actions; 29 Svelte source parity tests; Svelte type check and 82 unit tests. The browser caught and verified the fix for a delayed close event clearing a reopened dialog's selection. Corrected the previously stale typed join_world hostCandidate argument while checking Svelte.
- The recovered-copy browser receipt identifies both world IDs, with explicit confirmation and no original authority change. Actual installed native Minecraft recovery and exact-SHA CI remain separate acceptance. See docs/RECOVERED_COPY.md. Continue #10 invite lifecycle and further locally testable items.

### 25 November: authenticated invite feedback and removed-member replication

- #10: connection-authenticated issuer responses are correlated with the exact outstanding join request (world, signed grant hash, issuer and nonce). Only fixed revoked/expired reasons are stored as local diagnostics, never as authority, membership, or cancellation. The pending status survives process restart; unrelated/late responses cannot attach to a new request.
- Private replication uses the current signed membership with a matching descriptor and rechecks the exact authenticated recipient before normal payload, manifest acknowledgement and every blob chunk. A half-installed signed roster denies data instead of trusting an obsolete descriptor. Removed local recipients also refuse normal snapshot bytes. Ordinary unauthorized world requests receive a generic refusal.
- Evidence: strict CLI/storage Clippy, 51 daemon/CLI binary tests, 67 storage tests (1 optional soak ignored), and actual authenticated invite/leave/replication test passed (19.73s). A reusable grant admitted a third peer; revocation left committed membership intact; joint leave committed; a real network probe authenticated as the removed member received status/history refusal and no newer manifest/blob payload while the remaining member verified a new checkpoint. The old device retained its old verified save. Prior commit 8653c2d required CI completed successfully (run 45585530110). Broader local CLI validation underway; UI and field acceptance remain.
