# SwarmCraft Player Journey Audit

**Audit date:** 2026-08-18  
**Audited base:** `main` at `e8fa4ffff16b4437156fd2c2dadc596298f8bafc`  
**Perspective:** a Minecraft player who does not understand libp2p, quorum, authority epochs, snapshots, Fabric internals, or signed records.

This audit grades what a normal player can discover and complete from the Desktop application. A backend feature does not count as a complete player journey merely because a CLI command exists.

## Classification

- **GREEN** — works and is obvious from Desktop.
- **YELLOW** — technically works in at least an important subset of cases, but is confusing, incomplete, hidden, or weakly proven.
- **RED** — cannot be completed as a normal Desktop journey.
- **FALSE GREEN** — Desktop implies the capability is complete or safe, but the inspected end-to-end path does not prove that claim.

## Executive result

SwarmCraft now has a credible launcher-shaped front door: Worlds, Create, Join, Play, Invite, Stop, Wake, and structured connectivity are visible in player language. The application no longer opens as a raw distributed-systems control panel.

The normal **Create → Play → Invite → friend becomes host-ready → safely shut down** journey is still blocked in several places:

1. **Play is manual-runtime-first.** `Play` redirects to Diagnostics unless the user supplies a Fabric server JAR and SwarmCraft Fabric mod JAR and accepts the EULA.
2. **Fabric API is not prepared.** The SwarmCraft Fabric mod declares `fabric-api` as a dependency, while runtime preparation resets the runtime directory and installs only `swarmcraft-fabric.jar` into `mods/`.
3. **Existing Minecraft world import is absent from Desktop.**
4. **Server mods are not a first-class world/player workflow.** There is no add/remove/enumerate/required-mod flow, and the runtime directory is rebuilt for authority launch.
5. **Desktop cannot answer “Can I turn off my PC?”** It has no authoritative composite state covering replica freshness, authority eligibility, runtime readiness, required mods, and reachability.
6. **Manual host transfer is intentionally disabled in Desktop.**
7. **The Stop UI overstates graceful behavior.** Desktop reports a graceful stop after killing the owned host sidecar, while the canonical graceful checkpoint/sleep path lives inside the host runtime's Fabric shutdown barrier.
8. **Wake is not a complete normal multi-peer lifecycle.** Single-member durable sleep has a backend wake path; multi-member wake is explicitly blocked pending a quorum-backed wake transition.

The current product is therefore best described as a strong technical preview with an improved launcher shell, not yet a clean-machine consumer Minecraft launcher.

## Journey matrix

