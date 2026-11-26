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
- State: **LOCALLY VERIFIED**
- Source: `crates/swarm-cli/src/{recovery_review,historical_authority,world_import,main}.rs`; `crates/swarm-cli/tests/recovery_review.rs`; Desktop native bridge, shipping and Svelte diagnostics/recovery dialogs, recovery copy and browser regressions; `docs/RECOVERY_REVIEW.md`.
- Evidence: Exact historical checkpoint selection verifies certified history and payloads; conflict-set digest binds explicit acknowledgement. Independent divergent stores exported their exact respective head bytes and signed evidence without changing either original. Missing/invalid branch data, changed evidence, corrupt signatures/payloads, and unsafe destinations fail closed; selected earlier checkpoint creates a separate private identity. CLI integration passed; CLI library 161 passed (2 ignored), binary 51 passed; strict CLI/Tauri Clippy and native check passed. Desktop 147/147 (including actual Chrome rendering with mocked IPC at three window sizes), Svelte 82/82, source parity 30/30, zero Svelte diagnostics, local native acceptance script syntax and six Python acceptance regression tests passed.
- Remaining / next action: Installed native GUI/platform verification; unavailable divergent payloads must be exported on the retaining device, not silently recreated. Unsupported legacy solo history remains evidence-only where certified verification fails. No in-place merge, authority recovery, or encrypted portable backup/import is claimed. Next: #31 encrypted checkpoint bundle.
- External dependency: Installed-platform field certification; necessary final CI only under local-first policy.
- Commit: See 25 November historical recovery checkpoint.

### #31 — P1 · Proposal
- Work: Offer an encrypted checkpoint export/import bundle for backup and deliberate transport. Treat Drive or object storage as optional transport adapters
- Acceptance: An interrupted upload never becomes a current save. The recipient validates immutable manifest/blob hashes and signature before restore. Backup cannot grant host authority
- State: **LOCALLY VERIFIED**
- Source: crates/swarm-cli/src/checkpoint_backup.rs, historical_authority.rs, main.rs; crates/swarm-cli/tests/checkpoint_backup.rs; crates/swarm-storage/src/streaming.rs; docs/CHECKPOINT_BACKUP.md.
- Evidence: Generated owner-only random key file, versioned XChaCha20-Poly1305 framed file, authenticated end and strict EOF, original signed history and exact checkpoint pins, bounded deduplicated blobs, isolated verified restore, no-clobber export/import. Real CLI test restores old checkpoint including compressed, duplicate, empty and multi-frame files; rejects partial, wrong-key, wrong-pin, modified and appended transfers and unsafe paths. Four unit tests cover malicious re-encryption, invalid original signatures/blob hashes, framing and publication races. Historical replay regression exports as a storage-only member across signed transfer, recovery and later admission. 165 CLI library tests (2 ignored), 51 CLI binary tests, both backup/recovery integration tests, 67 storage tests (1 ignored), strict CLI/storage Clippy and format/diff checks passed. Full CLI regression requires the existing umask 077 private test environment.
- Remaining / next action: CLI file transport only; optional provider/Desktop integration and direct data-only hydration of pinned replicas require separate work. No automatic authority recovery, current-save activation or cloud upload is claimed. Next #32: bounded parallel unrelated blob scheduling across authorized replicas.
- External dependency: Installed-platform/Windows ACL and power-loss/large-scale certification; optional provider adapters; necessary final CI under local-first policy.
- Commit: See 25 November encrypted checkpoint backup commit.

### #32 — P2 · Partial
- Work: Download unrelated missing blobs in parallel from multiple authorized replicas, with bounded scheduling and resume
- Acceptance: Compare time to verified restore against source fallback. Failure or corruption from one source does not restart or invalidate safe work from others
- State: **LOCALLY VERIFIED**
- Source: crates/swarm-cli/src/daemon_parallel_replication.rs, daemon.rs, crates/swarm-network/src/wire.rs and node.rs, crates/swarm-storage/src/integrity.rs and streaming.rs, docs/SNAPSHOT_REPLICATION.md.
- Evidence: Signed connection capability; exact world/manifest/membership/epoch pins; four global outstanding reads/four active worlds, one writer per blob and logical source; background seeding consent; game traffic pacing; durable offset resume, corrupt-prefix clean retry, per-source backoff, pressure pause, legacy fallback and transaction-locked final publication. Two independent successful three-daemon process runs measured 4.74 s vs 8.33 s verified restore for roughly 6 MiB, with two distinct contributing sources, two concurrent reads, and unchanged unrelated verified blob through corrupt-source interruption (roughly 12 MiB, one resumed blob, two failures). All 165 CLI library tests (2 ignored), 51 binary tests, 135 network library tests, 67 storage library tests (1 ignored), focused wire/handshake/input, storage integrity/space/scheduler/resume tests, and strict Clippy passed. All six real invitation/bootstrap/successor replay/retirement/quota/privacy process tests passed, including new removed-member replica-pull denial, and the paced QUIC game-echo regression passed. The randomly generated fixture membership was sorted before signing to fix a setup-only rerun failure; the corrected process test passed without loosening validation.
- Remaining / next action: Next #33: controlled incremental region/chunk checkpoint evaluation. Receiver-driven ordinary replication only; separate historical replay remains on its certified protocol. Independent-device/WAN throughput, larger-fleet fairness and installed-platform validation remain open; explicit remote inventories are optional future optimization.
- External dependency: Field and necessary final CI under local-first policy.
- Commit: See 25 November receiver-driven replica pull commit.

### #33 — P2 · Missing
- Work: Evaluate incremental region/chunk checkpoints and snapshot scheduling
- Acceptance: Demonstrate lower bytes and shorter save window on a controlled dataset. A known full restore still works and is cross-checked
- State: **LOCALLY VERIFIED**
- Source: crates/swarm-storage/tests/incremental_checkpoint_lab.rs; crates/swarm-storage/tests/support/checkpoint_schedule_lab.rs; research/incremental-checkpoints/REPORT.md.
- Evidence: Actual V1 production streaming capture/commit/restore vs isolated 256 KiB block checkpoint candidate on eight 4 MiB synthetic region-shaped files. Initial, three sparse and dense rounds compare actual encoded payload plus manifest bytes, local transfer and durable capture timing. Three local runs measured median sparse capture 1.029/1.089/1.083 s vs 0.256/0.231/0.262 s; median sparse transfer 25,195,436 vs 1,582,212 bytes (93.7% lower). Initial/dense overhead is recorded rather than hidden. Every restored tree matches the original byte-for-byte; original and latest are cross-checked after reopening both stores. Wrong/noncanonical pins, malformed paths, reordered/missing/corrupt blocks and immutable numbered slots are tested. Deterministic schedule model requires an immutable view and exact verified receipt, preserves edits made in flight, rejects stale generations/receipts, coalesces explicit requests and reports missed recovery targets instead of bypassing gates. Full storage suite (67 library tests, one optional soak ignored, all integration tests including 4 new research regressions), strict all-target/all-feature Clippy and formatting passed.
- Remaining / next action: #34 independent operation journal and erasure coding evaluation. #33 evidence is quiescent synthetic capture-stage research, not actual Minecraft pause or live recovery-window certification. No new production format or online cadence activated. Preserve V1 until immutable capture, production retention/GC, signed format, fault recovery and installed-platform/real-world performance are independently verified.
- External dependency: Actual Fabric consistent-view protocol and real game/world/network measurement; necessary final CI under local-first policy.
- Commit: See 25 November incremental checkpoint research commit.

### #34 — P2 · Research
- Work: Evaluate operation journaling and erasure coding independently
- Acceptance: Proven improvement against an existing snapshot baseline; fault tolerance, restore complexity, and storage cost are measured before new live formats
- State: **LOCALLY VERIFIED**
- Source: crates/swarm-storage/tests/operation_journal_lab.rs, erasure_coding_lab.rs, support/research_v1_fixture.rs; research/journal-erasure/REPORT.md.
- Evidence: Independently compare actual V1 full-file capture/replication/restore against bounded ordered byte-operation journal and 4+2 GF(256) archive. Journal: three sparse rounds, 6.30 MB full delta vs 12.7–12.8 KB actual journal record; retained base+records 6,338,215 vs 25,199,566 bytes, ordered replay increases restore cost (111–124 ms vs 55–58 ms). Exact base, predecessor, preimage and whole-tree checks; missing middle record blocks suffix while base/prefix restore; reordered/truncated/corrupted/unsafe operations denied; reopened final replay cross-checks full V1 restore. Erasure: actual V1 encoded blobs, six separate simulated domains, replicated pinned metadata, 9,460,962 bytes vs 18,900,003 for three full copies. All 154 bounded recoverable erasure patterns and 140 triple-loss refusals, corrupt metadata fallback, missing/corrupt domain recovery and 12 repaired shards plus exact metadata, third-domain denial without canonical publication, original V1 hash rejection despite self-consistent corrupted parity. Actual reconstruction/repair read/write bytes and timing recorded; all three research tests and strict all-target/all-feature storage Clippy passed.
- Remaining / next action: #35 real game-server capacity evidence. Byte-operation journal is not semantic Minecraft operation capture/write-ahead or a live recovery protocol. Erasure failure domains are directories on one host, not independent machines. Both are isolated research-only, with no production activation; independently prove real semantic replay, audited codec, durable signed metadata, retention, streaming/resource limits, independent placement and WAN fault recovery before considering live formats.
- External dependency: Actual game/semantic write-ahead and installed real-device independent-failure-domain tests; necessary final CI under local-first policy.
- Commit: See 25 November independent journal and erasure research commit.

