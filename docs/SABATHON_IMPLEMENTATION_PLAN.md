# SwarmCraft engineering audit and implementation plan

Engineering sabathon brief · 23 November 2026, UAE time  
Audited baseline · SwarmCraft 0.5.1 technical preview on `main`  
Commit · `aaffac22e32547983f1fd558af6eb32c0da4917b`  
Git tree · `2ba139e6e1032fbd49e8aa0df207856a2cba68af`  
Merged integration · [PR #115](https://github.com/MousaXD/swarmcraft/pull/115)

## Start here

**Build the private-friends experience first.** The first useful milestone is a repeatable demo in which Alice creates or imports a private world, Bob joins, the app shows which save Bob has verified, Alice hands over hosting, and both reconnect to the correct world. With a third voting replica online, Alice can then turn her PC off while Bob continues. The two-PC experience with no third peer needs its own hosting policy and is the next product milestone.

SwarmCraft already ships the hard parts of a technical preview. It can persist identity, invite private members, build signed snapshots, replicate and resume world data over authenticated P2P, install a compatible Minecraft server, and start a safe successor once quorum and runtime conditions are met. The missing link is turning **“the new server is ready”** into **“the player has safely rejoined it.”** Windows/macOS installed GUI proof and real three-network Minecraft gameplay are also missing. [Main source](https://github.com/MousaXD/swarmcraft/tree/aaffac22e32547983f1fd558af6eb32c0da4917b) · [Implementation status](https://github.com/MousaXD/swarmcraft/blob/main/docs/IMPLEMENTATION_STATUS.md) · [Client continuity](https://github.com/MousaXD/swarmcraft/blob/main/docs/CLIENT_CONTINUITY.md)

| Priority | Outcome | Stop condition |
| --- | --- | --- |
| P0 | A private three-peer world supports a real join, controlled handoff, crash recovery, and verified manual reconnect | A reviewer can follow one recording and evidence bundle from checkpoint to actual gameplay |
| P0 | A friend can understand whether it is safe to stop the host | Desktop distinguishes save present, save verified, host authorized, server ready, and game reachable |
| P0 | Platform and release coverage is trustworthy | Installed Linux, Windows, Apple Silicon macOS, and supported Intel macOS have scoped evidence, and release artifacts are tied to their commit |
| P1 | Two friends can deliberately take turns with only one PC awake | A new opt-in policy proves exclusive write ownership through a clean handoff and restart |
| P1 | Private play tolerates ordinary WAN and compatibility problems | Game relay, useful diagnostics, resumable downloads, and a verified runtime on both PCs |
| P2 | The platform supports larger groups and more worlds | Measured player and replica capacity, disk/bandwidth controls, and optional discovery and backup |
| Expansion | A single world scales across resources volunteered by multiple peers | Real machines simulate different regions of the same world and show a measured capacity gain |
| Expansion | A world can be created with no permanent owner and no player OP | The founder has the same game permissions as other players and can leave permanently while the world continues |
| Research | Compare new editions and loader adapters | Separate prototypes with explicit compatibility and performance results |

### How to use this plan

Start with the [open issue ledger](#open-issue-ledger), then the [implementation backlog](#implementation-backlog) and [sabathon schedule](#sabathon-schedule). Priorities describe when to work on an item. “Built,” “partial,” “missing,” “unverified,” and “proposal” describe the evidence. A merged fix can still need issue closure or independent field testing. Items without a GitHub issue number are proposed work, and this audit has not filed or closed tickets.

## What the merged tree actually supports

| Area | Evidence on the audited tree | Missing outcome |
| --- | --- | --- |
| World lifecycle | Stable world IDs, create/import, signed invite/join, local data and persisted device identity | First-time setup across two remote real clients, role/revocation polish, and better share UX |
| Snapshots | Signed manifests and hashes, verified immutable blobs, resumable transfer, source fallback, replica acknowledgments and storage safety checks | Useful replication progress, per-peer retention and bandwidth controls, parallel source scheduling, and measured save-loss window |
| Authority | Signed leases, fencing, durable recovery ballots, stale-host rejection, and process-level recovery tests | Full real-client failover and a two-device exclusive-turn model |
| Planned transfer | Signed prepare/accept/commit/activate/observe stages and a real Desktop wizard | Automated exchange and a complete reconnect journey. An unchanged two-voter world still requires two votes after transfer |
| Sleep and wake | Durable sleep and a daemon path for wake with a surviving exact-replica quorum | Good desktop wake orchestration after everyone shuts down, and real end-to-end evidence |
| Voluntary hosting | Per-world local opt-in and heap budget, zero-budget player-only state, Tauri bridge, and rechecks on sensitive runtime paths | Whole-process resource caps, guaranteed withdrawal timing, and V2 world-policy activation |
| Player connection | Server runtime and safety status are available. The UI avoids treating a local endpoint as a verified successor route | Authenticated game transport and deliberate reconnect, followed by optional client automation |
| Network | Authenticated libp2p, discovery, QUIC, direct dialing, NAT traversal support, and relays for SwarmCraft traffic | Verified Minecraft game relay and representative home-network field results |
| Desktop | Shipping legacy HTML/JS, plus a staged Svelte candidate with its own native Linux CI | Final Svelte cutover and interactive acceptance on Windows/macOS |
| Compatibility | Managed server runtime, EULA, version checks, provider and modpack building blocks, exact hash checks | Polished compatible game-client setup and repeatable modded cross-PC journeys |
| Discovery | Signed announcements, configured-provider search/exact resolve, and friend presence/contact primitives | Full public/friends lobby, consent and moderation, dependable discovery defaults, and join conversion |
| Release | Required CI, artifact checks, immutable preview workflow, and production signing policy | Post-merge release evidence, published preview verification, production credential and signing proof, and upgrade/rollback on installed machines |

### Findings that affect sprint scope

1. **The README, roadmap, and implementation status can disagree.** Some passages describe multi-member wake, host-transfer UI, managed runtime, or discovery as absent. Other passages and the shipped source show working subsets. The audit traced these to the daemon wake path, Tauri transfer wizard, managed runtime, and discovery commands. Rewrite the capability matrix from current code and scoped tests so developers avoid rebuilding features. [README](https://github.com/MousaXD/swarmcraft/blob/main/README.md) · [Roadmap](https://github.com/MousaXD/swarmcraft/blob/main/ROADMAP.md) · [Implementation status](https://github.com/MousaXD/swarmcraft/blob/main/docs/IMPLEMENTATION_STATUS.md)
2. **A transfer changes the host and preserves the membership rules.** The launch path observes a live permit for multi-member worlds. If the world still has two voters and Alice turns off her device, Bob cannot rely on that planned transfer to run canonically alone. The two-PC rotation you want needs an explicit new authority policy. [Migration runtime](https://github.com/MousaXD/swarmcraft/blob/main/crates/swarm-cli/src/migration.rs) · [Daemon lease handling](https://github.com/MousaXD/swarmcraft/blob/main/crates/swarm-cli/src/daemon.rs)
3. **A healthy SwarmCraft link does not prove the game route.** Current discovery proves useful world and peer information. Local `game_endpoint` and a persisted Ready report do not authenticate where a Minecraft client should connect after migration. The game connection needs its own route proof and actual network path. [Client route RFC](https://github.com/MousaXD/swarmcraft/blob/main/docs/rfcs/minecraft-client-route.md)
4. **A public or unlisted listing does not change admission.** Creation still establishes invite-only membership in the current V1 path. Expand listing and membership in separate steps. A visible card can say how to request or enter an invite without silently enrolling a peer. [World creation](https://github.com/MousaXD/swarmcraft/blob/main/crates/swarm-cli/src/main.rs) · [Discovery](https://github.com/MousaXD/swarmcraft/blob/main/crates/swarm-cli/src/discovery.rs)
5. **The current Fabric support range is narrower than the product vision.** The shipped adapter checks Minecraft `~26.1.2`, Fabric Loader at least `0.19.3`, and Java major at least `25` when Java is checked. Provider catalog results are broader than this adapter. Treat additional releases and loaders as explicit compatibility work. [Runtime support contract](https://github.com/MousaXD/swarmcraft/blob/main/crates/swarm-protocol/src/runtime_support.rs)
6. **The hosting number in the UI is a Java heap budget.** `-Xmx` is real and local memory admission is conservative. It is not the whole JVM's process memory or a guarantee across multiple data roots. An unresponsive server can delay save-and-stop on withdrawal. Both limits affect what #84 can claim. [Hosting budget contract](https://github.com/MousaXD/swarmcraft/blob/main/crates/swarm-cli/HOSTING_BUDGET_CONTRACT.md)
7. **One surviving disk copy is a data-recovery statement.** A valid snapshot lets an operator recover the world bytes. Continuing writes under the original shared-world identity also needs a valid authority transition. The UI and website should state both separately. [Authority recovery](https://github.com/MousaXD/swarmcraft/blob/main/docs/AUTHORITY_RECOVERY.md)
8. **The field harness is useful but manual.** The GitHub workflow runs negative tests and can run a provisioned-host preflight. The positive field result needs installed packages, real clients, actual gameplay, reviewed evidence, and the certified three-device run. A green harness badge does not meet that condition. [Field run](https://github.com/MousaXD/swarmcraft/actions/runs/45022501088) · [Runbook](https://github.com/MousaXD/swarmcraft/tree/main/scripts/e2e/three_machine)
9. **Runtime provenance differs by component.** Managed Java requires an upstream SHA-256. The generated Fabric launcher JAR path has no independently expected digest and records a local hash after acquisition. Make that trust distinction explicit, require expected publisher integrity where supported, and decide a documented trust policy for generated launchers before production rollout. A local hash is useful for detecting later changes. It does not authenticate first acquisition. [Runtime installer](https://github.com/MousaXD/swarmcraft/blob/main/crates/swarm-cli/src/runtime_installer.rs)
10. **Concurrent runtime setup needs a shared-cache check.** The install lock is scoped to the world while managed Java is shared by Java major and platform. Cover two simultaneous installs or repairs for different worlds in an isolated test. If the shared cache can be modified concurrently, serialize publication by runtime tuple and keep a usable last-known-good copy. This is a source-review finding with a test still needed. [Runtime installer](https://github.com/MousaXD/swarmcraft/blob/main/crates/swarm-cli/src/runtime_installer.rs)

## Open issue ledger

Nine GitHub issues were open when checked. The table gives the closure or completion criterion at this baseline.

| Issue | State at audit | Work before closure |
| --- | --- | --- |
| [#84 Voluntary hosting](https://github.com/MousaXD/swarmcraft/issues/84) | Partial. Local opt-in, local admission, zero budget and desktop/CLI are present. V2 activation remains unwired | Exercise direct launch, recovery, transfer and wake on real devices. Verify revocation while hosting and after restart. Agree whether a whole-process cap is required for this scope. Keep local consent and distributed policy distinct |
| [#85 Three real installations](https://github.com/MousaXD/swarmcraft/issues/85) | Open product blocker. Process recovery and managed real server launch are separately proven | Implement verified game routing. Run actual independent Minecraft clients, crash/transfer, restart, marker and inventory preservation, consent denial, stale host fencing, and network evidence |
| [#97 Transient artifact downloads](https://github.com/MousaXD/swarmcraft/issues/97) | Managed Java uses an upstream checksum, pinned AppImage/NSIS acquisition uses SHA-256, and 15 local CI-artifact tests passed | Finish the original outage acceptance matrix, prove verified-cache recovery on native lanes, and record which third-party download failures still block a run. Track generated runtime launcher provenance separately |
| [#120 Field withdrawal and player-only negatives](https://github.com/MousaXD/swarmcraft/issues/120) | Verifier enhancements are merged. Real-world evidence is still missing | Observe withdrawal, checkpoint/fence/stop, denied re-host, and zero-budget player joins on the real field configuration. Record who had a live JVM and who did not |
| [#135 Quinn dependency advisories](https://github.com/MousaXD/swarmcraft/issues/135) | Both merged locks use `quinn-proto 0.11.19` and `quinn-udp 0.5.16`. Normal main root and desktop audit jobs passed | Attach the version and job evidence, then close or consolidate after maintainer review |
| [#137 Quinn audit duplicate](https://github.com/MousaXD/swarmcraft/issues/137) | Overlaps #135 | Link to the canonical issue and close as duplicate after the evidence is recorded |
| [#138 Linux WebKit and managed Java coverage](https://github.com/MousaXD/swarmcraft/issues/138) | Fixes merged and candidate Linux installed-GUI and live managed-Java jobs passed | Attach exact main-run evidence that the real package launches and the fresh flow installed managed Java before closing |
| [#140 Safe preview publication](https://github.com/MousaXD/swarmcraft/issues/140) | Immutable staging and five-group verification merged. PR rehearsal passed | Inspect a real main preview, verify provenance and all assets, confirm prior preview remains usable and exercise download/rollback |
| [#141 Installed Windows/macOS GUI](https://github.com/MousaXD/swarmcraft/issues/141) | Native package builds passed. Installed WebView2 and WKWebView user journeys remain unverified | Drive real installed builds with real IPC, first boot, create/join, restart, consent and missing-sidecar negative. Cover Win plus supported Mac architectures and prove test hooks stay out of shipping artifacts |

### Open PRs and branch cleanup

GitHub returned 17 open PRs. The table reflects exact head ancestry and comparison of changed files at the time of the audit. A changed blob can include a superseding fix or a distinct gap. It warrants review before closure. Preserve any unique work.

| Open PRs | Observed relationship | Sprint disposition |
| --- | --- | --- |
| [#110](https://github.com/MousaXD/swarmcraft/pull/110), [#142](https://github.com/MousaXD/swarmcraft/pull/142) | Exact heads are ancestors of the audited main | Add the merge reference and close the stale records after checking any extra acceptance criteria |
| [#92](https://github.com/MousaXD/swarmcraft/pull/92), [#99](https://github.com/MousaXD/swarmcraft/pull/99), [#101](https://github.com/MousaXD/swarmcraft/pull/101), [#114](https://github.com/MousaXD/swarmcraft/pull/114) | Exact head is outside main ancestry. All files changed against their merge bases match main byte for byte | Mark content as landed and preserve approvals/limitations. Avoid a redundant merge |
| [#111](https://github.com/MousaXD/swarmcraft/pull/111) | Eleven of twelve changed files match; CI workflow has diverged | Reconcile the distinct workflow delta and existing V2 tests. Treat the foundation as inert |
| [#93](https://github.com/MousaXD/swarmcraft/pull/93), [#100](https://github.com/MousaXD/swarmcraft/pull/100) | Main has later overlapping hosting CLI/runtime/Tauri code. Some head blobs differ | Compare use cases and tests. Carry forward only protections or contracts missing from latest source |
| [#96](https://github.com/MousaXD/swarmcraft/pull/96) | Six of fourteen changed files match; main has an evolved field harness | Map unique supported checks into the current harness and preserve the remaining real execution work |
| [#77](https://github.com/MousaXD/swarmcraft/pull/77), [#119](https://github.com/MousaXD/swarmcraft/pull/119), [#124](https://github.com/MousaXD/swarmcraft/pull/124), [#129](https://github.com/MousaXD/swarmcraft/pull/129) | These safety and regression heads carry source or test differences | Compare current regression coverage and exact invariants. Keep only missing tests or fixes, with a current-source result |
| [#122](https://github.com/MousaXD/swarmcraft/pull/122), [#125](https://github.com/MousaXD/swarmcraft/pull/125) | Non-ancestor V2 signed quorum implementations. Their own changed files are not identical to main | Reconcile overlapping cryptographic contracts in one reviewed plan. Do not activate V2 during the private-world demo |
| [#91](https://github.com/MousaXD/swarmcraft/pull/91) | Independent review ledger with distinct history | Preserve the historical veto and evidence. Record a new exact-source verdict when field proof is ready |

### CI evidence on the merge tree

- The parent PR head was `9a3165c`. Git proves that it and `aaffac22` have the same tree. The PR Required Validation completed with 20 successful jobs and its optional network-soak job skipped. The installer workflow completed with 27 successful jobs, including the soak and the package/checksum verifier. Its PR-only publication skip is expected. [Required Validation](https://github.com/MousaXD/swarmcraft/actions/runs/45022501035) · [Installer verification](https://github.com/MousaXD/swarmcraft/actions/runs/45022501050)
- The installed Linux Svelte candidate test passed, and Windows, Apple Silicon macOS, and Intel macOS candidate builds passed. These jobs cover native interaction on Linux and construction on Windows/Mac. [Native Linux candidate](https://github.com/MousaXD/swarmcraft/actions/runs/45022495991) · [Windows/macOS candidate packages](https://github.com/MousaXD/swarmcraft/actions/runs/45022496027)
- The merged commit triggered its own push workflows. At inspection they were still in progress, with both RustSec audit jobs and the live managed-Java Minecraft job already green. The release list showed the older `main-latest` preview. Verify the final run and actual assets before referencing a new downloadable preview. [Main Required Validation](https://github.com/MousaXD/swarmcraft/actions/runs/45080577660) · [Main Installers](https://github.com/MousaXD/swarmcraft/actions/runs/45080577651) · [Releases](https://github.com/MousaXD/swarmcraft/releases)
- From a fresh archive of the exact merged tree, this audit reran 100 field-harness tests, 16 release publication/artifact tests, 15 CI-artifact acquisition tests, release metadata checks, and the workflow policy check. They passed. The desktop Node suite passed 110 tests and failed one runner prerequisite because ChromeDriver was absent locally. Rust was unavailable in this audit shell, so Rust evidence comes from the linked GitHub runs and earlier exact-tree checks. No new real WAN, platform or production signing evidence was generated.

## Implementation backlog

Items 1–56 retain the original audit backlog. Items 57–68 specify the requested horizontal simulation and ownerless world modes. Each row has a completion check. The original private-world sabathon demo remains the first milestone. The full expanded plan requires both new modes.

### Private play and host handoff

| # | Priority and state | Work | Acceptance |
| --- | --- | --- | --- |
| 1 | P0 · Partial | Make one private flow connect Create/Import, Invite, Join, Verify save, and Play in the shipping UI. Reuse the Rust membership and installer paths | Two fresh devices join one world by invite, without terminal steps or manual world-folder copying |
| 2 | P0 · Missing | Define and implement an authenticated route for the currently authorized Minecraft host | Resolver proves exact world/generation, fresh authority, membership, and runtime readiness before returning a route. Expired, retired, conflicting, and wrong-world routes are rejected |
| 3 | P0 · Missing | Add a verified **Join current host** and **Reconnect** action. Separate server ready from client reachable | After a planned transfer, a real client rejoins only the verified successor and sees the saved marker. Errors distinguish route, game reachability, permission, and compatibility |
| 4 | P0 · Partial | Automate the signed staged transfer exchange behind the existing wizard. Reuse final save and fencing | Both sides can resume once after app restart without duplicate transfer. The successor sees a durable checkpoint. The source releases hosting and returns as a replica |
| 5 | P0 · Partial | Surface Host Readiness, replica freshness, and shutdown outcome in one player flow | Stale replica, unreachable voter, missing mods, zero budget, and active safe successor produce distinct messages. Green requires fresh backend proof |
| 6 | P0 · Partial | Make sleep/wake reachable through ordinary UI and verify the supported quorum path with real Minecraft | All peers stop safely. Original host stays off. A supported surviving majority wakes the exact saved world with one active authority |
| 7 | P1 · Missing | Build a fixed local connection path and optional Fabric client auto-reconnect | A disconnect closes the old game stream. With explicit opt-in, a fresh Minecraft connection starts only after a new trusted route is available. Retry is bounded and cancelable |
| 8 | P1 · Proposal | Design an explicit two-device **Pass hosting** policy with one writable holder | Both devices can transfer the right to play even if they sign in at different times, provided the recipient already has or can fetch the committed checkpoint. Partition tests prove there is never a second writer |
| 9 | P1 · Proposal | Provide a supported headless third-replica profile for small private worlds | A small third device can run and restart without Java, acknowledge current saves, and vote under the existing supported membership rules. Either PC plus the replica can form a majority |
| 10 | P1 · Partial | Make invite expiry, reuse, membership approval, pending state, and revocation legible. Review a recipient-bound or one-use invite option | An expired or revoked grant cannot admit a new device. A removed device cannot fetch newer private snapshots. Existing downloaded copies have clearly documented retention limits |
| 11 | P1 · Partial | Expose a clear recovery path when a sole valid copy survives but quorum is gone. Reuse export/import and add a labeled recovered-world workflow | The original bytes are preserved. A new recovered world uses a new identity unless a valid original authority transition exists. UI clearly identifies the old and new worlds |

### Connectivity and real Minecraft validation

| # | Priority and state | Work | Acceptance |
| --- | --- | --- | --- |
| 12 | P0 · Missing | Build actual game-traffic transport with direct TCP or bounded authenticated forwarding. Keep it separate from snapshot transfer | Remote players complete real login and play over each claimed path. Losing game transport does not affect authority validation |
| 13 | P0 · Unverified | Execute the three-device field run for #85 and #120, including opt-out | Alice, Bob, and Carol play on independent networks. The original host is stopped or isolated with approved controls. The correct successor restores the marker, and the original host cannot resume stale writes |
| 14 | P1 · Partial | Finish discovery and connection diagnosis for ordinary players. Show whether directory, peer link, host proof, or Minecraft game path failed | Per-path results are fresh and actionable. Failure has retry/copy options and no endless loading or false Ready |
| 15 | P1 · Unverified | Certify representative LAN, WAN, home NAT, CGNAT, IPv6, mobile, blocked UDP, and packet-loss scenarios | Matrix reports network class, selected control and game routes, success/failure, and reason. Any unsupported path is named accurately |
| 16 | P1 · Partial | Publish deployable discovery bootstrap, private rendezvous and game relay configurations with documented boundaries | A new group can bring up its own compatible node, swap relays, and recover from relay outage. Relay and directory services never grant world authority |
| 17 | P1 · Proposal | Define relay connection, bandwidth, duration, auth and resource policies before shared hosting | Fair-use and disconnect behavior are measured with multiple worlds. Invites and private route details are absent from public logs |
| 18 | P2 · Proposal | Add adaptive replication throttling so background saves do not ruin game latency | On a limited uplink, measured game responsiveness stays within a published target while the replica continues to make bounded progress |

### Hosting consent, membership, and safety

| # | Priority and state | Work | Acceptance |
| --- | --- | --- | --- |
| 19 | P0 · Partial | Prove per-world player-only default, opt-in persistence, and withdrawal through CLI, native IPC, and live hosting | Zero-budget peer can join and replicate with no JVM. Consent stays scoped to one world and persists across restart. Every launch path rejects missing or withdrawn consent |
| 20 | P0 · Partial | Clarify withdrawal lifecycle and handle a hung JVM without silently losing the current save | UI shows **Stopping at checkpoint** until proof of checkpoint/fence/exit, or a truthful error with recovery choices. Stale readiness is cleared immediately |
| 21 | P1 · Partial | Improve process identity and coordinate a safe resource budget across data roots where supported | Local concurrency/orphan tests cover process identity, restart, and no second host when capacity or ownership is ambiguous |
| 22 | P1 · Missing | Add whole-process limits only on platforms that can enforce them. Evaluate delegated Linux cgroups, Windows Job Objects, and macOS capability separately | Measurement includes native memory and children. Unsupported enforcement is labeled heap-only. Existing Java heap and memory headroom rules stay in force |
| 23 | P1 · Partial | Reconcile V2 transcript, signed consent, voter proof and admission work from #122/#125. Plan durable migration from V1 separately | Exact source history, signed approvals, stable and joint voter majorities, replay bounds, no legacy stale writes, interrupted migration and rollback behavior are reviewed and tested |
| 24 | P1 · Partial | Split member, player, replica, voter, administrator, and host-candidate permissions in UI and policy | A non-hosting replica can safely vote. A player or viewer can’t become host from UI preference alone. Membership changes preserve durable quorum rules |
| 25 | P1 · Proposal | Add supported invite/device lifecycle, including contact removal, replacement devices and recovery of a lost identity | Losing a PC has a documented repair path. A removed device cannot regain access through stale presence or an obsolete invite. Private recovery material is only exported with explicit user action |
| 26 | P1 · Proposal | Connect private SwarmCraft membership to Minecraft login/session policy. Decide online-mode/allowlist behavior for the supported game path | Swarm membership and game identity remain distinct and consistently enforced. Authorized people can play, and unintended game connections are rejected |
| 27 | P2 · Proposal | Define public/unlisted/private visibility behavior and moderation roles before public rollout | Visibility never bypasses membership. Admin actions and role changes can be reviewed, and private data does not leak to discovery |

### Snapshots, storage, and scale

| # | Priority and state | Work | Acceptance |
| --- | --- | --- | --- |
| 28 | P0 · Partial | Show accepted checkpoint, last replica acknowledgment, size, lag, and progress since the last recoverable save | A player can tell how much progress is at risk. A locally created but unreplicated save has different status from a verified replica |
| 29 | P1 · Partial | Add backup retention, disk quota and space recovery controls on top of current storage primitives | The newest recoverable checkpoint and in-use roots survive cleanup. Low disk space pauses with a usable remediation path |
| 30 | P1 · Partial | Finish manual checkpoint selection, verified restore and conflict inspection UX | User can inspect/export both divergent histories, choose an explicit recovery outcome, and keep a recoverable original copy |
| 31 | P1 · Proposal | Offer an encrypted checkpoint export/import bundle for backup and deliberate transport. Treat Drive or object storage as optional transport adapters | A interrupted upload never becomes a current save. The recipient validates immutable manifest/blob hashes and signature before restore. Backup cannot grant host authority |
| 32 | P2 · Partial | Download unrelated missing blobs in parallel from multiple authorized replicas, with bounded scheduling and resume | Compare time to verified restore against source fallback. Failure or corruption from one source does not restart or invalidate safe work from others |
| 33 | P2 · Missing | Evaluate incremental region/chunk checkpoints and snapshot scheduling | Demonstrate lower bytes and shorter save window on a controlled dataset. A known full restore still works and is cross-checked |
| 34 | P2 · Research | Evaluate operation journaling and erasure coding independently | Proven improvement against an existing snapshot baseline; fault tolerance, restore complexity, and storage cost are measured before new live formats |
| 35 | P2 · Unverified | Publish capacity with separate player, voter, replica, world-size and relay tests | Real game-server measurements report player counts, TPS and resource ceilings. Replica count is never presented as extra simulation capacity |
| 36 | P2 · Proposal | Add background replica scheduling, quotas, bandwidth windows, and replica placement controls | A user can choose what to cache. Hot worlds retain the configured recoverability target under normal churn without overrunning disk or bandwidth |

### Desktop, runtime, and compatibility

| # | Priority and state | Work | Acceptance |
| --- | --- | --- | --- |
| 37 | P0 · Unverified | Implement installed Windows/macOS native GUI QA for #141 | Exercise actual WebView2/WKWebView installers, real sidecars, real IPC, restart, host consent, and an intentional failure in a separate QA build |
| 38 | P1 · Partial | Decide Svelte cutover after shipping parity review | The candidate covers all legacy journeys, including private invite, host transfer, safe stop/wake, mod setup, and recovery. Installed builds and rollback prove the packaging switch |
| 39 | P1 · Partial | Simplify first run and import. Add a native folder picker when supported and show source/runtime requirements before commit | A fresh user can import a supported world, finish explicit EULA, see capacity and mod blockers, and recover an interrupted install |
| 40 | P1 · Partial | Deliver compatible game-client preparation and handoff to the chosen launcher. Preserve the user's account and settings | Matching Java/Fabric/mod set launches or is clearly directed. Existing Mojang/Microsoft sign-in and entitlement remain intact. No credentials travel through invites |
| 41 | P1 · Partial | Finish modpack dependency acquisition for each supported provider and manual-artifact fallback | A pristine target acquires allowed artifacts from their authorized source, verifies exact bytes, and shows unavailable or incompatible components. No silent redistribution |
| 42 | P2 · Partial | Add a compatibility matrix for Minecraft versions and Fabric builds. Develop and test an adapter before widening support | Every offered tuple passes a real server launch, save, restore and game-client connection. Unsupported versions remain visible with accurate messaging or filtering |
| 43 | P2 · Proposal | Improve accessibility and everyday polish across main actions | Both supported window sizes, keyboard-only flow, screen-reader names/live state, scaling, long names and IDs, and interrupted progress pass on installed builds |
| 44 | P2 · Research | Evaluate additional loaders, Bedrock clients and cross-play separately | Work only from a proven Fabric baseline. Each new adapter documents worlds, mods, game protocol, lifecycle limits, and cross-platform evidence |

### Discovery, privacy, and community

| # | Priority and state | Work | Acceptance |
| --- | --- | --- | --- |
| 45 | P2 · Partial | Build the normal-player world lobby on signed discovery and provider results | Public search and exact unlisted resolve have useful empty states, bounded paging, freshness display, and a separate private Join action |
| 46 | P2 · Partial | Connect friend contact/presence primitives to explicit consent, hide/offline mode and useful invitations | Friends see only the metadata a user agreed to share. Removing a contact stops future private presence access. Offline has honest freshness |
| 47 | P2 · Proposal | Add public-world governance and abuse handling once private play is stable | Listing visibility, moderation, region/language metadata, report flow and local filtering work without granting registry veto power over world history |
| 48 | P2 · Proposal | Add opt-in, redacted diagnostics export and a useful local history of world actions | Users can export time, commit, path class, and event summaries while excluding private invites, credentials, raw private addresses and world data |

### Production engineering and long-term scope

| # | Priority and state | Work | Acceptance |
| --- | --- | --- | --- |
| 49 | P0 · Partial | Reconcile open issues, PRs, and capability claims against main | Every stale ticket has a landed commit or exact residual; open acceptance blockers remain linked. README, roadmap and install instructions agree |
| 50 | P0 · Partial | Verify an immutable main snapshot and rollback, then establish production release evidence | Preview retains previous assets and validates source/version/hash. Tagged release independently verifies Windows signing and Mac notarization; mobile and unsupported packages are clearly scoped out |
| 51 | P1 · Partial | Extend release resilience for download outages, executable provenance, shared runtime cache publication, and dependency health | Corrupt input fails validation. Same-runtime installs from two worlds preserve the last known-good cache. Every generated or downloaded executable has explicit provenance and verification status |
| 52 | P1 · Partial | Establish upgrade, downgrade, schema and interrupted migration acceptance from the supported older preview | One real saved world and consent history survive upgrade and restart. Unsupported downgrade stops clearly, with verified export/rollback guidance |
| 53 | P1 · Partial | Extend crash, disk-full, retention, platform resource and network campaigns beyond current automated gates | Published stress matrix includes repetitions, actual resource limits, last accepted checkpoint, and explicit failure causes. Saved canonical data remains recoverable |
| 54 | P2 · Proposal | Add signed update delivery and platform installation hygiene after release provenance is stable | Client verifies manifest and package signature, separates preview/stable channels, supports opt-out, and rejects an incompatible or untrusted update |
| 55 | P2 · Missing | Define production metrics, support runbooks, and per-version compatibility policy | Field team can diagnose boot, replication, lease, game connect, restore, and publication with bounded and redacted evidence |
| 56 | Expansion · Design prerequisite | Reopen the distributed region/tick feasibility decision for the expanded product scope. Choose the experimental runtime before implementation | Compare the existing single-host baseline with actual multi-host simulation. Document cross-region correctness, compatibility limits and measured performance before adopting the runtime |

### Horizontal simulation and ownerless worlds

| # | Priority and state | Work | Acceptance |
| --- | --- | --- | --- |
| 57 | Expansion · Missing | Add per-device CPU and memory contribution, with desired limits, measured capacity, and a player-only choice | Joining grants no compute access automatically. A player can select 4 GiB and a CPU limit, change or withdraw it, and see requested, admitted and active resources. Zero preserves play where available |
| 58 | Expansion · Missing | Build opt-in worker admission and placement based on usable memory, CPU, network path and region load | A newly admitted machine takes real simulation work only when useful. Inadequate capacity scales down safely. Platform limits and heap versus process memory are visible |
| 59 | Expansion · Missing | Choose or build a region-aware runtime. Keep one world identity as different machines simulate different regions | Players traverse a continuous world while independent machines tick different regions. The supported Minecraft/protocol/mod matrix is documented and verified |
| 60 | Expansion · Missing | Bind every region to a single authorized writer and generation. Define the ownership of global game state | Region reassignment, overlap and worker restart never produce two canonical writers for the same region. World-wide saves include a consistent accepted state |
| 61 | Expansion · Missing | Make player, entity and inventory transitions durable across regions, including block and global interactions in the supported ruleset | Players walk or use a supported portal across workers and preserve identity, state and inventory through restart. Supported cross-region mechanics pass replay and correctness tests |
| 62 | Expansion · Missing | Handle scale-in, loss, and partition. Drain and reassign live work when volunteers leave | A graceful exit drains eligible work and releases the device budget. A crash freezes or fails over affected regions from accepted checkpoints without duplicating state |
| 63 | Expansion · Unverified | Run real distributed gameplay and publish the scale envelope | Compare the same world, load and network with one worker and multiple independent workers. Show increased sustainable active regions or player capacity at the published service target. Publish clustered-load and degraded-network limits |
| 64 | Expansion · Missing | Add a creation-time **Ownerless / no OP** world option with a signed policy | The founding player receives no permanent creator privileges, player OP, unilateral rule changes or unilateral control over recovery. Turning on more compute grants no privilege |
| 65 | Expansion · Missing | Bind enforced permission rules to the canonical runtime and each regional worker | The founder and successor hosts have ordinary in-game permissions. Legitimate players cannot obtain administrator abilities through the supported game or management interfaces. Runtime review covers direct host control as a separate trust boundary |
| 66 | Expansion · Missing | Define shared authority, validator admission and loss recovery for ownerless worlds | A verified majority can keep the world available after the founder leaves permanently. No single key can publish a unilateral policy change or rewrite accepted state. A lost majority leaves verified recovery copies |
| 67 | Expansion · Missing | Separate world governance, public/private participation, and volunteer worker trust | Ownerless can be invite-only or public. Joining a public world grants neither a vote nor write authority. Anonymous game access and eligibility to run or certify workers are separate policies |
| 68 | Expansion · Unverified | Certify ownerless and horizontally scaled operation together | In one real game session, the founder exits permanently, other peers keep the same world available with no player OP, opt-in worker resources change, and players continue across reassigned regions when the required quorum and capacity survive |

Sources for the backlog · [Open issues](https://github.com/MousaXD/swarmcraft/issues?q=is%3Aissue+is%3Aopen) · [Implementation status](https://github.com/MousaXD/swarmcraft/blob/main/docs/IMPLEMENTATION_STATUS.md) · [Product vision](https://github.com/MousaXD/swarmcraft/blob/main/docs/PRODUCT_VISION.md) · [Network validation](https://github.com/MousaXD/swarmcraft/blob/main/docs/NETWORK_VALIDATION.md) · [Distributed simulation research](https://github.com/MousaXD/swarmcraft/tree/main/research/distributed-simulation)

## Build the private world in this order

### 1. Make the next host reachable

Use the existing [Minecraft client route RFC](https://github.com/MousaXD/swarmcraft/blob/main/docs/rfcs/minecraft-client-route.md) as a design input. Start with deliberate reconnect. Keep the optional Fabric client and hands-free retry for a later milestone.

1. In the Rust authority/runtime boundary, publish a bounded, signed offer only after authenticated Fabric readiness and an accepted generation. Bind it to world, compatible runtime, current authority, membership proof, fencing generation, accepted snapshot, short validity, and selected transport.
2. In the resolver, obtain fresh evidence that this generation is still canonical. Bind the offer to a fresh request and accepted quorum or equivalent reviewed authority proof. Keep a durable generation floor. Refuse inconsistent history, missing quorum, stale identities, or unknown route state. A persisted migration report or configured endpoint alone cannot satisfy this check.
3. Build an explicit game route for the supported topology. Bench the direct connection first, then add authorized fallback with bounded connections and backpressure. Maintain separate state for world replication and live game traffic.
4. Connect the signed route through a typed Tauri command and use it for the Desktop Join/Reconnect action. Clear any cached route immediately on a world change or new generation. Show distinct states for waiting for quorum, syncing save, restoring server, verifying connection, and ready to connect.
5. Drive a real Minecraft player through the route. Record pre/post game state, observable client disconnect/rejoin, service reachability, authority generation, and accepted checkpoint. Use the control-plane process tests as regression coverage alongside this field result.

### 2. Turn the transfer wizard into one safe journey

Keep the existing transfer record and Fabric checkpoint barrier. Begin with the three-voter topology so this work can ship without a new voting policy.

1. Select a reachable recipient with verified runtime, compatible mods, current eligible consent, and surviving quorum. Display its checkpoint lag and whether a clean shutdown can follow.
2. Checkpoint and save. Send only the committed signed transfer state after the final snapshot is verified on the recipient. Revalidate authorization at the final boundary where it is used.
3. Persist the transfer identifier and phase. Treat a lost response or restart as a status recovery. Reuse the existing operation with identical identities and intent.
4. Stop or fence the source before the target accepts game traffic. Launch from the exact accepted snapshot. Use the verified route to reconnect clients.
5. Return the old host as an ordinary replica and prove it syncs forward. Show “Safe to turn off” only when the handoff has reached its durable outcome and the remaining peers can sustain the chosen policy.

### 3. Decide which two-friend experience to ship

| Design | What Alice does before logging off | What Bob can do later | Failure behavior |
| --- | --- | --- | --- |
| Current quorum model with a third lightweight voting replica | Sync the latest checkpoint to authorized replicas. Complete the planned transfer or leave enough votes for recovery | Use Bob's PC plus the third voting replica to form quorum. Bob hosts Minecraft if opted in and compatible | If votes or the current checkpoint are unavailable, remain read-only or blocked. Copies can still be recovered |
| Proposed exclusive-turn policy for exactly two PCs | Finish **Save, sync, and hand off**. Persist a relinquishment and transfer write permission to Bob | Start as the named holder after verifying the committed save, even if Alice is off, under the new reviewed policy | If the source crashed before relinquishing, Bob waits or uses explicit assisted recovery. An expired timer alone cannot allow a second writer |
| Optional encrypted checkpoint transport | Upload a completed signed bundle. Publish its manifest only after all required encrypted data is present | Fetch and verify newer checkpoint bytes when no P2P source is reachable | Missing partial uploads remain invisible. Storage or a newer timestamp grants no host permission |

**Order of implementation** · Ship and certify the third-replica private experience. Then review the exclusive-turn policy and prototype it on a newly opted-in world. Add a cloud adapter only if friends need asynchronous transfer when their devices never overlap and no replica stays online.

For the two-device policy, write the state machine and test it before UI. A proof must identify one holder, a monotonically increasing world generation, the exact accepted checkpoint, and durable state on both sides. Define which side may recover each partial handoff. Cover disconnect before sync, restart after commit, delayed delivery, rollback of a local save, lost recipient device, duplicate transfer requests, and stale attempted resume by the old host. Decide up front whether lost-grant recovery creates a separately labeled world or requires a trusted coordination service. Preserve the default world quorum model and existing worlds.

If the desired promise is **automatic takeover after an unexplained crash**, either keep an independent third voter or add a separately reviewed coordination system. When both devices are still independently allowed to write, they cannot safely tell a powered-off computer from a broken link just by waiting. Choose safety, clear availability limits, and a predictable recovery action.

### 4. Validate data loss and reconnect time honestly

Record three timestamps per run · the last accepted checkpoint, interruption, and first successful post-recovery player action. Report data loss using the checkpoint and actual in-game markers, and recovery time from interruption to playable state. Compare clean handoff with a forced crash. Unsaved progress after the accepted checkpoint is a separate measure. Start with a baseline, then agree targets from observed results instead of promising a universal downtime or player count.

## Sabathon schedule

This is a session-based plan because the duration, team size, and available test hardware were unspecified. A weekend can produce a credible vertical slice and evidence. It cannot establish every platform, network, or production guarantee in this backlog.

| Session | Primary work | Parallel work if people are available | Exit evidence |
| --- | --- | --- | --- |
| 0 · Set the baseline | Build the audited tree, verify the main preview if available, provision three independent machines, and agree one supported runtime tuple | Check ticket/PR overlap and document exactly which installers and routes are usable | Installed package hashes, a repeatable setup script/runbook, scheme for redacted evidence, and a verified base commit |
| 1 · First private play | Run the current private create/invite/sync/start flow from fresh profiles and record its first real blocker | Add actionable checkpoint/freshness copy and start Win/Mac installed GUI harnesses | Both players join the same world with one marker and the exact running host is observable |
| 2 · Verified reconnect | Implement a narrow route verifier and direct-path reconnect slice. Keep the initial route mode explicit | Extend current transfer automation for idempotent recovery and confirm opt-out/wake gates | Wrong generation and unavailable quorum refuse a route. A supported direct connection can be followed from authenticated proof to player join |
| 3 · Planned transfer | Connect the new route to the existing signed transfer wizard. Commit the last save, launch the successor, and rejoin | Test package restart and consent withdrawal on native installations | Alice can power off after a valid handoff in the supported three-voter setup. Bob sees the committed marker and no old-host traffic |
| 4 · Crash and recovery | Repeat with approved interruption and no prepared handoff, then test the old host returning | Execute the two-voter loss, unavailable replica, relay and paused-save negatives appropriate to the test environment | Successful cases and blocked cases each have original evidence. Record time to play and the last preserved save |
| 5 · Demo and handover | Walk through the supported path in one uninterrupted recording. Deliver the exact source, packages, evidence and open gaps | Update docs and prepare separate issue drafts for remaining work | Demo claims match the observed results. The release decision still refers to the complete matrix |

### Scope choices for the event

- **Minimum demo** · Two real people join a private world and both read the same retained in-game marker after host transfer while a third voter remains reachable. Require native installed packages and verified manual reconnect for the actual demo claim.
- **If WAN or driver setup takes longer** · Demonstrate the exact route verifier and real local gameplay on controlled devices. Report which network and platform checks were blocked. Keep the three-network claim open.
- **If the main demo passes early** · Add relay connectivity, the withdrawal and all-off wake variants, and a field recording. Prototype two-device exclusive turns on a separate development branch after documenting the policy.
- **After the event** · Add the full game relay, optional client auto-rejoin, two-device mode, Svelte cutover, broader resource enforcement and release rollout in independently testable increments.
- **Keep out of the first demo** · Live V2 activation, public discovery, multiple Minecraft editions, distributed ticks, unbounded performance promises, and a mandatory cloud backend.

### Work that can run in parallel

| Workstream | Start with | Rendezvous |
| --- | --- | --- |
| Rust authority and game routing | Route contract, durable freshness, direct reachability, resolver API | Desktop accepts only this contract. Runtime provides authenticated readiness |
| Desktop private-world flow | Existing wizard, readiness and checkpoint displays, native command adapter | Integrate once Rust can produce a typed route and transfer state |
| Storage and reliability | Checkpoint acknowledgement, restart/resume, disk/backup risk | Handoff completion is bound to the existing signed checkpoint |
| QA and platform packaging | Fresh Windows/Mac/Linux installs, field rigs, in-game state and package evidence | Run against the exact integration commit after the main paths land |
| Release and documentation | Open issue reconciliation, artifact run, fix agreement between documents | Certify the exact shipped build only when the adjacent scope has evidence |

### Main dependencies

```mermaid
flowchart TD
  A["Signed authority and checkpoint"] --> B["Verified game route"]
  B --> C["Desktop reconnect"]
  C --> D["Three-device gameplay"]
  A --> E["Planned host transfer"]
  E --> D
  D --> F["Scoped release evidence"]
```

Windows/macOS installed acceptance and artifact provenance are independent prerequisites for the broad desktop release. The two-device project starts with exclusive-turn policy review. It does not depend on enabling V2 world-policy migration first unless design review selects that implementation path.

## Acceptance and measurement matrix

Run the positive cases and the relevant negative controls on the integrated source. Preserve the application's existing production enforcement. Use sanctioned test machines and a dedicated test world for any intentional failure injection.

| Scenario | Expected outcome | Evidence to retain |
| --- | --- | --- |
| Fresh private Create/Import and Invite | Correct World ID, accepted membership, managed supported runtime, explicit EULA | Package hash, screen/log evidence, accepted member result, starting checkpoint |
| Invite reuse, expiry, remove or wrong world | Behavior matches the published grant policy. A failed request starts neither hosting nor replication | Redacted action result and unchanged membership/checkpoint |
| Alice/Bob only, Alice unexpectedly disappears | One of two votes produces an explicit blocked state and no successor game launch | Vote count, no additional live authority, retained last verified checkpoint |
| Planned transfer with a surviving quorum | Exact final save, target accepts once, source fenced, target runs game, clients rejoin | Signed event identities, JVM timestamps, in-game marker and player inventory before/after |
| Per-role opt-out and withdrawal | Zero-budget user can play. Mid-session withdrawal goes through the safe checkpoint and stop/transfer outcome. Restart cannot re-enable hosting | Budget/consent revision, runtime lifecycle, current authority, post-restart status |
| Crash with three eligible voters and two survivors | Canonical majority, one successor, last accepted checkpoint, actual player rejoin | Pre/post game recording, signed checkpoint and epoch evidence, path class |
| Old host returns or client holds an old route | Old host syncs forward. Stale route is refused or cleared. One current writer remains | Durable generation floor, old/new route result, host process and checkpoint evidence |
| Sleep after clean stop and later wake | One valid successor starts from the exact sleeping checkpoint with a valid majority. Insufficient quorum remains blocked | Sleep-record identifier, checkpoint hash, survivor visibility, actual successful game login |
| Damaged, stale, missing or partially synced checkpoint | No start from unverified data. Retry or explicit restore works without destroying prior accepted data | Verification result, current canonical identity and replayable restore decision |
| NAT/relay/reconnect interruption | Each claimed network path works or returns a bounded, honest failure. Game and replication route results are separate | Network class, no private addresses in public evidence, game success/failure and reconnect duration |
| Old and new desktop versions | Supported upgrade keeps worlds, consent and identity. Unsupported version or downgrade fails clearly | Installer/package type, before/after commit, preserved world state, rollback proof |
| Installed package with broken sidecar | The exact native smoke journey fails visibly at startup or action, while the valid package passes | Same QA script, platform WebView, binary provenance, error state and screenshots |
| Preview verification interrupted | Previous downloadable preview survives. Retry resumes or safely fails on a mismatched draft | Original and new release references, package checksums, rollback attempt |

### Evidence to capture for each real field run

- Exact main or development commit, Git tree and package hash, platform and architecture, Java/Minecraft/Fabric versions, adapter, and whether the installed build is a QA variant.
- Distinct devices, network class per peer, direct/relay selection for both the game and replica paths, and evidence approved for sharing.
- World identity, membership/quorum, host eligibility, consent changes, epoch, accepted snapshot/hash, and the client-visible stage transition.
- Last confirmed in-game marker, inventory or player state, actual game join, planned or unplanned interruption, and proof of the former host's fencing on return.
- Save age at transition, observed progress loss, total recovery time, bounded retries, relevant resource samples and the explicit verdict.
- A separate reviewer links conclusions to original outputs. Mark skipped, unavailable, fixture-based, and packaging-only evidence accurately.

### Capacity measurements after the private demo

| Measure | Split the experiment by | Report |
| --- | --- | --- |
| Minecraft gameplay | Host CPU/RAM, player count, mods, view distance, movement and exploration patterns | Tick performance, latency, memory, peak bandwidth, and useful supported presets |
| Replication | World size, changed bytes, replica count, bandwidth, simultaneous gameplay | Time until one and several verified copies, disk overhead, retry traffic, and last checkpoint age |
| Authority and recovery | Voter count, connect/leave churn, planned transfer, crash, partial quorum, cold restart | Time to one safe authority, time to playable, stale-route result, and recoverable progress |
| Relay and discovery | Direct versus forwarded paths, multiple worlds, load, offline peers | Connection success, response time, bandwidth/capacity cost, and failure messaging |

## Expanded product requirements

### Horizontal simulation using volunteered resources

The product promise is **one playable world that admits additional simulation capacity while people join and volunteer resources**. If Bob offers 4 GiB and up to two CPU cores, his device can simulate a suitable region while Alice's device simulates another. Each worker runs within its own admitted budget. Allocation responds to useful additional capacity, actual load and network quality. A player's joining alone grants no resources, and two machines keep separate memory spaces. The scale target is more active regions or players at an agreed performance level. An overloaded single area can remain a bottleneck.

The current SwarmCraft runtime has one authoritative Minecraft process with warm replicas and successor recovery. The repository's separate feasibility study found general-purpose distributed simulation unsuitable for the current Fabric architecture and contains no real distributed Minecraft benchmark. Treat this as a new runtime track. Region scheduling work such as Folia is a useful architectural reference for ownership and boundaries. Multi-machine SwarmCraft still requires its own network, consistency, and recovery design. [SwarmCraft distributed simulation RFC](https://github.com/MousaXD/swarmcraft/blob/main/research/distributed-simulation/RFC.md) · [Folia region architecture](https://docs.papermc.io/folia/reference/overview/)

1. **Choose a supported first runtime and workload.** Start with a narrow Minecraft version, documented mechanics and approved compatibility set. Run real clients, two physically separate worker machines and a separate control/voting failure domain where needed. Preserve the existing single-host world mode and its saves.
2. **Make resource consent enforceable.** Ask for a memory and CPU limit per device and per world. Show current admission, reserve client and OS headroom, enforce tested CPU and memory boundaries where supported, and display whether the number is a Java heap target or a tested whole-process limit. Support zero, live reductions and withdrawal. Never make a joining player install or run unapproved server workloads.
3. **Allocate by region and keep one canonical state.** Lease regions with fenced generations, and define a deterministic owner or ordered submission path for time, rules, player data and other global effects. Use a stable Minecraft entry point. When an admitted worker leaves, drain work, checkpoint safely and reassign only after the old grant is fenced. Stop affected updates when the remaining authority or resources are insufficient.
4. **Certify gameplay across boundaries.** Cover walking/portals, player identity, inventory, autosave, crash mid-transfer, reconnect and stale worker return. For each supported mechanic, test ordering and safe recovery across boundaries. Include redstone, entities, fluids and cross-region updates in the declared support matrix. Keep unsupported mods and mechanics out of the compatibility claim.
5. **Prove useful horizontal capacity.** Before measuring, define hardware, workload, network conditions, tick-time targets and accepted data-loss/recovery bounds. Compare one active worker against two or more on geographically spread and concentrated loads. Record authoritative work placement, player capacity or active region count, tick latency, bandwidth, and measured memory/CPU. Also show the player experience when a worker reduces a budget or powers off.

**Call horizontal scaling done when** the same world is played through independent active machines, a new 4 GiB and CPU contribution takes observable game work, supported players cross regions with state intact, healthy comparable workloads sustain more than with one active worker, and removing that worker produces an observed safe drain, failover, or clear blocked state.

### Ownerless, no-OP world option

Keep **Ownerless / no OP** as a creation-time option. “2b2t-inspired” describes a persistent world and its gameplay philosophy. This SwarmCraft mode adds the explicit requirement that no player, including the founder and current host, has operator powers. Joining rules are a separate choice, so the world can be public or invite-only. Publish the exact limits of any gameplay, spam or infrastructure policies at creation.

1. **Bind the rules to the world identity.** Sign the creation policy and bind accepted runtimes, worker grants and game permission enforcement to it. Remove built-in founder privileges and disable player OP and privileged console/RCON interfaces in this mode. Decide which rule invariants are fixed, which published changes require a quorum, and when a policy change requires a new world identity.
2. **Separate everyday play from maintenance authority.** Joining, playing, seeding data, running simulation and certifying the next generation are separate roles. Donation size buys no governance privileges. Use an explicit validator enrollment model with a known failure and trust assumption. Recover from accepted checkpoints without the founder's key while enough accepted replicas and votes survive.
3. **Define what the mode can enforce on volunteer devices.** The software can deny game-level permissions and reject state outside accepted rules. A person with OS control can modify a process running on their device. Decide which machines can execute authoritative work, what their peers independently verify, and how the mode responds when trust or quorum is lost. Keep the public join experience separate from admission to trusted execution.
4. **Prove there is no privileged founder or player.** Observe a normal founder session, normal volunteer-host session, founder departures and restarts, replacement of accepted peers under the published rules, and continued play on the same world. Attempts to grant ordinary players operator rights or unilaterally change protected rules fail and leave state unchanged. A quorum outage produces the documented safe outcome.

**Call the ownerless mode done when** regular players, the creator and active hosts all have the advertised game permissions, the founder can permanently leave while the remaining quorum keeps the same world operating, a single participant cannot unilaterally change protected rules, and the enforcement limits on volunteer hosts are evidenced and published.

### Expanded release finish line

The original private-world sabathon demo remains the first milestone. The **full expanded SwarmCraft vision** also requires both completion tests above, plus one combined game session where the founder leaves permanently and an active worker is added or removed while the same world continues under the supported quorum and resource conditions. Broader editions, loaders and optional research retain separate scope and acceptance criteria.

## Release decisions

**Demo-ready** means a repeatable installed private-world journey on the explicitly tested platform and networks. **Preview-ready** additionally means the exact commit passes automated validation, package verification, and live install/configuration for the advertised subset. **Production-ready** requires the complete supported platform and network matrix, independent current-source review, a verified upgrade/rollback story, and policy-approved signed/notarized artifacts.

- Link the release notes to the exact evidence. Distinguish source, test-only package, unsigned preview and signed release.
- Implement the Windows/macOS test adapter in isolated QA builds. Run unmocked IPC for native acceptance, and inspect the shipping package for driver absence. Use a separate installed production-path smoke where the fixture differs. [Tauri WebDriver documentation](https://v2.tauri.app/develop/tests/webdriver/)
- Keep package hashes, provenance and commit references in the verified artifacts. Consider signed build attestations and an SBOM after the current version and signing flow works end to end. [Release workflow](https://github.com/MousaXD/swarmcraft/blob/main/.github/workflows/release.yml) · [GitHub artifact provenance](https://docs.github.com/en/actions/how-tos/secure-your-work/use-artifact-attestations)
- Keep three-device gameplay, resource promises, platform interaction and signing as separate release decisions. A passing packaging run answers the packaging question.

## Where to start in the repository

| Work | Existing entry points |
| --- | --- |
| Authority, leases and candidate selection | [Daemon](https://github.com/MousaXD/swarmcraft/blob/main/crates/swarm-cli/src/daemon.rs) · [Consensus](https://github.com/MousaXD/swarmcraft/tree/main/crates/swarm-consensus) |
| Save/stop, runtime readiness and transfer | [Migration](https://github.com/MousaXD/swarmcraft/blob/main/crates/swarm-cli/src/migration.rs) · [Fabric bridge](https://github.com/MousaXD/swarmcraft/tree/main/minecraft/fabric) · [Tauri transfer commands](https://github.com/MousaXD/swarmcraft/blob/main/apps/desktop/src-tauri/src/transfer_commands.rs) |
| Trusted game routing | [Client route RFC](https://github.com/MousaXD/swarmcraft/blob/main/docs/rfcs/minecraft-client-route.md) · [Current Desktop continuity](https://github.com/MousaXD/swarmcraft/blob/main/apps/desktop/src/client-continuity.js) |
| Local consent and process admission | [Hosting budget](https://github.com/MousaXD/swarmcraft/blob/main/crates/swarm-cli/src/hosting_budget.rs) · [Native command contract](https://github.com/MousaXD/swarmcraft/blob/main/apps/desktop/src-tauri/src/hosting_contribution_commands.rs) |
| V2 foundations | [Hosting policy RFC](https://github.com/MousaXD/swarmcraft/blob/main/docs/rfcs/hosting-policy-migration.md) · [Signed quorum PR #125](https://github.com/MousaXD/swarmcraft/pull/125) · [Admission PR #122](https://github.com/MousaXD/swarmcraft/pull/122) |
| Networking and preservation | [Network crate](https://github.com/MousaXD/swarmcraft/tree/main/crates/swarm-network) · [Storage crate](https://github.com/MousaXD/swarmcraft/tree/main/crates/swarm-storage) · [Snapshot replication](https://github.com/MousaXD/swarmcraft/blob/main/docs/SNAPSHOT_REPLICATION.md) |
| Private join, discovery and mod support | [Invite validation](https://github.com/MousaXD/swarmcraft/blob/main/crates/swarm-cli/src/invite.rs) · [Discovery](https://github.com/MousaXD/swarmcraft/blob/main/crates/swarm-cli/src/discovery.rs) · [Protocol runtime support](https://github.com/MousaXD/swarmcraft/blob/main/crates/swarm-protocol/src/runtime_support.rs) |
| Desktop and Svelte | [Shipping Tauri config](https://github.com/MousaXD/swarmcraft/blob/main/apps/desktop/src-tauri/tauri.conf.json) · [Svelte candidate](https://github.com/MousaXD/swarmcraft/tree/main/apps/desktop/frontend) |
| Exact-source release and field evidence | [Release gates](https://github.com/MousaXD/swarmcraft/blob/main/docs/RELEASE_GATES.md) · [Package verifier/publisher](https://github.com/MousaXD/swarmcraft/tree/main/scripts/ci-release) · [Field runbook](https://github.com/MousaXD/swarmcraft/tree/main/scripts/e2e/three_machine) |


