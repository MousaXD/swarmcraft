# Runtime Player Journey Integration

This document records the integration contract implemented on `integration/runtime-player-journey` for independent release audit. It does not replace exact-head CI results.

## Integrated sources

- `agent/runtime-installer`
- `agent/server-mod-management`
- `agent/host-readiness`
- `agent/runtime-wizard-ui`
- `agent/runtime-setup-hardening`

## Player journey contract

Normal **Play** uses backend runtime preflight and the Runtime Wizard. Runtime setup is owned by `swarmcraft-runtime`; Desktop does not download or assemble Java launch commands in JavaScript. Minecraft EULA acceptance remains explicit and is persisted only after the player accepts it.

Runtime verification is the readiness boundary. Managed runtime verification records machine-local runtime proof only after authoritative verification. Manual Advanced configuration remains usable, but static file existence is not a host-readiness proof: the exact configured runtime must pass the authenticated Fabric compatibility handshake before it is considered verified.

Canonical server-mod requirements are verified by ID, version, side/environment, and artifact hash. Desktop may supply or remove a local copy of an already-canonical required artifact; it does not mutate the signed world modpack.

## Shared launch and stop paths

Desktop managed launch delegates to the shared daemon/migration runtime orchestration path that consumes persisted `RuntimeLaunchConfig`.

Desktop **Stop world** requests the shared safe-stop path. Success is reported only after the Fabric shutdown/save barrier, Minecraft process exit, final signed canonical snapshot, durable signed sleep record, and `sleeping` migration status. A save/barrier failure must not be presented as a graceful stop.

## Host Readiness

The selected-world screen includes the player-facing question **Can I turn off this PC?**. Wording is mapped from structured backend Host Readiness state; JavaScript does not recompute takeover safety.

## Packaging

Desktop packaging stages all three Rust sidecars on supported platforms:

- `swarmcraft`
- `swarmcraft-host`
- `swarmcraft-runtime`

The Fabric CI artifact check verifies that the released SwarmCraft Fabric JAR embeds Fabric API under `META-INF/jars`, so the normal path does not require a separately hunted Fabric API JAR.

## Installer hardening

Core artifact hashing is implemented in Rust. Installer locking uses an OS-backed file lock so a crashed process does not permanently wedge setup while concurrent live installers remain excluded. Replacement preserves a known-good artifact until the new verified artifact is ready to publish and attempts rollback if publication fails.

## CI contract

The integration branch CI runs the repository Rust matrix and process-level acceptance tests, all Desktop JavaScript tests via `node --test apps/desktop/tests/*.test.mjs`, native Desktop packaging, Fabric build and embedded-Fabric-API verification, dependency audit, fuzz smoke, and impaired QUIC resume.

## Audit boundary

This branch must not be merged to `main` until an exact-head CI run and the independent player-journey audit have completed. Known out-of-scope or intentionally unavailable product gaps, including existing-world import and unsafe multi-member wake shortcuts, remain documented as limitations rather than being simulated in Desktop.
