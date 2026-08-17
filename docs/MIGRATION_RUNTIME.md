# Migration Runtime Orchestration

SwarmCraft uses one backend runtime path for authority recovery, manual authority transfer, and safe world wake. The orchestration boundary deliberately consumes the existing authority generation, fencing, snapshot, quorum-permit, and Fabric IPC mechanisms rather than replacing them.

## Shared pipeline

The runtime supervisor follows this order:

1. Observe an accepted local authority generation.
2. Require the exact changing quorum permit for multi-member worlds.
3. Select and cryptographically verify the canonical snapshot.
4. Re-check the accepted authority generation.
5. Prepare the local Minecraft runtime and restore the snapshot.
6. Re-check authority before launch.
7. Launch Minecraft and establish authenticated Fabric IPC.
8. Verify Minecraft/Fabric compatibility and the restored world directory.
9. Re-check authority and publish runtime-ready state plus the configured game endpoint.
10. While running, continuously reject generation changes. A superseded runtime is terminated and cannot commit a canonical snapshot.
11. On safe stop or transfer, use the Fabric save barrier, checkpoint, sign, and commit the resulting snapshot only while the same authority generation is still accepted.

The standalone host entry point routes through this same implementation. Multi-member hosting cannot bypass the exact-generation quorum-permit gate.

## Backend state for Desktop

`swarmcraft world migration-status <world> --json` exposes the current authority peer, epoch, fencing token, migration trigger and phase, runtime-ready flag, game endpoint when configured, snapshot hash, and failure reason.

`swarmcraft world runtime-configure` stores machine-local Java/server/Fabric launch configuration. `swarmcraft world wake` submits a wake intent. The manual transfer stages are exposed through `transfer-prepare`, `transfer-export`, `transfer-accept`, `transfer-commit`, `transfer-activate`, and `transfer-observe`.

Desktop code should consume this backend state instead of inferring readiness from peer preference or process presence.

## Safety boundaries

Automatic recovery launches only after the recovered generation is accepted and the existing quorum permit is live. Manual transfer advances exactly one epoch and fencing token from the source generation and requires the exact canonical checkpoint. A returning former authority is fenced by the accepted successor generation.

Single-member durable sleep can wake locally through the shared path. Multi-member wake remains intentionally blocked until a quorum-backed wake authority transition is available; the backend does not invent a solo authority transition for a replicated world.

The manual transfer records and epoch tokens are currently exposed as a signed backend/CLI exchange. Automatic peer-to-peer advancement of all manual-transfer phases over the existing network message is a follow-up integration item.

Manual transfer records are generation-scoped operational state. Prepared, Accepted, and Committed records remain durable across restart while their source authority generation is still accepted. Once the accepted epoch/fencing generation advances to the committed successor or any later generation, that record is terminal historical evidence: supervisor launch gating and future transfer preparation ignore it. Replayed phase tokens are still signature-checked and rejected unless they exactly extend the currently accepted source generation and canonical base snapshot.
