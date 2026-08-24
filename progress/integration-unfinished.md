# August 24 Feature Completion + Integration Audit

## STATUS
IN PROGRESS

This file is the source-of-truth integration ledger for the unfinished August 24 feature wave.

Environment note: this execution environment does not expose a local repository checkout, so local `git fetch --all --prune` / local branch inventory cannot be run honestly here. Remote branches, exact SHAs, PR metadata, and exact-head GitHub Actions are verified directly through GitHub and are treated as authoritative. No change in this ledger should be read as proof of an unrun local command.

## Repository state before fixes

Remote branches verified before any engineering edits:

- `main` @ `105b19ade82be606e5a855df4e82ce18bb7e885a`
- `backup/local-work-20260824` @ `41c9b5b650aac1e320195f6e1855945f2722abc4`
- `agent/minecraft-fabric-catalog` @ `68c6713d6658b0bcc6011803f9684564e3e562c1`
- `agent/modrinth-provider` @ `c5d76875c33645bd64c6bc0109c8adef68d68621`
- `agent/curseforge-provider` @ `344f086eaa7499ba2e4dfa86f6e27cd3410f5d5a`
- `agent/automatic-invites` @ `110ed6f9558ab2417b281725018fc11dc70ae5fc`
- `agent/discovery` @ `2105d4f5d897fcfbbd24918fdaf8609fa2a0c2b7`

Expected later-stage branches are absent remotely and must not be treated as existing work:

- `agent/canonical-modpack`
- `agent/player-launcher-journey`
- `integration/player-launcher-v1`

All audited feature heads matched the checkpoint SHAs supplied in the completion brief before this ledger was added.

Open PRs verified from current GitHub PR metadata:

- #42 `backup/local-work-20260824` -> `main`, head `41c9b5b650aac1e320195f6e1855945f2722abc4`, draft, mergeable, 15 changed files
- #44 `agent/modrinth-provider` -> `backup/local-work-20260824`, head `c5d76875c33645bd64c6bc0109c8adef68d68621`, draft, mergeable, 13 changed files
- #45 `agent/discovery` -> `backup/local-work-20260824`, head advanced to `a3e8cc4e0e45bbb10ced99369c4a931642f19940` when this audit ledger was first committed, draft, mergeable, 11 changed files
- #46 `agent/automatic-invites` -> `backup/local-work-20260824`, head `110ed6f9558ab2417b281725018fc11dc70ae5fc`, draft, mergeable, 6 changed files
- #47 `agent/minecraft-fabric-catalog` -> `backup/local-work-20260824`, head `68c6713d6658b0bcc6011803f9684564e3e562c1`, draft, mergeable, 16 changed files
- #48 `agent/curseforge-provider` -> `backup/local-work-20260824`, head `344f086eaa7499ba2e4dfa86f6e27cd3410f5d5a`, draft, mergeable, 10 changed files

Audit corrections:

- PR #48 is the CurseForge validation PR. The backup-to-main PR is #42.
- The feature validation PR mapping is Modrinth #44, Discovery #45, Automatic Invites #46, Catalog #47, CurseForge #48.

Branch classification:

- `main`: stable base; no August 24 feature PR above is merged into it.
- `backup/local-work-20260824`: active unmerged foundation/staging branch, not disposable debris. Live compare against `main` is 2 commits ahead, 0 behind, 15 changed files. It contains tracked Desktop lockfile work, runtime/migration hardening, runtime setup hardening tests, installer documentation changes, gitignore hardening, and the progress ledger framework. It must be audited intentionally before use as an integration base and must not be merged to `main` merely because feature branches descend from it.
- Catalog, Modrinth, CurseForge, Automatic Invites, Discovery: active and unmerged.
- No verified superseded remote branch was found in the audited branch set.
- `progress/agent*.md` ledgers are advisory only. Several record older SHAs or pending validation and therefore do not supersede live GitHub state.

## Branch: agent/discovery

STATUS: BLOCKED (compile/lint failures; fix in progress)

BRANCH: `agent/discovery`

EXACT HEAD SHA before audit ledger: `2105d4f5d897fcfbbd24918fdaf8609fa2a0c2b7`

BASE SHA: `41c9b5b650aac1e320195f6e1855945f2722abc4`

PR: #45 -> `backup/local-work-20260824`

FILES CHANGED versus PR base: 11 after the audit ledger commit

BUGS FOUND:

- Rust E0308 in `crates/swarm-cli/src/discovery.rs`: match arms using `detail.get_or_insert(error)` return `&mut String` where `()` is required.
- Rust E0502 in friend update flow: mutable borrow from `store.friends.iter_mut()` remains live across `save_friend_store(paths, &store)`.
- Clippy `large_enum_variant` failures in discovery/network-related enums reported by exact-head CI.
- Discovery CI also reports unused-assignment warnings around observed host/world state.
- Formatting/lint validation is red and must not be bypassed.

FIXES: pending engineering patch.

CONTRACT CHANGES: none accepted yet. Required semantics remain PRIVATE hidden, UNLISTED non-browsable but exact/invite resolvable where designed, PUBLIC discoverable, using existing authenticated SwarmCraft networking.

TEST COMMANDS required:

- `cargo fmt --check`
- `cargo check --workspace`
- `cargo clippy --workspace --all-targets --all-features`
- `cargo test --workspace`
- project-specific discovery/network tests

TEST RESULTS: pre-fix exact-head CI is red.

CI RUN/CHECK RESULTS before fixes:

- CI run `32727751915`: FAILURE
- PR Target Guard run `32727751916`: SUCCESS
- Release Guard run `32727751906`: SUCCESS
- dedicated discovery validation run `32745647013`: FAILURE