### #35 — P2 · Unverified
- Work: Publish capacity with separate player, voter, replica, world-size, and relay tests
- Acceptance: Real game-server measurements report player counts, TPS, and resource ceilings. Replica count is never presented as extra simulation capacity
- State: **LOCALLY VERIFIED**
- Source: research/game-capacity/README.md, REPORT.md, evidence/; measure.py, bots.cjs and pinned npm lock; crates/swarm-storage/tests/replica_capacity_lab.rs; crates/swarm-network/tests/game_stream_transport.rs and game_stream_relay.rs; crates/swarm-cli/tests/discovery_freshness.rs and consensus_partition_safety.rs.
- Evidence: Actual isolated Minecraft 26.1.2, Java 25, 1,536 MiB heap, 0/1/2/4/8 moving protocol clients for 16 seconds and separate fresh-world 12/16-client stages for 30 seconds. At the configured 16-slot ceiling: 601 game ticks / 30.065 s = 19.990 TPS, 7.9 ms median sampled MSPT, 1,397.6 MiB peak sampled Java RSS, 443.6 MiB client RSS, 1,965.7 MiB minimum host headroom and 2,214,270 B/s sampled aggregate client RX peak; every client moved and both labs stopped cleanly. Five direct/relay regressions passed (one-MiB request/echo IPv4 QUIC 157.617 ms, direct TCP/Noise 125.558 ms, relayed 317.817 ms; independent circuit survives cap exhaustion). Nine synthetic 1/8/32 MiB x 1/2/3 destination V1 measurements verified reopened stores and exact restores; 32 MiB x three destinations 75,606,843 payload bytes, 2,440.626 ms cumulative verified copy. Separate 3/5/9/17-voter signed freshness benchmark passed, 59.489–120.483 ms median verification, insufficient and replayed proofs refused, seven discovery regressions passed. No replica or voter count added to game simulation capacity.
- Remaining / next action: #36 user-controlled background replica quotas, scheduling and placement. #35 measures a bounded local vanilla baseline, not a production capacity ceiling or installed authenticated Fabric/gateway gameplay. Positive live 3/5-daemon recovery tests explicitly skipped because available host RAM 3,466 MiB was below the real 4,543 MiB admission requirement; no gate weakened. Repeat on adequate hosts; longer diverse/modded workloads, concurrent replication, independent machines and WAN/public relay remain unverified.
- External dependency: Adequate host memory for positive live quorum/recovery, installed real-device/real-network capacity matrix, and necessary final CI under local-first policy.
- Commit: See 25 November local game-server capacity research commit.

### #36 — P2 · Proposal
- Work: Add background replica scheduling, quotas, bandwidth windows, and replica placement controls
- Acceptance: A user can choose what to cache. Hot worlds retain the configured recoverability target under normal churn without overrunning disk or bandwidth
- State: **LOCALLY VERIFIED**
- Source: docs/REPLICA_CACHE_POLICY.md; crates/swarm-storage/src/replica_policy.rs, space.rs; crates/swarm-cli/src/replica_cache.rs, daemon.rs, daemon_parallel_replication.rs, host_readiness.rs, main.rs; crates/swarm-storage/tests/storage_space.rs; crates/swarm-cli/tests/parallel_replication.rs.
- Evidence: Durable device/world follow and hot preference, daily UTC window and aggregate per-world ordinary encoded-payload dispatch budget; fail-closed policy parsing, quota preflight, controlled single-source pull with no unlimited legacy fallback, bounded hot priority and rotating background polling independent of hosting, exact-manifest retry after cache pause. CLI policy/status smoke passed including closed window, invalid UTC refusal, pause/resume and verified unchanged local head. Two live daemon regressions passed: paused receiver and closed window refused offers; zero quota prevented payload, quota increase allowed partial, pause retained exact bytes and old head, receiver restarted and authority disappeared, surviving opted-in seeder completed 983,073 remaining blob bytes in 6.276 s at configured 256 KiB/s with one active pull, exact restore and two surviving verified copies; repeat measured 6.273 s. Existing two-source, one-source fallback and corrupt/disconnected-source restoration passed. 132 storage tests, 168 CLI library and 51 CLI binary tests passed; 4 separately gated/soak tests ignored. Strict all-target/all-feature Clippy and formatting passed.
- Remaining / next action: #37 installed native Windows/macOS GUI QA. Local placement is voluntary on already admitted members; no remote placement solver or new desktop UI. Target/deficit reporting distinguishes exact connected receipts from hints and is observational, not authority or availability proof. WAN and installed multi-device target retention, later-generation background catch-up under real churn, large multi-world workloads, OS-wide shaping and global multi-world disk quotas not established. Certified historical replay retains separate verified admission and is outside ordinary bandwidth windows. No forced eviction or bypass when insufficient opted-in capacity exists.
- External dependency: Independent installed devices/networks and longer multi-world capacity/churn validation; necessary final CI under local-first policy.
- Commit: See 25 November replica cache policy commit.

### #37 — Release-Gate P0 · Unverified
- Work: Implement installed Windows/macOS native GUI QA for #141 using platform-valid automation
- Acceptance: Exercise actual WebView2 (`msedgedriver`) on Windows and native WKWebView automation (Accessibility/XCUITest or isolated QA bridge, not `tauri-driver`) on Apple Silicon & Intel macOS with real sidecars, real IPC, restart, host consent, and an intentional failure in a separate QA build
- State: **EXTERNALLY BLOCKED** (local harness implemented; installed acceptance not established)
- Source: [Native Desktop QA](NATIVE_DESKTOP_QA.md); scripts/acceptance/{native_desktop_qa.py,native_desktop_ax.swift,test_native_desktop_qa.py}; apps/desktop/frontend/tauri.native-qa.{candidate,broken}.conf.json.
- Evidence: 14 new runner contract tests and six existing acceptance/startup tests passed; five-workflow local-first policy check passed. Built and extracted the real Linux shipping Debian package with all four current sidecars, launched the actual WebKitGTK GUI in an isolated authenticated Xvfb display, created a private world, verified its signed checkpoint with the bundled CLI and observed unchanged persisted identity/world after process restart. Full Linux journey did not pass: the legacy sidebar hides Diagnostics even with Tools open; corrected the harness to use selected-world advanced controls and World details. A later native interaction still failed before signed-invite/join completion. No platform acceptance credited.
- Remaining / next action: Finish the native interaction regression with categorized driver evidence, run the complete shipping/candidate/broken-module/missing-sidecar matrix, compile and exercise the AX client on actual Mac hardware, and retain exact-commit signed-join and hosting-consent evidence. Candidate remains opt-in; no shipping cutover. Proceed with #38 parity/rollback preparation without treating #37 as passed.
- External dependency: Interactive native Windows x86_64 with matching WebView2 driver, native Apple Silicon and Intel macOS devices with Accessibility and screen-recording permissions, pinned installers and isolated test LAN. Swift helper is not compiled or runtime-verified on Linux.
- Commit: Native installed QA checkpoint following 26c8c8b; see git history.

### #38 — P1 · Partial
- Work: Decide Svelte cutover after shipping parity review (post-sabathon)
- Acceptance: The candidate covers all legacy journeys, including private invite, host transfer, loopback reconnect, safe stop/wake, mod setup, and recovery. Installed builds and rollback prove the packaging switch
- State: **LOCALLY VERIFIED** (compiled parity and native Linux idle rollback; defer production Svelte cutover)
- Source: apps/desktop/SVELTE_MIGRATION.md; apps/desktop/tests/desktop-browser-webdriver.test.mjs; scripts/acceptance/native_frontend_rollback.py; scripts/acceptance/test_native_frontend_rollback.py; [sanitized native evidence](../research/desktop-rollback/evidence/linux-native-20261125.json).
- Evidence: Compiled Svelte candidate passed five shared real-Chromium scenarios at each of 980x760, 720x560 and 1280x900 with a mocked Rust IPC boundary; legacy browser regression passed at all three sizes. 82 Vitest, 30 source-parity, 23 acceptance-contract, three frontend script and six hosting-consent CLI tests passed; Svelte check, targeted strict Clippy, formatting and workflow policy passed. A packaged native repro proved CLI tracing contaminated strict JSON; production CLI now writes diagnostics to stderr. Real corrupt-runtime-record regression preserves warning, returns valid unavailable JSON, denies hosting and retains the record. Private QA/test directories respect ownership checks under a permissive umask. Corrected same-product Debian packages built at c1e3ade with identical four sidecar hashes. Actual WebKitGTK/Xvfb idle frontend replacement passed all five stages: shipping creation, candidate explicit 512 MiB opt-in, shipping rollback, candidate withdrawal, final shipping rollback. Identity, private world, exact signed checkpoint and preference verified throughout; owned processes exited between stages. Screenshots confirm the capacity gate refused current hosting even after preference opt-in and withdrawal restored player-only. Exact build commit, package and executable hashes and five stage results retained. No Java server or live hosting authority granted.
- Remaining / next action: Keep #37 native Diagnostics visibility and full signed join separate from the completed idle rollback. Full live transfer/reconnect/safe stop/wake, installed recovery, OS updater rollback, and Windows/macOS parity remain gates. Continue #39 without changing production frontendDist. No full release acceptance credited.
- External dependency: Native Windows, Intel and Apple Silicon macOS and live multi-peer gameplay environments.
- Commit: c1e3ade (implementation, compiled parity and native CLI output fix); subsequent evidence-only checkpoint.

