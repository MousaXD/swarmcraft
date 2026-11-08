# Svelte + TypeScript migration workspace (NOT SHIPPING)

This is the dedicated frontend migration workspace for SwarmCraft issue #87. The installed app **continues to load** `apps/desktop/src` from Tauri 2 until all parity, locked-build and installed-GUI gates pass. Do not point production at this unfinished workspace.

## What exists

- Svelte 5, TypeScript and Vite source/configuration in an isolated directory.
- Ten extracted Svelte components retain all 171 existing DOM IDs. `App.svelte` mounts them before loading the original controller for still-unported flows. The Create, Join and Import forms now have native Svelte submit/busy/error handlers and exact Rust IPC calls; Activity is a reactive Svelte store. Complex hosting, runtime, provider, safety, recovery, diagnostics and navigation workflows still delegate to legacy modules. Do not mislabel the transitional architecture as complete.
- Classic PR #88 startup watchdog copied byte-for-byte to `public/bootstrap.js`, with a test preventing drift.
- A typed, non-patching native invoke wrapper for incremental controller extraction; backend remains authoritative.
- Typed native World actions, fail-closed world/safety modeling, a reactive startup/session coordinator staged for atomic cutover, and component/native-contract tests. The separate session coordinator is **not** currently instantiated alongside the legacy one.
- Feature inventory and blockers in `../SVELTE_MIGRATION.md`.

## Development and gates

Use Node 22.12+. `package-lock.json` was generated on a clean GitHub Actions Node 22.16 runner and committed after matching the exact Git blob SHA-1 with the downloaded candidate artifact. Run `npm ci` (never an unlocked install) to reproduce the dependencies. This is candidate-build evidence, not a certification of the native Tauri installer.

```sh
cd apps/desktop/frontend
npm ci                      # use the committed lockfile
npm run check
npm run test
npm run build
node --test tests/source-parity.test.mjs
```

## Isolated native candidate acceptance

Production still serves `../src`, but the candidate can be packaged **without modifying production Tauri configuration** using `frontend/tauri.candidate.conf.json`, whose only override is `build.frontendDist`. See:

- `.github/workflows/svelte-migration-validation.yml` — locked Svelte, TypeScript, Vitest and source parity.
- `.github/workflows/svelte-candidate-native-gui.yml` — actual installed Linux Debian/WebKitGTK first boot, create, restart, persistence, missing-sidecar negative control, and visual/error checks at 980×760 / 720×560.
- `.github/workflows/svelte-candidate-platform-builds.yml` — Windows NSIS and macOS arm64/Intel DMG **packaging only**. These are not proof the UI was opened on those platforms.

The new Svelte Create, Join and Import forms use exact Rust command names and a legacy **completion-only** world-list refresh bridge. Duplicate native submissions from the legacy form handlers are suppressed only while the Svelte owner flags are active. Tests and native screenshot evidence are mandatory before qualifying these new paths as stable.

A production cutover additionally needs Agent 00 approval for `src-tauri/tauri.conf.json` (`build.frontendDist`), tightly coordinated workflow changes so Vite runs before every package build, unchanged CSP/sidecars, real installed WebKitGTK acceptance, missing-sidecar negative control, and documented Windows/macOS package builds. These changes are **not** made here.

Never replace a missing action with a placeholder or claim UI completeness based on compilation. The majority of rich desktop behaviors remain delegated to old JavaScript modules. Migrate each with contract tests and installed-GUI evidence; do not replace a working action with a placeholder.

### Failure before Svelte can render

A classic candidate startup guard and minimal static fallback DOM/CSS ensure a broken or missing Vite module still produces `Startup failed` and a clickable retry, rather than a blank WebKitGTK window. PR #88's original `bootstrap.js` is copied verbatim and continues to own the failure state. Do not remove the fallback until the native deliberately-damaged installer test remains green.