BLOCKERS: genuine compiler and clippy failures.

NEXT DEPENDENCY: Minecraft/Fabric catalog only after Discovery is technically green.

## Branch: agent/minecraft-fabric-catalog

STATUS: BLOCKED (dedicated validation stops at formatting)

BRANCH: `agent/minecraft-fabric-catalog`

EXACT HEAD SHA: `68c6713d6658b0bcc6011803f9684564e3e562c1`

BASE SHA: `41c9b5b650aac1e320195f6e1855945f2722abc4`

PR: #47 -> `backup/local-work-20260824`

FILES CHANGED versus PR base: 16

BUGS FOUND: dedicated catalog validation fails at `cargo fmt --check`; downstream functional validation does not run.

FIXES: pending.

CONTRACT CHANGES: none accepted yet; Mojang/Fabric authoritative catalogs and compatibility enforcement remain required.

TEST COMMANDS intended by dedicated validation include catalog fixture generation/cache tests, Minecraft version validation, frontend fixture checks, catalog regeneration comparisons, and authoritative Fabric/Minecraft checks.

TEST RESULTS:

- normal CI run `32721432165`: SUCCESS
- dedicated catalog validation run `32744102086`: FAILURE at formatting before functional validation

BLOCKERS: formatting must be corrected, then the whole dedicated suite must run.

NEXT DEPENDENCY: Modrinth provider.

## Branch: agent/modrinth-provider

STATUS: BLOCKED (Desktop validation job red)

BRANCH: `agent/modrinth-provider`

EXACT HEAD SHA: `c5d76875c33645bd64c6bc0109c8adef68d68621`

BASE SHA: `41c9b5b650aac1e320195f6e1855945f2722abc4`

PR: #44 -> `backup/local-work-20260824`

FILES CHANGED versus PR base: 13

BUGS FOUND: the supplied audit description saying provider unit tests are red is stale. Live dedicated run `32735413224` shows provider validation, Rust workspace tests, Network soak, and Actionlint green; the failing job is Desktop build/checks.

FIXES: pending exact Desktop failure diagnosis.

CONTRACT CHANGES: none accepted yet. Official Modrinth API, compatibility filtering, dependency resolution, permitted download, artifact verification, and deterministic provider metadata remain required.

TEST RESULTS:

- normal CI: SUCCESS
- PR Target Guard: SUCCESS
- Release Guard: SUCCESS
- dedicated validation run `32735413224`: FAILURE overall
- dedicated Modrinth provider validation job: SUCCESS
- Rust workspace tests job: SUCCESS
- Network soak job: SUCCESS
- Actionlint job: SUCCESS
- Desktop build/checks job: FAILURE

BLOCKERS: exact Desktop validation failure must be fixed rather than weakening provider tests.

NEXT DEPENDENCY: Automatic Invites.

## Branch: agent/automatic-invites

STATUS: BLOCKED (cross-platform CI failure)

BRANCH: `agent/automatic-invites`

EXACT HEAD SHA: `110ed6f9558ab2417b281725018fc11dc70ae5fc`

BASE SHA: `41c9b5b650aac1e320195f6e1855945f2722abc4`

PR: #46 -> `backup/local-work-20260824`

FILES CHANGED versus PR base: 6

BUGS FOUND: CI is red while Network Soak and release/target guards are green. Last audit symptom is macOS membership convergence timing out after invite acceptance; root cause still requires exact diagnosis and must not be papered over with an arbitrary sleep increase.

FIXES: pending.

CONTRACT CHANGES: connectivity hints must come from backend networking state; normal users must not construct libp2p multiaddresses or infer NAT truth in JavaScript.

TEST RESULTS:

- Release Guard: SUCCESS
- PR Target Guard: SUCCESS
- Network Soak: SUCCESS
- CI: FAILURE

BLOCKERS: macOS/integration convergence failure needs diagnosis.

NEXT DEPENDENCY: CurseForge provider.

## Branch: agent/curseforge-provider

STATUS: READY TECHNICALLY / RELEASE BOOKKEEPING RED

BRANCH: `agent/curseforge-provider`

EXACT HEAD SHA: `344f086eaa7499ba2e4dfa86f6e27cd3410f5d5a`

BASE SHA: `41c9b5b650aac1e320195f6e1855945f2722abc4`

PR: #48 -> `backup/local-work-20260824`

FILES CHANGED versus PR base: 10

BUGS FOUND: no technical CI failure currently observed; Release Guard is red.

FIXES: no isolated version bump will be made until combined release strategy is understood.

CONTRACT CHANGES: none accepted yet; official CurseForge API/no scraping/no committed secret/machine-local credential configuration/compatibility/dependency/permitted download/deterministic metadata/graceful missing-credential behavior must remain verified.

TEST RESULTS:

- normal CI: SUCCESS
- PR Target Guard: SUCCESS
- dedicated CurseForge validation: SUCCESS
- Release Guard: FAILURE

BLOCKERS: release-version bookkeeping only unless further validation uncovers a technical defect.

NEXT DEPENDENCY: combined integration after upstream technical reds are cleared.

## Integration stages

`integration/package-discovery-foundation`: NOT CREATED. Must wait for upstream technical green.

`agent/canonical-modpack`: NOT CREATED. Must wait for exact-green package/discovery foundation.

`agent/player-launcher-journey`: NOT CREATED. Must wait for exact-green canonical modpack.

`integration/player-launcher-v1`: NOT CREATED. Must wait for exact-green player journey and final acceptance audit.

## Current next action

Fix `agent/discovery` at its current remote head, update this ledger plus `progress/agent6.md`, then require exact-head GitHub validation before proceeding downstream.
