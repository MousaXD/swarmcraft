# Svelte + TypeScript migration workspace (NOT SHIPPING)

This is the dedicated frontend migration workspace for SwarmCraft issue #87. The installed app **continues to load** `apps/desktop/src` from Tauri 2 until all parity, locked-build and installed-GUI gates pass. Do not point production at this unfinished workspace.

## What exists

- Svelte 5, TypeScript and Vite source/configuration in an isolated directory.
- A transitional `App.svelte` that retains every pre-existing DOM ID and loads the existing tested desktop controllers **after Svelte mounts**. This is a compatibility scaffold, not a claim that those controllers are Svelte-native or reactive.
- Classic PR #88 startup watchdog copied byte-for-byte to `public/bootstrap.js`, with a test preventing drift.
- A typed, non-patching native invoke wrapper for incremental controller extraction; backend remains authoritative.
- A pure startup-state model and world-list parser with tests.
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

A production cutover additionally needs Agent 00 approval for `src-tauri/tauri.conf.json` (`build.frontendDist`), tightly coordinated workflow changes so Vite runs before every package build, unchanged CSP/sidecars, real installed WebKitGTK acceptance, missing-sidecar negative control, and documented Windows/macOS package builds. These changes are **not** made here.

Never replace a missing action with a placeholder or claim UI completeness based on compilation. `App.svelte` presently delegates behaviors to the old JS modules; migrate individual flows only alongside tests and native evidence.
