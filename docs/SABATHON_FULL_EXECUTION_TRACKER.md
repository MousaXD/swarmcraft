# Sabathon full execution tracker

Baseline: `2d45f97b5e31118a7074361e141a90b9296eccbd` (2026-11-25).

All 68 original backlog IDs retained. Acceptance text is copied from the two roadmap documents. Historical partial work is not counted as complete. NOT STARTED means not yet started in this execution ledger, not necessarily absent from source. IN PROGRESS includes existing partial work. LOCALLY VERIFIED here is scoped to implemented automated components, not full acceptance. CI VERIFIED requires the current source and complete item-specific evidence; FIELD VERIFIED requires field evidence. No item is marked fully accepted at initialization.

Current CI: [45519415707](https://github.com/MousaXD/swarmcraft/actions/runs/45519415707) completed successfully: 20 selected jobs passed, optional soak skipped, exact source commit `2d45f97`. Previous `a7f2326` run 45515921964 passed 20 selected jobs on retry. See [live integration status](SABATHON_P0_LIVE_INTEGRATION_STATUS.md) for scope and residuals.

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
- State: **NOT STARTED**
- Source: Pending item-specific source mapping; see original roadmap links.
- Evidence: Historical roadmap baseline only; current item-specific evidence pending.
- Remaining / next action: Inspect current source against acceptance; implement missing dependencies and test.
- External dependency: Not yet established; assess during execution.
- Commit: Pending item-specific attribution

### #9 — P1 · Proposal (recommended first)
- Work: Provide a supported headless zero-Java third-replica profile (`swarmcraft daemon`) for small private worlds
- Acceptance: A small third device (Raspberry Pi, NAS, or low-cost VPS) runs and restarts without Java, acknowledges current saves, and votes under the existing quorum rules — solving both crash failover and async offline handoff with zero new consensus machinery
- State: **NOT STARTED**
- Source: Pending item-specific source mapping; see original roadmap links.
- Evidence: Historical roadmap baseline only; current item-specific evidence pending.
- Remaining / next action: Inspect current source against acceptance; implement missing dependencies and test.
- External dependency: Not yet established; assess during execution.
- Commit: Pending item-specific attribution

### #10 — P1 · Partial
- Work: Make invite expiry, reuse, membership approval, pending state, and revocation legible. Review a recipient-bound or one-use invite option
- Acceptance: An expired or revoked grant cannot admit a new device. A removed device cannot fetch newer private snapshots. Existing downloaded copies have clearly documented retention limits
- State: **IN PROGRESS**
- Source: Pending item-specific source mapping; see original roadmap links.
- Evidence: Historical roadmap baseline only; current item-specific evidence pending.
- Remaining / next action: Inspect current source against acceptance; implement missing dependencies and test.
- External dependency: Not yet established; assess during execution.
- Commit: Pending item-specific attribution

### #11 — P1 · Partial
- Work: Expose a clear recovery path when a sole valid copy survives but quorum is gone. Reuse export/import and add a labeled recovered-world workflow
- Acceptance: The original bytes are preserved. A new recovered world uses a new identity unless a valid original authority transition exists. UI clearly identifies the old and new worlds
- State: **IN PROGRESS**
- Source: Pending item-specific source mapping; see original roadmap links.
- Evidence: Historical roadmap baseline only; current item-specific evidence pending.
- Remaining / next action: Inspect current source against acceptance; implement missing dependencies and test.
- External dependency: Not yet established; assess during execution.
- Commit: Pending item-specific attribution

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
- State: **IN PROGRESS**
- Source: Pending item-specific source mapping; see original roadmap links.
- Evidence: Historical roadmap baseline only; current item-specific evidence pending.
- Remaining / next action: Inspect current source against acceptance; implement missing dependencies and test.
- External dependency: Not yet established; assess during execution.
- Commit: Pending item-specific attribution

### #15 — P1 · Unverified
- Work: Certify representative LAN, WAN, home NAT, CGNAT, IPv6, mobile, blocked UDP, and packet-loss scenarios
- Acceptance: Matrix reports network class, selected control and game routes, success/failure, and reason. Any unsupported path is named accurately
- State: **NOT STARTED**
- Source: Pending item-specific source mapping; see original roadmap links.
- Evidence: Historical roadmap baseline only; current item-specific evidence pending.
- Remaining / next action: Inspect current source against acceptance; implement missing dependencies and test.
- External dependency: Not yet established; assess during execution.
- Commit: Pending item-specific attribution

### #16 — P1 · Partial
- Work: Publish deployable discovery bootstrap, private rendezvous, and game relay configurations with documented boundaries
- Acceptance: A new group can bring up its own compatible node, swap relays, and recover from relay outage. Relay and directory services never grant world authority
- State: **IN PROGRESS**
- Source: Pending item-specific source mapping; see original roadmap links.
- Evidence: Historical roadmap baseline only; current item-specific evidence pending.
- Remaining / next action: Inspect current source against acceptance; implement missing dependencies and test.
- External dependency: Not yet established; assess during execution.
- Commit: Pending item-specific attribution

### #17 — P1 · Proposal
- Work: Define relay connection, bandwidth, duration, auth, and resource policies before shared hosting
- Acceptance: Fair-use and disconnect behavior are measured with multiple worlds. Invites and private route details are absent from public logs
- State: **NOT STARTED**
- Source: Pending item-specific source mapping; see original roadmap links.
- Evidence: Historical roadmap baseline only; current item-specific evidence pending.
- Remaining / next action: Inspect current source against acceptance; implement missing dependencies and test.
- External dependency: Not yet established; assess during execution.
- Commit: Pending item-specific attribution

### #18 — P2 · Proposal
- Work: Add adaptive replication throttling so background saves do not ruin game latency
- Acceptance: On a limited uplink, measured game responsiveness stays within a published target while the replica continues to make bounded progress
- State: **NOT STARTED**
- Source: Pending item-specific source mapping; see original roadmap links.
- Evidence: Historical roadmap baseline only; current item-specific evidence pending.
- Remaining / next action: Inspect current source against acceptance; implement missing dependencies and test.
- External dependency: Not yet established; assess during execution.
- Commit: Pending item-specific attribution

### #19 — Sabathon P0 · Partial
- Work: Prove per-world player-only default, opt-in persistence, and withdrawal through CLI, native IPC, and live hosting; make `authority.permit` updates atomic (temp + rename or loopback IPC)
- Acceptance: Zero-budget peer can join and replicate with no JVM. Consent stays scoped to one world and persists across restart. Every launch path rejects missing or withdrawn consent, and concurrent permit reads on Windows/POSIX never see a truncated permit file
- State: **LOCALLY VERIFIED**
- Source: crates/swarm-cli/src/hosting_budget.rs; authority_permit.rs
- Evidence: Live integration status; scoped automated evidence, exact-SHA CI pending.
- Remaining / next action: Verify live withdrawal and exact-SHA CI
- External dependency: Not yet established; assess during execution.
- Commit: 2d45f97 baseline; see live status for individual commits

### #20 — Sabathon P0 · Partial
- Work: Clarify withdrawal lifecycle and handle a hung JVM without silently losing the current save
- Acceptance: UI shows **Stopping at checkpoint** until proof of checkpoint/fence/exit, or a truthful error with recovery choices. Stale readiness and active gateway routes are cleared immediately
- State: **LOCALLY VERIFIED**
- Source: crates/swarm-cli/src/migration.rs
- Evidence: Live integration status; scoped automated evidence, exact-SHA CI pending.
- Remaining / next action: Verify hung JVM shutdown and real withdrawal journey
- External dependency: Not yet established; assess during execution.
- Commit: 2d45f97 baseline; see live status for individual commits

### #21 — P1 · Partial
- Work: Improve process identity and coordinate a safe resource budget across data roots where supported
- Acceptance: Local concurrency/orphan tests cover process identity, restart, and no second host when capacity or ownership is ambiguous
- State: **IN PROGRESS**
- Source: Pending item-specific source mapping; see original roadmap links.
- Evidence: Historical roadmap baseline only; current item-specific evidence pending.
- Remaining / next action: Inspect current source against acceptance; implement missing dependencies and test.
- External dependency: Not yet established; assess during execution.
- Commit: Pending item-specific attribution

### #22 — P1 · Missing
- Work: Add whole-process limits only on platforms that can enforce them. Evaluate delegated Linux cgroups, Windows Job Objects, and macOS capability separately
- Acceptance: Measurement includes native memory and children. Unsupported enforcement is labeled heap-only. Existing Java heap and memory headroom rules stay in force
- State: **NOT STARTED**
- Source: Pending item-specific source mapping; see original roadmap links.
- Evidence: Historical roadmap baseline only; current item-specific evidence pending.
- Remaining / next action: Inspect current source against acceptance; implement missing dependencies and test.
- External dependency: Not yet established; assess during execution.
- Commit: Pending item-specific attribution

### #23 — P1 · Partial
- Work: Reconcile V2 transcript, signed consent, voter proof, and admission work from #122/#125. Plan durable migration from V1 separately
- Acceptance: Exact source history, signed approvals, stable and joint voter majorities, replay bounds, no legacy stale writes, interrupted migration, and rollback behavior are reviewed and tested
- State: **IN PROGRESS**
- Source: Pending item-specific source mapping; see original roadmap links.
- Evidence: Historical roadmap baseline only; current item-specific evidence pending.
- Remaining / next action: Inspect current source against acceptance; implement missing dependencies and test.
- External dependency: Not yet established; assess during execution.
- Commit: Pending item-specific attribution

### #24 — P1 · Partial
- Work: Split member, player, replica, voter, administrator, and host-candidate permissions in UI and policy
- Acceptance: A non-hosting replica can safely vote. A player or viewer can’t become host from UI preference alone. Membership changes preserve durable quorum rules
- State: **IN PROGRESS**
- Source: Pending item-specific source mapping; see original roadmap links.
- Evidence: Historical roadmap baseline only; current item-specific evidence pending.
- Remaining / next action: Inspect current source against acceptance; implement missing dependencies and test.
- External dependency: Not yet established; assess during execution.
- Commit: Pending item-specific attribution

### #25 — P1 · Proposal
- Work: Add supported invite/device lifecycle, including contact removal, replacement devices, and recovery of a lost identity
- Acceptance: Losing a PC has a documented repair path. A removed device cannot regain access through stale presence or an obsolete invite. Private recovery material is only exported with explicit user action
- State: **NOT STARTED**
- Source: Pending item-specific source mapping; see original roadmap links.
- Evidence: Historical roadmap baseline only; current item-specific evidence pending.
- Remaining / next action: Inspect current source against acceptance; implement missing dependencies and test.
- External dependency: Not yet established; assess during execution.
- Commit: Pending item-specific attribution

### #26 — Sabathon P0 (baseline) / P1 (full identity mapping) · Missing
- Work: Manage deterministic `server.properties` at Fabric launch (`server-ip=127.0.0.1`, per-world loopback port, explicit `online-mode`, `enable-rcon=false`, baseline allowlist); in P1 connect private SwarmCraft membership to full Minecraft login/session allowlist policy
- Acceptance: **P0:** Launched servers never bind `0.0.0.0:25565`, never collide across worlds on one PC, and enforce explicit `online-mode` so Mojang UUIDs/inventories survive host transfer. **P1:** Swarm membership and Minecraft game identity stay consistently enforced so unauthorized game sessions are rejected
- State: **IN PROGRESS**
- Source: minecraft/fabric/; crates/swarm-cli/
- Evidence: Live integration status; scoped automated evidence, exact-SHA CI pending.
- Remaining / next action: Verify baseline runtime policy; implement P1 identity mapping separately
- External dependency: Not yet established; assess during execution.
- Commit: 2d45f97 baseline; see live status for individual commits

### #27 — P2 · Proposal
- Work: Define public/unlisted/private visibility behavior and moderation roles before public rollout
- Acceptance: Visibility never bypasses membership. Admin actions and role changes can be reviewed, and private data does not leak to discovery
- State: **NOT STARTED**
- Source: Pending item-specific source mapping; see original roadmap links.
- Evidence: Historical roadmap baseline only; current item-specific evidence pending.
- Remaining / next action: Inspect current source against acceptance; implement missing dependencies and test.
- External dependency: Not yet established; assess during execution.
- Commit: Pending item-specific attribution

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
- State: **IN PROGRESS**
- Source: Pending item-specific source mapping; see original roadmap links.
- Evidence: Historical roadmap baseline only; current item-specific evidence pending.
- Remaining / next action: Inspect current source against acceptance; implement missing dependencies and test.
- External dependency: Not yet established; assess during execution.
- Commit: Pending item-specific attribution

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

Continue #4: planned and crash-recovered authority re-entry, exact certified voting fences, and durable crash-retirement replay implemented. Targeted successor proof offers and certified retargeting after membership changes are implemented. Pending join/leave and solo conflicts remain conservatively blocked; independent private WAN reachability remains unverified. Continue #9 and the remaining P0 acceptance. Keep publication and runtime fail-closed. #9 precedes #8; #23 precedes activation of V2-dependent permissions; #56 precedes distributed-runtime adoption; #64–68 require signed governance and independent acceptance.

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