| Journey | Current behavior | Expected player behavior | Severity | Grade | Responsible subsystem | Recommended fix | Backend work required? |
| --- | --- | --- | --- | --- | --- | --- | --- |
| First launch: understand where to begin | Sidebar and empty state expose **Worlds**, **Create**, and **Join**. Startup automatically initializes the device and attempts to start networking. | Player immediately sees Create or Join without node/daemon setup. | Low | **GREEN** | Desktop | Keep current launcher-first hierarchy. Consider one sentence explaining that worlds can move between friends' PCs. | No |
| Create a world record | Create form asks for name and visibility; compatibility settings are tucked into a details panel with defaults. | Create a named world with normal defaults. | Low | **GREEN** | Desktop / core | Keep compatibility details advanced. | No |
| Create a usable Minecraft world | After creation, Play still requires manual runtime paths. | Create world, press Play, automatic preflight/setup, EULA, launch. | Blocker | **RED** | Runtime installer + Desktop wizard | Add structured runtime status/install/repair backend and setup wizard. | **Yes** |
| Play on a clean machine | `runtimeValidationIssue()` requires Fabric server JAR, SwarmCraft mod JAR, and EULA from Diagnostics. Java defaults to `java`; missing Java is discovered only by launch failure. | Play checks Java/Minecraft/Fabric/Fabric API/SwarmCraft integration and prepares missing components. | Blocker | **RED** | Runtime installer + Desktop wizard | Replace manual-path validation as normal path with backend preflight/install. Keep manual paths only in Advanced. | **Yes** |
| Fabric API availability | SwarmCraft's Fabric metadata requires `fabric-api`, but authority runtime preparation creates a new `mods/` directory and copies only the SwarmCraft Fabric bridge. | Fabric API is resolved and installed automatically for the selected Minecraft/Fabric environment. | Blocker | **RED** | Runtime installer | Install and verify compatible Fabric API as a managed runtime component. | **Yes** |
| Existing Minecraft world import | No Desktop/Tauri import-world action is present. Export/recovery exist, but they are not an import journey. | Choose an existing Minecraft world folder, validate it, then create a SwarmCraft world from it. | High | **RED** | Core/storage + Desktop | Add an explicit import flow with validation, snapshot/genesis creation, and clear copy-vs-move semantics. | **Yes** |
| Add/remove server mods | No Mods surface exists in the selected world UI. | Add JAR, inspect Fabric metadata, remove mod, open mods folder. | High | **RED** | Server mod management + Desktop | Add first-class server-mod manifest and local management UI. | **Yes** |
| Preserve server mods across host launch | Runtime preparation resets the managed runtime and installs only SwarmCraft's Fabric bridge. | User-required server mods are deterministically restored into every eligible host runtime. | Blocker for modded worlds | **RED** | Server mod management + migration runtime | Materialize required world mods from an authoritative manifest before launch. | **Yes** |
| Know which peers have required mods | Desktop exposes generic authority eligibility, not a player-facing per-peer runtime/mod readiness matrix. | “Bob: Ready”, “Sarah: Missing Lithium”. | High | **RED** | Host readiness + mod management | Add backend readiness contract that distinguishes replica, authority, runtime, mods, reachability. | **Yes** |
| Create an invite | Selected-world **Invite** dialog creates a signed invite, lets the user choose expiry, and offers Copy Invite. Bootstrap addresses are advanced. | Click Invite, copy, send to friend. | Low | **GREEN** | Desktop / core | Preserve the simple default. | No |
| Join from an invite | Join view explicitly asks the recipient to paste the complete signed invite. | Paste invite, click Join. | Low | **GREEN** for comprehension | Desktop / core | Keep signed-token mechanics out of the normal explanation. | No |
| Invite a friend over the public internet | Signed invite UX exists, but real-world NAT/relay usability is not universally field-proven and invites may be created without explicit bootstrap addresses. | Invite works without the player understanding bootstrap/relay internals, or gives actionable connectivity guidance. | High | **YELLOW** | NAT/network + Desktop | Use authoritative connectivity state to warn before sharing an invite when no viable path/discovery exists; continue field validation. | Possibly |
| Understand internet connectivity | Main world panel shows a structured label such as Direct connection, Connected through relay, or Connection needs attention. Low-level listen multiaddress stays in Diagnostics. | Simple “friends can reach you / still checking / needs attention” with technical details optional. | Medium | **YELLOW** | NAT/network + Desktop | Make the primary copy explicitly answer friend reachability and next action; keep multiaddresses advanced. | Minor |
| Manual host transfer | Button exists but adapter deliberately forces transfer capability off because Desktop lacks one safe complete orchestration command. | Pick an eligible ready peer and transfer hosting safely. | High | **RED** | Migration core integration + Desktop | Expose one Desktop-safe orchestration command after target readiness is authoritative. | **Yes** |
| “Can I turn off my PC?” — only host | No shutdown-readiness state exists. | Explain that the world will go offline, while replicated data status remains separate. | Blocker | **RED** | Host readiness | Add authoritative shutdown readiness state. | **Yes** |
| “Can I turn off my PC?” — healthy successor | App does not prove successor freshness + authority eligibility + runtime + mods + reachability as one decision. | “Safe to shut down. Bob can take over.” | Blocker | **RED** | Host readiness | Add backend composite readiness and player-facing state. | **Yes** |
| Shutdown — successor missing runtime | Migration may later report runtime configuration missing on the local candidate, but there is no pre-shutdown peer readiness answer. | “Bob has the world but cannot host until runtime setup completes.” | Blocker | **RED** | Host readiness + runtime installer | Feed per-peer runtime readiness into shutdown calculation. | **Yes** |
| Shutdown — successor missing mods | No required server-mod readiness contract exists. | “Bob is missing required server mods.” | Blocker | **RED** | Host readiness + mod management | Include required-mod hash/version satisfaction in host readiness. | **Yes** |
| Shutdown — successor offline | Connectivity is shown for this device, not composed into successor host readiness. | “No other ready host is currently reachable.” | Blocker | **RED** | Host readiness + networking | Add peer reachability to authoritative readiness result. | **Yes** |
| Shutdown — replica exists but cannot host | UI correctly notes that storage-only replica and authority eligibility differ, but it does not turn that into a shutdown decision. | “Another copy exists, but it cannot host.” | High | **YELLOW** concept / **RED** decision | Host readiness | Reuse this distinction in the shutdown state. | **Yes** |
| Intentionally stop Minecraft | **Stop world…** explains that it stops local Minecraft and that replica storage can continue separately. However `sleepWorld()` calls `stop_host`, and Desktop's process layer uses `CommandChild.kill()`. The UI then says the runtime stopped gracefully. | Request Fabric save/shutdown barrier, commit final snapshot, then report success. | Blocker | **FALSE GREEN** | Desktop runtime process control + migration runtime | Add an explicit graceful-stop backend command that waits for checkpoint/sleep result; reserve process kill for force-stop diagnostics. | **Yes** |
| Understand stop vs replication vs leave | Separate controls exist for Stop world, Keep/Stop seeding, and Leave world; the stop dialog explicitly distinguishes runtime from replica storage. | Player understands each action affects a different layer. | Medium | **GREEN** for wording | Desktop | Keep this separation. Add explicit “closing SwarmCraft” behavior. | No |
| Understand closing SwarmCraft | No normal player-facing explanation or close flow says whether Minecraft remains running, networking stops, or migration/replication continues. | Closing app has an explicit, safe policy and warning where needed. | High | **RED** | Desktop lifecycle | Add close interception/policy tied to authoritative runtime/readiness state. | Possibly |
| Wake a single-member durably sleeping world | Backend supports safe single-member wake and Desktop can expose Wake when capability probing succeeds, provided runtime config already exists. | Click Wake and server returns. | Medium | **YELLOW** | Migration core + Desktop | Make state explicit and connect wake to automatic runtime repair/setup. | Minor/Yes for setup |
| Wake a normal multi-member sleeping world | Migration core explicitly blocks multi-member wake until a quorum-backed wake authority transition exists. | Any eligible peer can safely wake according to quorum policy without CLI plumbing. | High | **RED** | Migration core | Implement quorum-backed multi-member wake transition, then expose it as one player action. | **Yes** |
| Java missing | No managed Java preflight/install. Launch eventually fails. | Detect and install/use a compatible managed Java runtime. | Blocker | **RED** | Runtime installer | Managed Java. | **Yes** |
| Fabric server missing | UI tells user to set a Fabric server JAR in Diagnostics. | Resolve/install Fabric automatically. | Blocker | **RED** | Runtime installer | Managed Fabric server/loader setup. | **Yes** |
| SwarmCraft integration missing | UI tells user to locate the SwarmCraft Fabric mod JAR manually. | Install the integration matching the app build. | Blocker | **RED** | Runtime installer | Resolve bundled/release artifact automatically and verify it. | **Yes** |
| Server mod incompatible/missing | No first-class user mod manifest or preflight exists. | Identify exact missing/incompatible mod and block host eligibility with remediation. | Blocker for modded worlds | **RED** | Mod management + host readiness | Parse Fabric metadata, hash artifacts, compare deterministic requirements. | **Yes** |
| Network unavailable | Structured connectivity states and a top-level service warning exist; local worlds remain usable. | Clear explanation of offline impact and retry path. | Medium | **YELLOW** | NAT/network + Desktop | Add player-specific next actions and distinguish discovery failure from host safety. | Minor |
| Peer unavailable / authority disappears | Migration status has player-facing phases and backend automatic recovery orchestration exists when safety prerequisites and runtime config are satisfied. The player does not get a complete successor-readiness explanation. | “Host disconnected; Bob is taking over” or “world will remain offline because …”. | High | **YELLOW** | Migration + host readiness + Desktop | Combine migration state with readiness and recovery reason into one world-status narrative. | **Yes** for readiness |
| Migration blocked by missing runtime config | UI translates the blocked state to **Action required** and adds **Set up Minecraft runtime**, but that action only opens the manual Diagnostics runtime fields. | Setup action repairs/installs the missing runtime automatically. | High | **FALSE GREEN** | Runtime installer + Desktop wizard | Connect action to real preflight/install wizard. | **Yes** |

