# Agent 7 — Player journey

## Recovery status

`RECOVERED DIRECTLY ON FINAL INTEGRATION`

No live `agent/player-setup-migration-ux` / Agent 7 branch existed when final recovery resumed. The repository therefore could not truthfully provide an exact Agent 7 source-head handoff. The missing player-facing integration was completed directly on `integration/player-launcher-v1` rather than inventing history.

## Recovered player journey

- Authoritative Minecraft and Fabric selectors are active in the Desktop create/import journey.
- Create World now gains a provider-mod selector for Modrinth and CurseForge.
- Provider selections resolve exact dependency graphs, download through backend-managed staging, and are inspected by Rust for true Fabric mod identity.
- Canonical Create World persists exact canonical provenance/hashes and installs the selected exact JARs into the world mod profile.
- Legacy compatibility text/bootstrap-address controls are hidden from the ordinary player path.
- Public-world discovery and exact world-ID resolve are exposed in Desktop while membership remains separate.
- Backend errors are preserved as actionable player messages.
- Runtime install/repair can reacquire permitted exact provider artifacts from the frozen signed canonical provenance; manual/restricted artifacts stop with explicit remediation.

## Wiring evidence

`app.js` imports `import-flow.js`; `import-flow.js` imports both `catalog-selectors.js` and `launcher-controller.js`. The Tauri command table registers the launcher/provider/discovery bridge. Desktop frontend and Tauri bridge checks were green on the integrated validation head, and `launcher-controller.test.mjs` covers exact identity/provenance, missing required dependency fail-closed behavior, and actionable provider errors.

## Handoff

There is no independent Agent 7 branch to consume. Agent 8 owns validation of the recovered journey on the final integration SHA.