### #39 — P1 · Partial
- Work: Simplify first run and import. Add a native folder picker when supported and show source/runtime requirements before commit
- Acceptance: A fresh user can import a supported world, finish explicit EULA, see capacity and mod blockers, and recover an interrupted install
- State: **LOCALLY VERIFIED** (reviewed import, native Linux manual-path flow and picker cancellation; full first-run acceptance pending)
- Source: [Desktop import guide](../apps/desktop/README.md#existing-world-import); apps/desktop/src/import-review.js; apps/desktop/src-tauri/src/import_picker.rs; crates/swarm-cli/src/world_import.rs; crates/swarm-cli/tests/import_preview_cli.rs; [sanitized native evidence](../research/desktop-import/evidence/linux-native-20261125.json).
- Evidence: Shipping and candidate forms share a read-only import review, native folder selection/manual fallback, explicit mod declaration and stale-review invalidation. Rust rechecks the source lock, inventory and compatibility fingerprint before creating identity/staging and publishing a signed world. Destination-inside-source, unsupported tuples, changed artifacts, symlink and special-file cases fail closed. 15 world-import tests, one real sidecar integration, 11 runtime setup hardening/recovery tests, 10 frontend import tests, 82 Vitest and 30 source parity tests passed. Both frontends passed six shared Chromium scenarios at each of three viewports with mocked IPC. Strict targeted Clippy, locked offline checks, formatting, production build and workflow policy passed. Real Linux Debian package opened a GTK native chooser and preserved the path on cancel; automated folder selection timed out. In a separate isolated WebKitGTK/Xvfb run the packaged app successfully reviewed a manually entered synthetic save, displayed Java 25/source inventory, disabled stale review, re-reviewed and published one signed checkpoint verified by its bundled CLI. Source bytes and player-only hosting state were preserved. No actual Minecraft gameplay or EULA acceptance credited.
- Remaining / next action: Verify successful native folder selection on Linux and native Windows/macOS packages. Run a genuine supported Minecraft save through first-run explicit EULA, runtime install/capacity/mod blockers and interrupted-install recovery on clean machines. Keep existing runtime admission guards and production legacy frontend unchanged. Proceed to #40 without treating this scoped evidence as full release acceptance.
- External dependency: Actual supported Minecraft save/runtime artifacts, clean interactive desktop environments and native Windows/Intel and Apple Silicon macOS validation.
- Commit: Reviewed import implementation and local evidence checkpoint; see git history.

### #40 — P1 · Partial
- Work: Deliver compatible game-client preparation and handoff to the chosen launcher. Preserve the user's account and settings
- Acceptance: Matching Java/Fabric/mod set launches or is clearly directed. Existing Mojang/Microsoft sign-in and entitlement remain intact. No credentials travel through invites
- State: **LOCALLY VERIFIED** (manual launcher handoff, signed requirements and exact local client-JAR inspection; live launch/entitlement acceptance pending)
- Source: [Desktop client setup](../apps/desktop/README.md#minecraft-client-setup); crates/swarm-cli/src/client_profile.rs; crates/swarm-cli/tests/client_profile_cli.rs; apps/desktop/src/client-preparation.js; apps/desktop/tests/client-preparation.test.mjs; [native Linux evidence](../research/client-preparation/evidence/linux-native-20261125.json).
- Evidence: Real runtime sidecar exposes an advisory creator-signed, genesis-bound compatibility profile; includes client-only and Both requirements, including universal mods stored only in the server array, rejects contradictory declarations and excludes server-only bridge/datapacks. Explicitly selected top-level mod inventory is bounded and checked for exact ID/version/domain-separated artifact hash, missing, duplicate, invalid, unexpected and server-only JARs. Both frontends provide isolated instance instructions for Minecraft Launcher, Prism Launcher and other Java Edition launchers; no credential/config reads, writes, process launches, downloads, invite changes or route grants. Three Rust unit tests, one real sidecar integration (including changed-byte/version, duplicates, symlink denial, incompatible signed configuration denial, preserved account/settings fixtures and canonical checkpoint), three frontend contract tests, 30 source parity tests, Svelte check/build, and strict targeted Clippy passed. Both browser variants passed seven shared scenarios at three viewports, including stale inventory reply denial and independent verified-reconnect gating. Built and extracted current Linux shipping Debian with real sidecars; native WebKitGTK in isolated Xvfb displayed signed Minecraft/Fabric/Java and mod requirements, detected a missing required mod, confirmed exact inventory, preserved synthetic account/settings/JAR files and player-only hosting state, and verified the canonical checkpoint. No actual game launch or Microsoft/Mojang authentication tested.
- Remaining / next action: Exercise real compatible Minecraft/Fabric/Java client profiles in official and Prism launchers on clean native platforms, verify existing login/entitlement and settings, and connect through fresh authenticated gateway during Session 0/1. Manual setup is deliberate; automatic launcher integration, artifact acquisition and full dependency resolution are not credited. Proceed to #41.
- External dependency: Real licensed Minecraft clients, actual launcher instances/accounts, native Windows and macOS, and real multi-peer gameplay environment.
- Commit: Client preparation and local evidence checkpoint; see git history.

### #41 — P1 · Partial
- Work: Finish modpack dependency acquisition for each supported provider and manual-artifact fallback
- Acceptance: A pristine target acquires allowed artifacts from their authorized source, verifies exact bytes, and shows unavailable or incompatible components. No silent redistribution
- State: **LOCALLY VERIFIED** (server-side authorized acquisition and manual-artifact fallback; live provider and client acquisition acceptance pending)
- Source: crates/swarm-protocol/src/canonical_modpack.rs; crates/swarm-cli/src/provider_runtime.rs; crates/swarm-cli/src/server_mods.rs; crates/swarm-cli/tests/provider_acquisition_cli.rs; apps/desktop/src/launcher-controller.js; apps/desktop/src/mod-acquisition.js; [native Linux evidence](../research/provider-acquisition/evidence/linux-native-20261125.json).
- Evidence: Provider preparation now requires creator-signed genesis-bound compatibility and side-complete required dependencies, rejects overlapping runtime IDs, and rechecks the profile before publication. The explicit Acquire required server mods command returns per-artifact verified/acquired/manual/unavailable/incompatible outcomes without starting or installing the runtime or granting consent. Exact provider project/version, strong provider hashes and signed Fabric identity/version/artifact hash are checked; explicit CurseForge distribution refusal remains manual-only. Unique private staging and atomic no-clobber publication preserve existing files and prevent conflicting concurrent writes. Local fixture acquisition of both providers proved partial progress survives failure, retry skips verified artifacts, restricted and corrupt/substituted downloads never publish, and exact manual supply restores readiness. Both frontends support exact restricted artifact selection before canonical Create and manual supply for existing worlds. 10 canonical modpack, 8 provider-runtime, 11 server-mod, 16 Modrinth fixture, 26 CurseForge, 8 frontend contract, 30 source-parity tests passed; real manual-acquisition and import-sidecar integration, strict targeted Clippy, locked local build, Svelte check/build, both browser variants at all three viewports and five-workflow policy passed. Native extracted Linux shipping package verified manual-required display, exact local JAR supply, idempotent retry, original JAR and signed checkpoint preservation, player-only consent, no runtime installation and process cleanup. Earlier native harness attempts stopped on a read-only fixture setup guard and a collapsed disclosure; the corrected test passed with the same package.
- Remaining / next action: Exercise live authorized Modrinth and CurseForge acquisition on independent pristine targets with real compatible mods, provider credentials where required, and a complete modded gameplay journey. Client artifact acquisition remains manual through the #40 launcher workflow; no automatic client installation or datapack acquisition is credited. Proceed to #42 without claiming real cross-platform release acceptance.
- External dependency: Live provider availability and permitted exact files, machine-local CurseForge credentials, compatible real Minecraft clients/servers and native Windows/macOS environments.
- Commit: Provider acquisition and local acceptance evidence checkpoint; see git history.

### #42 — P2 · Partial
- Work: Add a compatibility matrix for Minecraft versions and Fabric builds. Develop and test an adapter before widening support
- Acceptance: Every offered tuple passes a real server launch, save, restore, and game-client connection. Unsupported versions remain visible with accurate messaging or filtering
- State: **LOCALLY VERIFIED** (exact new-world selection policy and negative compatibility gates; real Fabric lifecycle and authenticated client qualification remain pending)
- Source: crates/swarm-protocol/src/runtime_support.rs; crates/swarm-cli/src/world_import.rs; crates/swarm-cli/src/runtime_main.rs; crates/swarm-cli/tests/runtime_matrix_cli.rs; apps/desktop/src-tauri/src/catalog_commands.rs; apps/desktop/src-tauri/src/canonical_world_commands.rs; apps/desktop/src/runtime-catalog-policy.js; [matrix and field qualification requirements](../research/runtime-compatibility/REPORT.md); [real sidecar matrix output](../research/runtime-compatibility/evidence/matrix-20261125.json).
- Evidence: The exact pinned baseline is Minecraft 26.1.2 / Fabric Loader 0.19.3, with Java 25 as the reference runtime. Rust distinguishes the shipped bridge's declared range from the baseline candidate and unsupported combinations. Both Desktop Create and Import keep other releases visible but disabled, distinguish not-yet-qualified releases from incompatible ones, preserve changed import declarations against delayed catalog responses, and invalidate stale review when a declaration becomes unavailable. Native canonical Create and direct/reviewed import reject unqualified or incompatible tuples before publication. The runtime matrix command is read-only and does not initialize storage. Existing world fingerprints and runtime compatibility rules remain unchanged; the low-level data-only CLI Create remains a protocol/tooling primitive, not a qualification claim. Three protocol tests, 16 import/recovery tests, five real sidecar integration tests, three targeted native Desktop tests, 16 frontend unit tests, 30 source parity tests, strict targeted Rust Clippy, Svelte check/build, both browser variants at 720x560, 980x760 and 1280x900, and the five-workflow policy passed. The first import regression run had one outdated diagnostic expectation; the corrected final suite passed.
- Remaining / next action: Build the exact Fabric candidate and exercise the pinned tuple with the real server, authenticated Fabric readiness, canonical save/stop/restore/restart, and a licensed authenticated client through a verified gateway. Record evidence for each exact tuple before adding another selectable release. The earlier vanilla capacity run (#35) is not proof of Fabric adapter qualification. Proceed to #43 without marking the pinned tuple field-qualified.
- External dependency: Java 25 build/runtime toolchain, Gradle and exact candidate Fabric JAR, authorized live Minecraft server test and licensed authenticated Java Edition client, and native platform field environments.
- Commit: Compatibility matrix and selection-policy checkpoint; see git history.

### #43 — P2 · Proposal
- Work: Improve accessibility and everyday polish across main actions
- Acceptance: Both supported window sizes, keyboard-only flow, screen-reader names/live state, scaling, long names and IDs, and interrupted progress pass on installed builds
- State: **LOCALLY VERIFIED** (scoped UI and packaged Linux checks; full accessibility/platform acceptance pending)
- Source: apps/desktop/src/{accessibility,app,runtime-wizard,launcher-controller,catalog-selectors}.js; apps/desktop/src/{index.html,style.css,player-experience.css}; Svelte App/Dialogs/Topbar/WorldsView; apps/desktop/tests/desktop-browser-webdriver.test.mjs; scripts/acceptance/native_desktop_accessibility.py.
- Evidence: [Accessibility report](../research/desktop-accessibility/REPORT.md) and [real packaged Linux evidence](../research/desktop-accessibility/evidence/linux-native-20261125.json). Shared focus and dialog behavior, accessible names/live-state markup, long-name/ID handling, duplicate/pending-action guards and stale runtime response refusal implemented. 156 frontend Node tests, Svelte unit/source-parity checks, Svelte check/build and five-workflow policy passed. Both real Chromium variants passed shared journeys and interrupted-runtime/invitation tests at three viewports, keyboard-only flows at 980x760 and 720x560, and effective scaled viewports at 576x448 and 360x280. Both extracted current Debian packages with identical four real Rust sidecars passed native WebKitGTK keyboard-only Invite/Stop cancellation and focus restoration at both supported sizes, native dialog semantics/layout, exact long names/IDs and unchanged verified checkpoint/player-only consent. Repeated with retained native runner. Initial browser multi-viewport fixture state leak corrected; screenshot review corrected a shrinking close control before the final build.
- Remaining / next action: OS accessibility API/screen-reader and actual OS scale testing, system-installed Windows/macOS/Linux coverage, broader native main-action and real interrupted-runtime coverage. Keep qualified field acceptance distinct from browser fixtures. Do not mark full acceptance complete.
- External dependency: Native assistive technology and platform field environments, plus qualified real Fabric runtime for live operation interruption.
- Commit: Accessibility and everyday usability checkpoint; see git history.

### #44 — P2 · Research
- Work: Evaluate additional loaders, Bedrock clients, and cross-play separately
- Acceptance: Work only from a proven Fabric baseline. Each new adapter documents worlds, mods, game protocol, lifecycle limits, and cross-platform evidence
- State: **NOT STARTED**
- Source: Pending item-specific source mapping; see original roadmap links.
- Evidence: Historical roadmap baseline only; current item-specific evidence pending.
- Remaining / next action: Deferred by explicit roadmap prerequisite: #42 pinned Fabric tuple is not field-qualified. Do not widen loader/protocol support or advertise cross-play before proving the Fabric baseline. Continue independent backlog items such as #45.
- External dependency: #42 actual Fabric bridge/server/client lifecycle qualification with authorized licensed client and native platform evidence.
- Commit: Pending item-specific attribution

### #45 — P2 · Partial
- Work: Build the normal-player world lobby on signed discovery and provider results
- Acceptance: Public search and exact unlisted resolve have useful empty states, bounded paging, freshness display, and a separate private Join action
- State: **LOCALLY VERIFIED** (real signed loopback directory, both packaged Linux frontends, bounded snapshot; WAN and other native platforms pending)
- Source: crates/swarm-cli/src/{discovery,runtime_main}.rs; crates/swarm-cli/tests/discovery_network_freshness.rs; apps/desktop/src/{discovery-lobby,discovery-diagnostics,launcher-controller,app}.js; apps/desktop/src-tauri/src/launcher_commands.rs; Svelte DiscoveryView/JoinWorldView/Sidebar/Workspace; scripts/e2e/discovery_lobby_smoke.py; scripts/acceptance/native_discovery_lobby.py.
- Evidence: [Report](../research/discovery-lobby/REPORT.md), [real loopback CLI](../research/discovery-lobby/evidence/loopback-cli-20261125.json), [native Linux recheck](../research/discovery-lobby/evidence/linux-native-20261125.json). Rust caps provider collection and overall freshness verification; no relaxed signed membership/quorum proof. Both frontends expose separate Discover and signed-invite Join, six-card pages over a bounded 32-result snapshot, verification time/expiry, provider coverage, explicit incomplete/empty/network errors, and stale-response/duplicate/expired-record rejection. Nine discovery library tests, six real-network tests, targeted Clippy/format/native check, 160 Node tests, 82 Svelte tests, 30 source-parity tests, Svelte check/build and both Chromium variants at three sizes passed. The actual CLI verified nine public worlds, deterministic cap of three, unlisted exact lookup, private refusal and unchanged membership. Both extracted Debian packages passed the native filtered-search recheck with nine signed public listings, two pages, both supported window sizes, exact unlisted lookup and no membership creation. Initial native nine-listing timeout and intermediate eight-listing partial result are recorded; no limit or verification gate weakened.
- Remaining / next action: Remote multi-provider/WAN/NAT reliability and system-installed Windows/macOS field qualification. Discovery V1 intentionally has local bounded paging, not a distributed global index/cursor. Proceed to #46.
- External dependency: Reachable operator-managed providers, actual remote network and platform field environments.
- Commit: Bounded signed discovery lobby and local evidence checkpoint; see git history.

### #46 — P2 · Partial
- Work: Connect friend contact/presence primitives to explicit consent, hide/offline mode, and useful invitations
- Acceptance: Friends see only the metadata a user agreed to share. Removing a contact stops future private presence access. Offline has honest freshness
- State: **LOCALLY VERIFIED** (real consent-gated loopback discovery, native Linux both variants; independent-network/platform qualification pending)
- Source: crates/swarm-cli/src/{discovery,main,runtime_main}.rs; crates/swarm-cli/tests/discovery_network_freshness.rs; apps/desktop/src/friends.js; apps/desktop/src-tauri/src/{friend_commands,runtime}.rs; FriendsView, Sidebar, Workspace; scripts/e2e/friend_presence_smoke.py; scripts/acceptance/native_friends.py.
- Evidence: [Report](../research/friend-presence/REPORT.md), [real signed loopback and locator](../research/friend-presence/evidence/loopback-cli-20261125.json), [final native Linux](../research/friend-presence/evidence/linux-native-20261125.json). Contact-store V2 migration persists hidden and no per-contact grants under the proof lock; acceptance is not consent. Global hide and individual grants gate fresh signed responses/publication; presence-only service refuses world records and exits after last effective grant. Exact authenticated response/nonce verification and durable mutation revision prevent stale contact resurrection; unanswered probes are not-confirmed, positive observations expire within 30 seconds. Both Desktop variants use native settings and explicit removal confirmation; existing signed bearer-invite workflow remains separate from membership. 12 discovery library tests, signed-presence core regression, six existing real-network regressions plus one real delayed-proof remove/re-add test, native validation, strict Clippy, 164 Node tests, 82 Svelte tests, 30 source-parity tests, build/check, and both Chromium variants at three sizes passed. Actual three-identity loopback/locator with mDNS disabled denied ungranted/withdrawn/removed/re-added contacts and world metadata; both final extracted Debian packages passed 12/12 native checks including actual signed probes in both directions, hide, consent/removal persistence, 980x760 and 720x560, and separate invite flow. No membership authority changed.
- Remaining / next action: Field verification with reachable private LAN/VPN locators and independently operated Windows/macOS devices; default presence listen is loopback. Hidden denies new proofs, not transport anonymity; prior hints and delivered proofs expire. Invitations are manually shared bearer tokens, not automatic friend delivery. Continue to #47.
- External dependency: Independent devices, configured reachable private directory/listen interface, and native platform field environments.
- Commit: Consent-gated friend presence and local evidence checkpoint; see git history.

### #47 — P2 · Proposal
- Work: Add public-world governance and abuse handling once private play is stable
- Acceptance: Listing visibility, moderation, region/language metadata, report flow, and local filtering work without granting registry veto power over world history
- State: **LOCALLY VERIFIED** (non-authoritative directory policy and reporting, actual signed loopback and both packaged Linux frontends; public rollout and signed configuration edits remain gated)
- Source: crates/swarm-protocol/src/listing_metadata.rs; crates/swarm-cli/src/{directory_moderation,discovery,main,runtime_main}.rs; apps/desktop/src-tauri/src/{directory_commands,canonical_world_commands,launcher_commands,main}.rs; both Desktop frontends and shared directory-moderation.js; scripts/e2e/directory_moderation_smoke.py; scripts/acceptance/native_directory_moderation.py.
- Evidence: [Report](../research/directory-moderation/REPORT.md), [signed loopback CLI](../research/directory-moderation/evidence/loopback-cli-20261125.json), [native Linux](../research/directory-moderation/evidence/linux-native-20261125.json). Explicit coarse region/language uses existing signed presentation fields, private creation refuses metadata, search filters are signed. Reversible device hide, publisher browse suppression, and local publication pause are distinct and do not alter signed world visibility or authority. Exact fresh public config-bound allegations are stored privately, deduplicated, minimally exported and independently reviewed without automatic submission/adjudication. Malformed policy fails browse closed. Real owner/client/operator scenario passed all checks, preserving 28 canonical protocol files byte-for-byte and creating no membership. Both extracted packages passed 11/11 checks, including native signed search, pause/resume, save/export/delete report and persisted hide/restore at supported sizes. 28 protocol, 5 moderation, 12 discovery library, 7 real-network, 1 native validation, 169 Node, 82 Svelte, 30 source-parity tests, both final Chromium variants, build/check and scoped Clippy passed.
- Remaining / next action: Public directory deployment/intake/appeals, account moderation, general signed visibility edits (configuration-history sync prerequisite), independent LAN/WAN and installed Windows/macOS field qualification remain outside the local checkpoint. New-world metadata only; actual native GUI creation against an external catalog was not tested (real CLI creation and browser GUI payload were tested). Continue to #48.
- External dependency: Independently operated platforms/providers and public-rollout governance/qualification; safe signed configuration-history synchronization before widening signed edits.
- Commit: Non-authoritative directory moderation and report flow; see git history.

### #48 — P2 · Proposal
- Work: Add opt-in, redacted diagnostics export and a useful local history of world actions
- Acceptance: Users can export time, commit, path class, and event summaries while excluding private invites, credentials, raw private addresses, and world data
- State: **LOCALLY VERIFIED** (both Desktop frontends, actual Linux native file save and restart)
- Source: apps/desktop/src-tauri/src/{support_diagnostics,main,runtime,runtime_commands,canonical_world_commands,directory_commands,transfer_commands,import_picker}.rs; apps/desktop/src-tauri/build.rs; shared support-history.js; both Activity/Diagnostics views; scripts/acceptance/native_support_history.py; docs/SUPPORT_DIAGNOSTICS.md.
- Evidence: [Report](../research/support-history/REPORT.md), [native Linux](../research/support-history/evidence/linux-native-20261125.json). Default-off bounded private history, opt-in, typed results and local-only world association; pause/clear invalidates older recording revisions. Explicit allow-listed report projection includes timestamps, source-base commit/tree status, coarse last reported path class and event summaries, excludes raw logs, identifiers, names, tokens, addresses, paths and world data. Exact reviewed bytes saved via native folder selection, no overwrite or private-root destination, Unix owner-only mode. Both extracted packages passed 14/14 native checks including real success/failure, no backfill, scoped redaction, cancellation, exact save, restart, pause/clear and unchanged canonical world data. 71 Rust, 173 Node, 82 Svelte, 30 source-parity tests, both Chromium variants, native check/strict Clippy and workflow policy passed.
- Remaining / next action: Actual Windows/macOS native dialog/filesystem and clipboard qualification remain separate. Selected Desktop command outcomes only, not signed audit or comprehensive daemon trace; coarse last path observation is not live proof. Continue to #49; main-only issue closure remains authorization-gated.
- External dependency: Installed Windows/macOS field qualification; authorized main/release gates as applicable.
- Commit: Opt-in redacted support history and diagnostics export; see git history.

### #49 — Sabathon P0 · Partial
- Work: Reconcile open issues, PRs, and capability claims against main
- Acceptance: Every stale ticket has a landed commit or exact residual; open acceptance blockers remain linked. README, roadmap, and install instructions agree
- State: **LOCALLY VERIFIED** (read-only full reconciliation and documentation; remote ticket closure/merge not authorized)
- Source: [Live issue/PR reconciliation](SABATHON_ISSUE_RECONCILIATION.md); README.md; ROADMAP.md; docs/IMPLEMENTATION_STATUS.md; apps/desktop/README.md; docs/FINAL_PREVIEW_GATE.md.
- Evidence: [Pinned metadata and exact changed-path evidence](../research/issue-reconciliation/evidence/ledger-20261125.json). Main `aaffac22` is the ancestor of audited integration `6fd4e0a` (159 ahead, 0 behind). All 9 open issues and 17 open PRs mapped to main/integration source, historical CI, or specific residual; no remote edits. Both exact-main historical runs completed successfully, Quinn fixed lock versions verified in both trees, and published pinned preview plus legacy release observed (asset metadata only). 12 recovery certificate, 11 retention, one pin-root, one corrupt membership prepare and 112 field-harness tests passed locally. Updated source-vs-release claims, version, runtime/migration/wake/lobby/replication and local-first CI docs. No broad CI or release certification.
- Remaining / next action: Maintainer review/authorized main integration before remote stale closure; original Quinn and Linux baseline issues are closure candidates, six product/release/field issues retain exact blockers. Continue #50 for downloaded bytes, rollback and release-signing evidence. Integration-only work is not yet released.
- External dependency: Authorization to merge/edit issues or PRs and independent field/security/release qualification for their remaining criteria.
- Commit: Reconcile Sabathon issues, PRs and capability claims; see git history.

### #50 — Release-Gate P0 · Partial
- Work: Verify an immutable main snapshot and rollback, then establish production release evidence
- Acceptance: Preview retains previous assets and validates source/version/hash. Tagged release independently verifies Windows signing and Mac notarization; mobile and unsupported packages are clearly scoped out
- State: **LOCALLY VERIFIED** (published preview bytes and isolated native package rollback; production-native signing remains externally blocked)
- Source: `scripts/ci-release/verify_published_preview.py`, `scripts/acceptance/native_published_preview_rollback.py`, `scripts/ci-release/verify_artifacts.py`, Windows/macOS native verification scripts, production release workflow and release gates.
- Evidence: [Audit report and exact downloaded-asset/native evidence](../research/release-verification/REPORT.md). Verified all 21 published assets, both versions' five package checksums and Fabric version, 0.5.1 exact-main source manifest, and actual four-stage Linux package rollback with separate preserved profiles and unchanged signed snapshots/records. New byte-bound signing receipts required by publisher; offline release and workflow policy tests pass. No hosted CI, release or main mutation.
- Remaining / next action: Approved native Windows Authenticode and macOS notarization run on exact production candidate; do not infer live-data downgrade (#52) from package rollback. Continue #51.
- External dependency: Authorized native runners, signing/notarization credentials and release approval; previous release lacks a commit-bound manifest.
- Commit: Verify published previews and bind production signing evidence; see git history.

### #51 — P1 · Partial
- Work: Extend release resilience for download outages, executable provenance, shared runtime cache publication, and dependency health
- Acceptance: Corrupt input fails validation. Same-runtime installs from two worlds preserve the last known-good cache. Every generated or downloaded executable has explicit provenance and verification status
- State: **LOCALLY VERIFIED** (real two-world managed runtime/cache and corruption acceptance; native platform and secondary dependency provenance remain scoped)
- Source: `crates/swarm-cli/src/runtime_installer.rs`, `migration.rs`, `runtime_setup_hardening.rs`, workspace Postcard feature selection and both Rust lockfiles; [runtime installer contract](RUNTIME_INSTALLER.md).
- Evidence: [Resilience report and retained evidence](../research/runtime-resilience/REPORT.md). Stable per-world and shared publication locks, immutable Java/JAR generations, source/verification receipts, Java archive/tree digest, pre-launch runtime guard and fail-closed metadata/config matching. Real local official Java/Minecraft/Fabric/API acquisition, verified published bridge local override, simultaneous second-world denial and successful retry sharing exact runtime generations; no EULA, no gameplay or checkpoint changes. Actual simulated source outage reuses both verified worlds, fails repair without losing cache receipts, detects shared launcher/native Java library corruption and verifies exact restored bytes and canonical snapshots. 300 Rust tests passed, 2 existing ignored; all 11 runtime-hardening cases exercised, strict targeted Clippy, Desktop native compile and 15 bundler outage/integrity fixtures passed. Removed unused Postcard heapless dependency and nine soundness advisories from both graphs; both audits zero vulnerability findings against cached 23 November advisory DB, remaining warnings retained. No hosted CI.
- Remaining / next action: #52 real older-preview profile migration, consent history, downgrade refusal and rollback; native Windows/macOS and live Fabric secondary-dependency provenance, fresh-world metadata outage, safe old-generation cleanup and broader interrupted staging tests remain. Corrupt managed lock requires preserved evidence/review before repair; no silent manual fallback.
- External dependency: Native platform qualification and authorized real client/runtime acceptance, current advisory refresh and upstream dependency maintenance.
- Commit: Preserve shared runtime generations and verify provenance; see git history.

### #52 — P1 · Partial
- Work: Establish upgrade, downgrade, schema, and interrupted migration acceptance from the supported older preview
- Acceptance: One real saved world and consent history survive upgrade and restart. Unsupported downgrade stops clearly, with verified export/rollback guidance
- State: **LOCALLY VERIFIED** (real Java save, actual published Linux package chain and native candidate Desktop restart; historical downgrade preflight is operator-enforced)
- Source: `crates/swarm-storage/src/profile.rs`, storage open/schema guards, CLI profile check and import/Desktop entrypoints; `scripts/acceptance/profile_upgrade.py`, `native_profile_upgrade.py`; [operator procedure](PROFILE_UPGRADE.md).
- Evidence: [Profile upgrade report and exact evidence](../research/profile-upgrade/REPORT.md). Real Java 26.1.2 world (31 files, 2,215,315 bytes, five region files) imported with published 0.5.0, opened with published 0.5.1, explicit opt-in and withdrawal, then upgraded to actual locally packaged candidate. Killed process waiting for activation lock, partial staging refusal and restart; same identity, signed checkpoint, world bytes and latest consent revision. Both historical-target preflights refused initialized data. Verified legacy data-only recovery, denied unsupported certified history, fresh identity and encrypted backup round-trip. Native Linux Desktop boot/restart and future-profile startup/retry refusal passed. Seven profile unit and two CLI tests, import/runtime/backup/storage regressions, strict Clippy, native Desktop build. No CI dispatch.
- Remaining / next action: #53 resource/crash/retention/network campaigns. The historical executables predate the floor and direct invocation is not retroactively intercepted. Native Windows/macOS installation replacement and future incompatible schema migration remain unverified. No legacy authority conversion or actual live downgrade.
- External dependency: Authorized native Windows/macOS release qualification and explicit installation/rollback testing.
- Commit: Guard profile activation and verify real saved-world upgrade; see git history.

### #53 — P1 · Partial
- Work: Extend crash, disk-full, retention, platform resource, and network campaigns beyond current automated gates
- Acceptance: Published stress matrix includes repetitions, actual resource limits, last accepted checkpoint, and explicit failure causes. Saved canonical data remains recoverable
- State: **LOCALLY VERIFIED** (bounded real Linux kernel and process campaigns, not cross-platform or field certification)
- Source: `crates/swarm-storage/src/transaction.rs`, `crates/swarm-cli/tests/{parallel_replication,host_process}.rs`, `scripts/acceptance/{storage_stress,network_stress}.py`, existing delegated cgroup gate.
- Evidence: [Stress matrix, exact limits and checkpoint receipts](../research/stress-campaign/REPORT.md). Three independent real-Java-save profiles on 96 MiB tmpfs: nine actual ENOSPC metadata denials, three kernel 1 MiB file-limit failures, three observed partial-write SIGKILLs and exact old/new restore after retention/retry. Fixed handled atomic metadata staging leaks, no loss of existing records or crash evidence. Two actual netem profiles (15/30 ms delay, 0.25/0.75% configured loss, 30/15 Mbit/s) on isolated loopback; real signed three-device replication with receiver/source SIGKILL and durable resume from surviving seeder; old/new restored, all three signed snapshots retained, actual kernel packet counts and drops. Two 64 MiB group OOM trials, residual descendant fence and actual capped mock-Java host (512 MiB heap, 1536 MiB kernel cap) with verified signed checkpoint #2 restored. No CI dispatch.
- Remaining / next action: #54 signed update delivery/installation hygiene. Native Windows/macOS, physical power loss, persistent filesystem exhaustion at all publication phases, multi-GiB release soak, independent-machine/network and live Minecraft recovery remain unverified by this campaign.
- External dependency: Authorized native/field runners, persistent isolated disk and real game/network environments for broader certification.
- Commit: Exercise bounded real crash, disk, memory and network stress; see git history.

### #54 — P2 · Proposal
- Work: Add signed update delivery and platform installation hygiene after release provenance is stable
- Acceptance: Client verifies manifest and package signature, separates preview/stable channels, supports opt-out, and rejects an incompatible or untrusted update
- State: **LOCALLY VERIFIED** (explicit packaged CLI signed delivery and manual-install preflight; no production activation or automatic installer)
- Source: `crates/swarm-cli/src/updates.rs`, `scripts/ci-release/sign_updates.py`, `scripts/acceptance/signed_updates.py`; [operator contract](SIGNED_UPDATES.md).
- Evidence: [Signed-update report and exact package receipt](../research/signed-updates/REPORT.md). Seven cryptographic verification tests; 15 end-to-end checks against an actual extracted candidate Linux Debian package, ephemeral distinct channel keys, real previously published preview (equal version refused), inert next-version Debian and synthetic other platform artifacts. Signature, byte-count/digest, replay/equivocation, expiry, incompatible schema/profile/target, pin substitution and corrupt/missing package refusal. Atomic private staging, free-space reserve, explicit opt-out and re-verification. Signer rejects altered provenance/bytes and wrongly labelled Debian architecture. Actual 31-file Java save checkpoint, identity, withdrawal revision and all non-update profile bytes unchanged; no hosted CI.
- Remaining / next action: #55 production metrics, support runbooks and compatibility policy. Production trust key/index provisioning, public HTTPS positive delivery, native Windows/macOS signing and OS installer replacement/rollback, key rotation/revocation and optional Desktop update controls remain unverified. No installer execution or production release authorized.
- External dependency: Authorized release operator and offline channel keys, approved publication, native OS qualification.
- Commit: Add explicit signed update delivery and offline verification; see git history.

### #55 — P2 · Missing
- Work: Define production metrics, support runbooks, and per-version compatibility policy
- Acceptance: Field team can diagnose boot, replication, lease, game connect, restore, and publication with bounded and redacted evidence
- State: **LOCALLY VERIFIED** (opt-in CLI operational observations, bounded redacted export, field runbooks and compatibility policy; no production telemetry or live-field certification)
- Source: `crates/swarm-cli/src/support_metrics.rs`, build provenance and actual CLI/daemon/host/runtime operation hooks; `crates/swarm-cli/tests/parallel_replication.rs`; `scripts/acceptance/operational_support.py`; [operator contract](OPERATIONAL_SUPPORT.md).
- Evidence: [Operational support report and exact receipts](../research/operational-support/REPORT.md). Six redaction/retention/revision unit tests; real local authenticated daemon test with controlled pull (1,048,618 bytes / 2,953 ms), ordinary push finalization, quorum permit loss/refresh, real TCP gateway refusal, offline denial, exact restore/failed restore, signed checkpoint publication and local discovery refresh/pause. Actual candidate Debian package and four matching sidecars: six packaged checks, 31 genuine saved files restored byte-exact, unchanged non-support profile bytes, identity, checkpoint and withdrawn consent. 271 passing library/binary/selected regression tests; two existing cgroup-only tests ignored; strict Clippy and local gates. No hosted CI.
- Remaining / next action: #56 reopen distributed region/tick feasibility decision. Live Fabric and authenticated client metric capture, field support exercise, native Windows/macOS report acceptance and mixed-version multi-network qualification remain unverified. No fleet collector or SLO claim.
- External dependency: Authorized compatible runtime/player and field devices, native platform qualification and actual support operations.
- Commit: Add bounded opt-in operational support and version policy; see git history.

### #56 — Expansion · Design prerequisite
- Work: Reopen the distributed region/tick feasibility decision for the expanded product scope (revisiting the current Fabric NO-GO). Choose the experimental runtime before implementation
- Acceptance: Compare the existing single-host baseline with actual multi-host simulation. Document cross-region correctness, compatibility limits, and measured performance before adopting the runtime
- State: **LOCALLY VERIFIED** (research-only negative adoption decision; actual isolated virtual-node execution, not physical multi-host Minecraft acceptance)
- Source: [Predeclared experimental runtime](../research/distributed-simulation/REGION_LAB_DESIGN.md); `research/distributed-simulation/{region_runtime,network_lab,test_region_runtime,verify_lab_evidence}.py`.
- Evidence: [Networked feasibility report and raw receipts](../research/distributed-simulation/REGION_LAB_REPORT.md). 17 new and 10 prior tests passed; 72 networked measurements / 2,304 accepted ticks and eight serial references / 256 ticks, all per-tick roots and final full states identical. Separate worker processes on distinct cores and separate private veth/TCP network namespaces; actual kernel netem. Local spread 8,192/16,384 agents four vs one worker p95 3.03 vs 8.19 ms / 6.41 vs 13.35 ms. Clustered little benefit; boundary-heavy actual traffic and coordination cost. Measured RTT medians 0.094 / 10.272 / 50.406 ms; service misses 0 / 156 / 768 of 768 at each profile. Real SIGKILL after prepare and veth partition after seal preserved prior accepted root and froze further ticks/export. Surviving worker tentatively advanced locally during partition, proving durable atomic recovery still missing. Original #35 actual single-host vanilla baseline referenced separately, never compared as equivalent agents/players.
- Remaining / next action: #57 independent explicit CPU/memory contribution preferences; do not activate regional work. Selected custom reference engine is research-only, not a Minecraft implementation; synchronous barrier unsuitable for consumer WAN. Existing Fabric NO-GO retained. #58--63 production adoption requires a reviewed real runtime/ruleset, durable certified region grants and cross-boundary transactions, actual licensed Minecraft clients and independent physical worker/control domains with quantitative sustained gameplay qualification. No field acceptance credited.
- External dependency: Physical independent worker/control machines, compatible Minecraft-facing runtime and authenticated player journeys; real network/consensus/recovery qualification.
- Commit: Reopen region feasibility with isolated networked simulation; see git history.

### #57 — Expansion · Missing
- Work: Add per-device CPU and memory contribution, with desired limits, measured capacity, and a player-only choice
- Acceptance: Joining grants no compute access automatically. A player can select 4 GiB and a CPU limit, change or withdraw it, and see requested, admitted, and active resources. Zero preserves play where available
- State: **LOCALLY VERIFIED** (explicit per-device/per-world consent, actual delegated Linux kernel enforcement and native Desktop controls; no distributed runtime or field gameplay certification)
- Source: [Resource contract](DEVICE_RESOURCE_CONTRIBUTION.md); CLI hosting budget and managed cgroup admission, runtime ownership/live proof checks; native IPC and both Desktop frontends; `scripts/acceptance/native_resource_contribution.py`.
- Evidence: [Report and receipts](../research/device-resources/REPORT.md). 259 Rust library/binary tests, six public CLI integration tests, one real three-identity authenticated join/replication/recovery test with no implicit consent, 13 native boundary tests, 173 JS and 84 Svelte tests, 30 source parity and 18 harness tests. Explicit delegated kernel test measured 1,496,600 CPU microseconds over 3,000,159 elapsed at 500 millicores; 30 throttled periods, exact process/descendant and revision binding, prepared/active distinction, tamper rejection and withdrawal/exit. Both final native Debian packages 8/8 each: 4096 MiB + CPU preference, denied admission without delegation, revision change, zero withdrawal/restart, unchanged identity/checkpoint and no runtime activation. Both browser variants passed three viewport gates.
- Remaining / next action: #58 admission/placement research only; no distributed work grant. Actual Minecraft/Fabric with CPU cap, zero-contribution authenticated gameplay, Windows/macOS CPU enforcement and independent-device field acceptance remain unverified. Legacy heap-only CLI compatibility retained; an explicit limit cannot fall back to uncapped execution. Prior delegated memory-only cases not rerun in this milestone.
- External dependency: Authorized CPU+memory delegation on Linux; tested native enforcement on other OSes; real game clients and independent machines, compatible regional runtime and #56 adoption gates.
- Commit: Add explicit device CPU and memory contribution controls; see git history.

### #58 — Expansion · Missing
- Work: Build opt-in worker admission and placement based on usable memory, CPU, network path, and region load
- Acceptance: A newly admitted machine takes real simulation work only when useful. Inadequate capacity scales down safely. Platform limits and heap versus process memory are visible
- State: **LOCALLY VERIFIED** (research-only capped reference-runtime placement, not production worker admission or independent-host Minecraft acceptance)
- Source: [Worker placement design](../research/distributed-simulation/WORKER_PLACEMENT_DESIGN.md); `worker_placement.py`, `placement_lab.py`, `test_worker_placement.py`, `verify_placement_evidence.py`; isolated research runtime and network harness.
- Evidence: [Report and raw receipts](../research/distributed-simulation/WORKER_PLACEMENT_REPORT.md). Two real delegated Linux experiments with distinct private worker namespaces and actual kernel CPU/memory limits. 208 measured ticks match independent serial roots; selected second worker executed real region compute, identical-cut p95 service time 13.577→9.479 ms and 13.259→8.821 ms. End-to-end checked-loop speedup not demonstrated. Rejected low memory, low CPU, ~51 ms measured RTT and unhelpful clustered-load worker; zero never spawned. Verified stop-all scale-in with smaller cap, same tick/root continuity; three no-fit cases preserved prior cut. Deliberate leaf CPU policy change froze each experiment and refused new generation. 36 research tests; independent receipt replay, 12 altered-evidence negatives and six real static-network regression measurements plus kill/partition tests passed.
- Remaining / next action: #59 region-aware runtime/ruleset and real player interaction feasibility; production adoption stays gated. No production #57 preference-to-worker enrollment, live migration, independent-host placement or Minecraft compatibility claim. Estimator uses reference-specific heuristics; checked-loop overhead and reference coordinator trust limit the result.
- External dependency: Reviewed compatible region runtime, authenticated Minecraft clients, separately enrolled physical worker/control domains, durable certified authority and transactions (#60–62), quantitative field acceptance.
- Commit: Add resource-aware capped reference worker placement; see git history.

### #59 — Expansion · Missing
- Work: Choose or build a region-aware runtime. Keep one world identity as different machines simulate different regions
- Acceptance: Players traverse a continuous world while independent machines tick different regions. The supported Minecraft/protocol/mod matrix is documented and verified
- State: **LOCALLY VERIFIED** (continuous custom reference player world only; no Minecraft compatibility or independent physical-host acceptance)
- Source: [Player runtime and compatibility report](../research/distributed-simulation/PLAYER_RUNTIME_REPORT.md); `region_runtime.py`, `player_runtime.py`, `player_lab.py`, `test_player_runtime.py`, `verify_player_evidence.py`.
- Evidence: Two paced private TCP experiments, each with two real region-worker processes in separate network namespaces, two active scripted reference client processes and one identity-spoof negative client. 296 accepted ticks, 582 client-observed views including joins, 12 player transitions across workers, 16 across regions; every tick and final full state match the serial reference and independently projected views. Tick-loop p95 4.569/4.224 ms; zero 50 ms schedule misses, small 256-agent fixture. Actual SIGKILL after prepare and veth partition after seal froze tick 1 and refused any later view/input/join/tick; no failed-tick client publication, scoped processes exited. 44 research tests passed, including independent receipt replay and 15 altered-evidence refusals.
- Remaining / next action: #60 exclusive region writers/generation fencing and global-state ownership; retain the production NO-GO. No actual Minecraft client, native mods, durable authority/recovery, separate physical devices, WAN qualification, or sustained scale proof. The lab protocol and reference ruleset are not Minecraft-compatible.
- External dependency: Reviewed compatible game runtime, licensed authenticated game clients, independent enrolled worker/control hosts, certified region grants and cross-region transactions (#60–62), sustained comparative gameplay acceptance (#63).
- Commit: Add continuous reference player world and live fault verification; see git history.

### #60 — Expansion · Missing
- Work: Bind every region to a single authorized writer and generation. Define the ownership of global game state
- Acceptance: Region reassignment, overlap, and worker restart never produce two canonical writers for the same region. World-wide saves include a consistent accepted state
- State: **LOCALLY VERIFIED** (single trusted durable local reference authority; no distributed quorum, Minecraft or independent-host acceptance)
- Source: [Regional authority report](../research/distributed-simulation/REGIONAL_AUTHORITY_REPORT.md); `regional_authority.py`, `authority_lab.py`, `verify_authority_evidence.py`, `test_regional_authority.py`; opt-in research worker mode in `network_lab.py`.
- Evidence: Two live private-network runs with four concurrent region workers during reassignment, three generations per run, 18 accepted ticks and 34 full save observations matching the independent serial reference. Two rotation and two tick-publication process races each accept exactly one contender. Sixteen explicit overlap/stale/replay/restart refusals, four actual authority-writer SIGKILL injections before/after SQLite COMMIT, durable one-time generation claims including before tick one, old worker receipts rejected while old workers live, fresh worker restart from the exact accepted cut. All 58 research tests passed (14 new), 18 altered evidence cases refused; focused eight-tick prior network regression and kill/partition checks passed.
- Remaining / next action: #61 durable client-controlled player/entity/inventory transitions and replay across regions and restart. The ledger is a single local serialization point with a trusted controller, not quorum-certified regional grants, Byzantine protection, full-database rollback detection, independent physical domains or real Minecraft authority. Production NO-GO retained.
- External dependency: Reviewed compatible Minecraft runtime and actual game clients; durable independently certified regional grants and recovery protocol; field hosts and sustained gameplay evidence.
- Commit: Add durable fenced reference regional authority and verify process races; see git history.

### #61 — Expansion · Missing
- Work: Make player, entity, and inventory transitions durable across regions, including block and global interactions in the supported ruleset
- Acceptance: Players walk or use a supported portal across workers and preserve identity, state, and inventory through restart. Supported cross-region mechanics pass replay and correctness tests
- State: **LOCALLY VERIFIED** (custom transaction reference ruleset and single trusted local ledger; no native Minecraft, quorum or independent-host acceptance)
- Source: [Durable transitions report](../research/distributed-simulation/DURABLE_TRANSITIONS_REPORT.md); `transition_runtime.py`, `transition_lab.py`, `verify_transition_evidence.py`, `test_transition_runtime.py`; isolated research extensions to `region_runtime.py` and `regional_authority.py`.
- Evidence: Two real private TCP runs, 28 client sessions, 16 unique committed player commands and cross-worker player transitions, eight durable ticks across six generation instances, four actual authority-writer SIGKILL faults before/after COMMIT. Sealed inputs survive a pre-commit crash; after a post-commit lost acknowledgment the original receipts return without applying movement or inventory twice. Both runs preserve the exact agent set, world identity and resource total, independently verified full cuts and client views; cross-player and conflicting replays refused. 71 research tests passed (13 new), 16 altered evidence cases refused. Prior eight-tick static-network regression and kill/partition cases passed.
- Remaining / next action: #62 contribution withdrawal, worker loss, scale-in and partition recovery. Reference cell-bank actions and signals are not actual Minecraft blocks, portals, inventories or modded/global mechanics. The single trusted ledger, full serial validation, synthetic client identity and local process fault model do not prove production scale, quorum, rollback resistance, independent-host or actual game acceptance. Production regional NO-GO retained.
- External dependency: Real compatible Minecraft runtime, authenticated clients, independent enrolled physical worker/control hosts, durable quorum-backed publication/recovery and sustained comparative gameplay.
- Commit: Add durable reference player transactions and crash-safe command replay; see git history.

### #62 — Expansion · Missing
- Work: Handle scale-in, loss, and partition. Drain and reassign live work when volunteers leave
- Acceptance: A graceful exit drains eligible work and releases the device budget. A crash freezes or fails over affected regions from accepted checkpoints without duplicating state
- State: **LOCALLY VERIFIED** (capped custom reference runtime with stop-all generation recovery, one trusted local authority; no native Minecraft, independent-host or live partial-region acceptance)
- Source: [Worker recovery report](../research/distributed-simulation/WORKER_RECOVERY_REPORT.md); `recovery_runtime.py`, `recovery_lab.py`, `verify_recovery_evidence.py`, `test_recovery_runtime.py`; isolated research extensions to `placement_lab.py`, `regional_authority.py`, `transition_runtime.py`.
- Evidence: Two retained real private-network/cgroup-v2 experiments, 16 worker process instances, 36 TCP client sessions, 12 accepted durable ticks, 24 unique committed commands, 12 fully released scopes. Explicit withdrawal refuses a complete old-generation candidate while old workers remain alive; clean scale-in restores the sealed input on one capped worker. Real worker SIGKILL after prepare and real veth partition after seal (two of each) preserve accepted cuts, disconnect uncommitted client replies and recover only after fresh admission. Insufficient measured fit, all-zero contribution and unavailable workers pause without advancing the cut; unavailable slots remain blocked across reopen until revised consent. 80 research tests passed (nine new); 20 altered evidence cases refused; both final roots match independent serial replay.
- Remaining / next action: #63 sustained comparative reference-runtime scale envelope, including clustered/degraded cases. This is stop-all, trusted local recovery; not seamless live regional handoff, production resource consent, actual Minecraft, independent remote cleanup or quorum-backed failure recovery. Production regional NO-GO retained.
- External dependency: Reviewed compatible Minecraft runtime, actual clients and independent enrolled worker/control hosts, durable quorum-backed authority/recovery and sustained gameplay qualification.
- Commit: Add durable capped worker withdrawal and checkpoint recovery; see git history.

### #63 — Expansion · Unverified
- Work: Run real distributed gameplay and publish the scale envelope
- Acceptance: Compare the same world, load, and network with one worker and multiple independent workers. Show increased sustainable active regions or player capacity at the published service target. Publish clustered-load and degraded-network limits
- State: **IN PROGRESS** (local full durable reference scale envelope measured; correctness verified, service target failed; actual Minecraft and independent-host acceptance open)
- Source: [Scale envelope report](../research/distributed-simulation/SCALE_ENVELOPE_REPORT.md); `scale_lab.py`, `verify_scale_evidence.py`, `test_scale_evidence.py`; disposable scope allowance in `placement_lab.py`.
- Evidence: 24 real capped private-network runs, six workload/delay combinations, two repetitions each for one and two workers, eight-second 20 Hz windows (192 seconds total). 36 worker and 48 persistent TCP client process instances, 1,538 accepted durable ticks and 3,076 client commands independently replayed, all scopes released. 1,514 completions within 3,840 scheduled slots; zero runs and zero original slots meet the strict service target. At 8,192 spread agents, two workers improve mean throughput from 3.375 to 3.875 ticks/s (+14.8%) but remain below target; clustered and degraded-link results do not establish capacity gains. 86 research tests passed (six new), malformed/incomplete/altered evidence refused.
- Profile/cache follow-up: [Full durable service profile](../research/distributed-simulation/SCALE_PROFILE_REPORT.md). Exact persisted BLOB-keyed immutable state cache retains current transaction, root, grant, fencing, receipt, serial replay and synchronous durability checks. New 24-run same-contract matrix independently replayed 1,883 accepted ticks and 3,766 client receipts; 1,859 completions within 3,840 slots, 60 original slots on time, zero qualified runs. Heavy spread mean TPS 3.375 to 7.500 (one worker) and 3.875 to 7.625 (two), best single rate 17.75; delayed-link results regressed. Raw exclusive phase evidence identifies SQLite write-transaction COMMIT as leading local low-load cost (36.986 ms of 54.114 ms mean); heavy receipt validation/replay and sequential WAN exchange remain costly. Historical comparison, not a newly measured instrumented uncached control; outer scope wrapper reported exit 1 despite completed and independently verified receipt and released worker scopes. All 96 research tests passed, including cache mutation/rollback, phase accounting and retained schema-2 replay. Workflow policy and release-version checks passed.
- Batching follow-up: [Atomic input batching and committed receipts](../research/distributed-simulation/SCALE_BATCH_REPORT.md). Bounded distinct authenticated commands queue/seal atomically under current generation and contribution checks; read-only exact receipt retrieval checks accepted ancestry and sealed input. Real before/after-seal SIGKILL, restart, rollback, retired-generation and consent withdrawal regressions passed. Full 24-run schema-3 matrix verified 2,069 durable ticks and 4,138 client receipts; every tick had two write/two read transactions, four commits and no rollback (previous path six write/one read). 2,046 completions in 3,840 slots, 226 original slots on time, zero qualified runs. Peak single rate 20 TPS (150/160 deadlines), low-load one-worker mean 19.062 vs 17.250; heavy spread/clustered regressions, delayed-link improvements. Wrapper exit 0 and all scopes released. All 109 research tests, workflow policy and release-version checks passed. Historical cached comparison only, with differing host load and trace overhead; instrumented controls remain open.
- Remaining / next action: Profile/remove redundant full candidate parsing and serialization while retaining all global invariants and independent replay. Measure trace overhead and controlled individual/batched comparisons. Qualify sustained service on independent enrolled hosts with actual compatible game clients and increased active regions/player load. Production regional NO-GO retained.
- External dependency: Actual compatible Minecraft runtime and clients, independently enrolled physical worker/control hosts, durable quorum-backed regional authority/recovery, long-duration comparative gameplay at the published service target.
- Commit: Measure full durable reference scale envelope and retain negative qualification evidence; see git history.

### #64 — Expansion · Missing
- Work: Add a creation-time **Ownerless / no OP** world option with a signed governance policy
- Acceptance: The founding player receives no permanent creator privileges, player OP, unilateral rule changes, or unilateral control over recovery. Turning on more compute grants no governance privilege
- State: **LOCALLY VERIFIED** (isolated offline creation certificate and refusal of live activation; actual game permissions and shared canonical authority remain open)
- Source: [Ownerless creation RFC](rfcs/ownerless-creation.md); `crates/swarm-protocol/src/ownerless.rs`, `crates/swarm-consensus/src/ownerless.rs`, `crates/swarm-cli/src/ownerless_creation.rs`; real CLI and cryptographic contract tests.
- Evidence: Distinct ownerless genesis namespace, no creator or initial host key, exact immutable no-OP/solo/console/RCON/compute-vote policy, 3–15 canonical prime-order validator keys, unanimous strict Ed25519 genesis approvals pinned to exact identity and policy, proposed stable-roster majority threshold only. Seven new contract tests plus two real CLI tests; 122 protocol/consensus tests and two CLI integration tests passed. Separate temporary profiles sign; organizer and outsider get no automatic vote; duplicate, partial, forged, wrong-world and weak/mixed-torsion key cases refused. Offline commands never open canonical storage; normal world create --ownerless refuses rather than creating a creator-owned V1 world. Ordinary V1 creation regression passed.
- Remaining / next action: #65 actual managed runtime and worker permission enforcement and binding to accepted policy; #66 durable canonical quorum, validator transitions and founder-independent recovery; #67 distinct public/private admission. This is a certificate, not an active world or proof of actual no-OP Minecraft gameplay. The initial state and compatibility pins are not verified runtime/save artifacts. No automatic recovery or maintenance authority is granted. Production ownerless and distributed hosting remain gated.
- External dependency: Independent threat-model review, compatible managed game/worker runtime, durable shared authority and distinct enrolled failure domains, actual founder/successor-host and joint field acceptance.
- Commit: Add isolated signed ownerless genesis and offline creation workflow; see git history.

### #65 — Expansion · Missing
- Work: Bind enforced permission rules to the canonical runtime and each regional worker, with explicit CFT vs. BFT trust boundaries
- Acceptance: On unmodified runtimes, the founder and successor hosts have ordinary in-game permissions, and `/op`, RCON, and privileged management interfaces are disabled. Documentation explicitly separates software-enforced governance from OS-level host tampering
- State: **LOCALLY VERIFIED** (real Fabric permission lab and policy-bound reference workers; canonical activation and actual player admission remain gated)
- Source: [Ownerless permissions RFC](rfcs/ownerless-permissions.md); `minecraft/fabric/src/main/java/dev/swarmcraft/fabric/OwnerlessPermissionGuard.java`, required mixins, `research/distributed-simulation/ownerless_permission.py`, `scripts/e2e/ownerless_permission_smoke.py`.
- Evidence: Strict signed creation verifier issues non-activatable world/policy/artifact compatibility pin. Actual Minecraft 26.1.2/Fabric 0.19.3 JVMs pass 54 restricted permission probes (including restored save) and three legacy control probes; five unsafe startups fail before listener. Actual game, RCON and management sockets tested. Four real regional worker processes in two generations, 34 invalid/privileged requests refused, two accepted reference ticks match serial replay. Signed public evidence and artifact hashes retained; independent checker and nine evidence mutations tested. 88 reference tests, 208 CLI library tests (3 ignored), two real CLI tests, strict targeted Clippy, local Gradle build and workflow policy pass. Initial broad Rust run failed 13 existing private-directory/busy-store tests under umask 0002 and concurrent JVM load; documented umask 077 serial rerun passed without changing checks.
- Remaining / next action: #66 durable shared canonical authority, validator membership and founder-independent recovery; #67 admission and worker eligibility. No actual authenticated player, canonical host transfer, production worker admission or independent-machine gameplay verified. Pin passed by trusted launcher, not remote attestation; CFT only. Lab artifact fingerprint and reference state root are not production Minecraft save proofs. Production ownerless and distributed gameplay remain disabled.
- External dependency: Actual independent machines, account-authenticated player acceptance, canonical authority and reviewed runtime compatibility; #63 throughput qualification.
- Commit: Bind isolated ownerless policy to real Fabric permissions and reference workers; see git history.

### #66 — Expansion · Missing
- Work: Define shared authority, validator admission, and loss recovery for ownerless worlds
- Acceptance: A verified majority can keep the world available after the founder leaves permanently. No single key can publish a unilateral policy change or rewrite accepted state. A lost majority leaves verified recovery copies
- State: **LOCALLY VERIFIED** (bounded CFT reference history and independent local validator journals; live canonical Minecraft authority remains gated)
- Source: [Ownerless authority RFC](rfcs/ownerless-authority.md); `crates/swarm-protocol/src/ownerless_authority.rs`, `crates/swarm-consensus/src/ownerless_authority.rs`, `crates/swarm-cli/src/ownerless_authority.rs` and actual CLI regression suite.
- Evidence: Exact parent, policy, compatibility, sequence, fence and checkpoint pins; strict distinct Ed25519 majority; joint old/new majority plus every entrant's approval. Owner-only Linux journal, stable lock, atomic synced signed vote persisted before output; same-head witness changes cannot unlock, certified extension can. Four actual CLI integration scenarios using separate temporary profiles: founder offline, four certified transitions including recovery and replacement, removed/outsider refusal, minority copy without authority, concurrent conflicts, lost stdout, invalid/missing/linked journal and corrupted checkpoint refusal, same-head witness and competing certified extension. 127 protocol/consensus tests (five new authority tests), four new and two prior real CLI integration tests, one actual SIGKILL-after-durable-vote regression, strict targeted Clippy and workflow policy passed. No V1 world, runtime or control records created.
- Network follow-up: [Signed private TCP observation RFC](rfcs/ownerless-network-observation.md): exact head/nonce/key and checkpoint checks, pending-vote and stale-head refusal, bounded parallel collection, no authority grants. 133 protocol/consensus tests, 13 CLI integration scenarios (including retained signatures), strict Clippy. Real distinct local network namespaces: founder absent, two-validator observation, actual link-down refusal, new nonce after link restoration, unchanged journals. One physical host; not a lease or live decision transport.
- Encrypted decision follow-up: [Network decision RFC](rfcs/ownerless-network-decision.md): one-shot Noise NN channel with mutually verified transcript-bound application Ed25519 identities and exact world/head/proposal pins; reviewed remote proposal submission uses the existing fsynced durable vote lock. 14 authority and 2 creation CLI integration tests passed, including 3 new real TCP tests. Full 1 MiB checkpoint with tampered ciphertext refusal, dropped vote and exact retry after process restart, wrong identities, retired listener, and joint roster entrant approval verified. Strict targeted Clippy, formatting, workflow policy and release version passed. Existing quorum and no-runtime boundaries preserved. Offline certificate assembly/acceptance unchanged.
- Certified collection/sync follow-up: [Network decision RFC](rfcs/ownerless-network-decision.md) adds explicit bounded parallel vote collection, independently verified joint/current quorum, durable local acceptance, separate authenticated encrypted full-history/checkpoint sync, and signed exact world/base/head/sequence/fence/checkpoint receipts. 16 authority and 2 creation CLI tests passed (2 new real TCP scenarios); unreachable majority, conflicting retry, silent optional voter, duplicate targets, corrupted encrypted checkpoint, lost receipt, exact same-target retry preserving later pending vote, joint entrant approval, retired/outsider denial, newly admitted validator voting, two-decision suffix reconciliation and stale target refusal. Initial two test assertions used display strings rather than serialized protocol hashes; corrected typed assertions passed without altering verification. Reference-only; no canonical gameplay records or runtime activation.
- Collector-loss recovery follow-up: [Network decision RFC](rfcs/ownerless-network-decision.md) adds an independent authenticated recovery domain, bounded evidence retrieval, reconstruction solely from existing durable votes or a surviving complete certificate, and verified exact checkpoint acceptance. 19 authority and 2 creation CLI tests passed (3 new real TCP scenarios). Founder/collector no longer contacted, surviving votes rebuild the certificate, unavailable and wrong-identity peers leave locks unchanged, complete surviving certificate resolves a conflicting pending vote without new voting, old/new quorum enforcement and entrant consent remain intact, exact retry and recovered checkpoint sync pass. No canonical gameplay records or runtime activation.
- Remaining / next action: Automatic discovery and background recovery/dissemination across independently controlled validators, audited live canonical runtime authority and current consent/admission. Explicit recovery requires a reviewed next proposal, an existing verified base and a requester in both base and target rosters. The explicit sync sender must belong to both pinned base and target rosters; recipient must belong to the target and have an existing base/target journal. Reference journal capped at 128 records and 1 MiB checkpoint; no automatic view change, fresh quorum lease, real Minecraft save, or host selection. Loss of majority or split pending votes can freeze; no timeout reset. Production ownerless and distributed gameplay remain disabled.
- External dependency: Independent physical failure domains, audited durable enrollment/rollback assumptions, canonical gameplay/worker integration, #63 throughput and #68 combined field acceptance. CFT only; local key/profile cloning and full disk rollback are outside assumption.
- Commit: Add bounded ownerless quorum history and durable validator journals; see git history.

### #67 — Expansion · Missing
- Work: Separate world governance, public/private participation, and volunteer worker trust
- Acceptance: Ownerless can be invite-only or public. Joining a public world grants neither a vote nor write authority. Anonymous game access and eligibility to run or certify workers are separate policies
- State: **LOCALLY VERIFIED** (signed public/private reference policy and independent reference role eligibility; actual Minecraft authentication, admission and runtime activation remain gated)
- Source: [Ownerless participation RFC](rfcs/ownerless-participation.md); `crates/swarm-protocol/src/ownerless_participation.rs`, `crates/swarm-consensus/src/ownerless_participation.rs`, `crates/swarm-cli/src/ownerless_participation.rs`, #66 signed history/journals.
- Evidence: Explicit public format 2 distinct signing domains/identity; original private policy hash unchanged and existing #65 evidence reverified. Current majority independently certifies/revokes private invitations and capped worker eligibility; no implicit player, worker or validator role. Signed verifier challenge binds exact current head, subject, verifier, role, nonce and budget; explicit separate volunteer consent. Durable one-shot local consumption under world lock, concurrent/replayed/canceled/replaced/tampered/stale proofs and lost stdout tested. 131 protocol/consensus tests (four new); seven authority CLI and two creation CLI integration tests (three new); all role reports deny live quorum, runtime, actual admission and write authority.
- Remaining / next action: #68 combined qualification; live Minecraft authenticated account mapping, actual public/private routing, live quorum transport and current consent/resource/runtime integration remain gated. Public reference eligibility requires application key proof, not anonymous offline Minecraft login. One-shot offer is not an ongoing worker grant; no actual capacity or enforcement inferred. Public format explicitly refused by existing private Fabric lab.
- External dependency: Real independently owned machines, authenticated accounts, supported distributed Minecraft runtime, live authority/worker integration and #63 sustained service target.
- Commit: Separate public/private ownerless reference roles and durable challenge consumption; see git history.

### #68 — Expansion · Unverified
- Work: Certify ownerless and horizontally scaled operation together
- Acceptance: In one real game session, the founder exits permanently, other peers keep the same world available with no player OP, opt-in worker resources change, and players continue across reassigned regions when the required quorum and capacity survive
- State: **IN PROGRESS — combined local reference qualification passed; real game/independent-host acceptance open**
- Source: [Combined qualification report](../research/distributed-simulation/COMBINED_OWNERLESS_REPORT.md); `combined_ownerless_lab.py`, independent evidence verifier, and retained signed history in the research evidence directory.
- Evidence: Two isolated combined runs using actual Rust signed governance journals, separate TCP clients and policy-pinned Linux cgroup-capped regional worker processes. Retained final run: 13 certified decisions, three replayed checkpoints/ticks, four workers (1→2→1), four client receipts and nine role-proof observations. Founder journal untouched after departure; surviving majority recovers, player crosses different workers, revoked worker cannot volunteer, reduced CPU/memory enforced, retired authenticated receipts refused by session, minority recovery publication denied and exact recovery copy preserved. Client acknowledgements follow matching checkpoint certification. All creation/history/checkpoint commitments independently verified with actual CLI; deterministic replay and observation mutation tests. All 90 distributed-reference tests, release-version and workflow-policy checks passed.
- Remaining / next action: Independent live quorum transport and canonical publication, resource consent/admission integration, real authenticated account mapping and region-aware Minecraft runtime. Repeat actual combined gameplay across independently controlled machines and verify sustained service at the required target; #63 did not qualify.
- External dependency: Real independent hosts and accounts, supported distributed Minecraft runtime and live authority, #63 service target. One trusted local controller and reference simulation; no production activation or real game acceptance.
- Commit: Combined ownerless reference qualification and retained verification; see git history.

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

Latest actionable continuation after #68 combined local reference qualification, #66 explicit encrypted collection, synchronization and collector-loss recovery, and #63 full durable phase profiling/cache/atomic batching: optimize redundant full candidate validation/serialization and measure controlled comparisons; the strict service target still fails. Continue reviewed canonical authority/runtime integration and persistent authenticated recovery/dissemination, plus remaining original private-world acceptance. No remaining local reference qualification is asserted to substitute for these gates. Ownerless gameplay stays disabled until the canonical authority, admission and field gates are met. The #63 full durable service target, independent quorum, physical, compatibility and actual Minecraft gameplay gates remain open.