## Detailed findings

### 1. First launch is finally launcher-shaped

`apps/desktop/src/index.html` puts **Worlds**, **Create**, and **Join** at the top of navigation. The empty-world state also presents **Create world** and **Join with invite**. `startup()` in `apps/desktop/src/app.js` automatically initializes the node, starts networking if possible, then loads worlds.

This is the strongest part of the current player journey. Device identity and listen multiaddresses still exist, but they are pushed into the sidebar status and Diagnostics instead of blocking the player at startup.

### 2. Create is easy, but Create → Play is broken for normal players

The Create form is appropriate for the target audience: world name and visibility are primary; Minecraft/Fabric compatibility fields are behind **Compatibility settings** and have defaults.

The break occurs on Play. `runtimeValidationIssue()` in `apps/desktop/src/app.js` requires:

- Fabric server JAR path;
- SwarmCraft Fabric mod JAR path;
- EULA checkbox.

If any are absent, the app navigates to Diagnostics and focuses the manual field. `hostWorld()` then sends those paths to `configure_world_runtime` and `host_world`.

This violates the desired clean-machine path. The normal player is still expected to know where JAR files live.

### 3. Fabric API is a concrete clean-runtime blocker

`minecraft/fabric/src/main/resources/fabric.mod.json` declares:

- Fabric Loader `>=0.19.3`;
- Minecraft `~26.1.2`;
- Java `>=25`;
- `fabric-api: "*"`.

In `crates/swarm-cli/src/migration.rs`, runtime preparation resets the managed runtime directory, creates `runtime/mods`, and copies only the configured SwarmCraft Fabric bridge to `mods/swarmcraft-fabric.jar` before starting the server.

No Fabric API artifact is installed by this path. No Desktop field asks for it either. Therefore a clean runtime cannot be considered complete merely because `serverJar` and `modJar` are filled in.

### 4. Existing-world import is missing

The inspected Desktop bridge exposes create, join, leave, status, compatibility, conflicts, verification, export, recovery, migration status/wake, runtime configuration, connectivity diagnostics, and process controls.

There is no Desktop/Tauri `import_world` flow. A player with an existing `.minecraft/saves/<world>` or server world directory cannot turn it into a SwarmCraft world from Desktop.

This should be treated as missing product functionality, not an advanced-only omission.

### 5. Server mods are not yet a stable world concept in the player path

The selected-world UI has no Mods section. There is no add/remove/open-folder operation, no Fabric metadata inspection, no duplicate-ID warning, and no per-peer required-mod comparison.

More importantly, authority runtime preparation reconstructs its runtime directory and installs the SwarmCraft bridge itself. User server mods are not materialized from a world manifest during that preparation. Even a manual “copy JARs into the runtime folder” workaround is therefore not a dependable host-migration story.

### 6. Invite UX is good; public-internet certainty is not yet good enough

The Invite flow is straightforward:

1. Alice selects a world and presses **Invite**.
2. She chooses expiry, creates a signed invite, and copies it.
3. Bob opens **Join**, pastes the complete `scinvite:…` token, and presses **Join world**.

The invite encodes signed genesis/membership information and may contain bootstrap addresses. Desktop correctly hides bootstrap address entry under an Advanced details disclosure.

The remaining issue is operational certainty: users need to know whether the invite is likely to work across the internet without understanding relays or bootstrap nodes. Structured connectivity is a good base, but the app should translate it into “friends can reach this device” and remediation before the user sends an invite.

### 7. Connectivity language improved, but it does not yet close the loop

`backend-adapter.js` maps backend states into player labels such as:

- Checking connectivity;
- Direct connection;
- Direct connection established;
- Connected through relay;
- Relay needed;
- Connection needs attention;
- Discovery unavailable;
- Could not reach other peers.

This is substantially better than exposing AutoNAT/DCUtR terminology. The selected world also displays Connectivity directly.

The missing piece is a player action model. For example, **Discovery unavailable** should say whether an invite will still work through an existing direct/relay path, and **Connection needs attention** should offer a retry or concise next step rather than sending the player into multiaddresses.

### 8. There is no authoritative shutdown answer

The app separately knows some useful facts:

- local world safety;
- local authority eligibility;
- local connectivity;
- migration phase;
- whether this device keeps a replica.

It does not expose the combined state needed to answer **Can I turn off my PC?**

A correct shutdown answer must not infer host readiness from one of those facts. It needs an authoritative backend calculation that distinguishes, for every possible successor:

- sufficiently current canonical state;
- authority eligibility;
- compatible Minecraft/Fabric runtime;
- required server mods;
- current reachability;
- whether a safe authority transition is possible now.

Until that exists, all of the requested shutdown scenarios remain RED even though individual control-plane pieces exist.

### 9. Stop is worded well but the implementation does not prove graceful stop

The player-facing dialog is careful to say that **Stop world** stops local Minecraft and that background replica storage can continue separately. That distinction is good.

However, the execution path is:

`sleepWorld()` → `backend.stopHost()` → Tauri `stop_host` → `RuntimeProcesses::stop_host()` → generic `stop()` → `CommandChild.kill()`.

After this returns, Desktop displays **World runtime stopped gracefully.**

The canonical graceful runtime path in `crates/swarm-cli/src/migration.rs` is different: it asks the Fabric session to prepare shutdown, waits for Minecraft to stop, creates the final snapshot, signs/commits it, writes the durable sleep record, and publishes Sleeping state.

Desktop does not currently call that explicit graceful checkpoint path. Therefore the success message is a **FALSE GREEN**. A future backend command should request graceful stop and return only after the final checkpoint/sleep result is durable; a raw process kill should be an explicitly dangerous force-stop tool.

### 10. Wake exists, but the common replicated-world lifecycle is unfinished

The migration backend has a real wake intent and shared runtime supervisor. Single-member durable sleep can wake through it.

For multi-member worlds, the backend deliberately publishes Blocked because a quorum-backed wake authority transition is not yet available. That safety decision is correct, but it means a normal friends SMP cannot yet rely on **Stop today → anyone eligible wakes it tomorrow** as a finished Desktop workflow.

This gap is made more confusing by the current Desktop Stop path because a raw sidecar stop does not prove that the durable sleep record was produced in the first place.

### 11. Migration blocked by runtime is actionable in wording but not in capability

Desktop nicely normalizes a missing runtime configuration into **Action required** rather than **Migration failed**. It even creates a **Set up Minecraft runtime** button.

Today that button simply opens Diagnostics at the manual Java/server-JAR/mod-JAR/EULA fields. The label therefore promises more assistance than the product provides. Once the runtime installer exists, this is the correct place to launch repair/preflight.

## Recommended integration order

### P0 — required for the clean-machine promise

1. Runtime installer/preflight backend: managed Java, Minecraft server, Fabric Loader/server launcher, Fabric API, SwarmCraft integration, directories, verification, explicit EULA state.
2. Desktop setup wizard wired to that contract; Play must launch the wizard rather than Diagnostics when setup is incomplete.
3. Server-mod manifest and deterministic host compatibility checks.
4. Authoritative host-readiness/shutdown state.
5. Explicit graceful-stop backend command that completes the Fabric save barrier and durable checkpoint before reporting success.

### P1 — required for a convincing friends-SMP lifecycle

1. Existing-world import.
2. Safe Desktop manual host transfer after target readiness is authoritative.
3. Quorum-backed multi-member wake.
4. Player-facing connectivity remediation and pre-invite reachability guidance.
5. Explicit application-close behavior for running worlds and replication.

## Ownership map

- `agent/runtime-installer`: Java/Minecraft/Fabric/Fabric API/SwarmCraft runtime preparation and repair.
- `agent/runtime-wizard-ui`: Play preflight/setup wizard and Advanced manual controls.
- `agent/server-mod-management`: server-mod manifest, metadata, hashes, add/remove/open-folder, peer compatibility.
- `agent/host-readiness`: authoritative shutdown/readiness state.
- migration core: graceful stop orchestration, safe transfer integration, multi-member wake transition.
- NAT/internet: authoritative reachability inputs and field validation.

## Audit source paths

Primary player-path evidence was taken from:

- `apps/desktop/src/index.html`
- `apps/desktop/src/app.js`
- `apps/desktop/src/backend-adapter.js`
- `apps/desktop/src-tauri/src/main.rs`
- `apps/desktop/src-tauri/src/runtime.rs`
- `apps/desktop/src-tauri/src/runtime_commands.rs`
- `apps/desktop/tests/frontend-contract.test.mjs`
- `crates/swarm-cli/src/migration.rs`
- `crates/swarm-cli/src/host_main.rs`
- `crates/swarm-cli/src/invite.rs`
- `minecraft/fabric/src/main/resources/fabric.mod.json`
- `docs/MIGRATION_RUNTIME.md`
- `docs/IMPLEMENTATION_STATUS.md`
- `docs/PRODUCT_VISION.md`

## Bottom line

The Desktop shell now tells the player **where to go**. The remaining product gap is that several important buttons still terminate in infrastructure plumbing instead of completing player jobs.

The release-defining journey is not green until a clean device can complete:

`Create world → Play → automatic runtime setup → explicit EULA acceptance → Minecraft starts → Invite friend → friend becomes host-ready → Safe to shut down`

without manual JAR hunting or distributed-systems knowledge.
