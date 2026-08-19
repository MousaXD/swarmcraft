# Host Readiness and Shutdown Safety

SwarmCraft's shutdown answer is a backend safety decision, not a frontend estimate.

The question is deliberately stricter than "does another replica exist?" A replica can preserve bytes while still being unable to become Minecraft authority. Likewise, a reachable peer can be ineligible for authority, have stale state, or lack the runtime/mod set required to start the world.

## Structured contract

The daemon continuously publishes a machine-readable report at the machine-local control path for the world. The supported CLI view is:

```text
swarmcraft world host-readiness <world> --json
```

Desktop consumes the same contract through the Tauri `host_readiness` command. Reports older than the freshness window are converted to `unknown` with `safe_to_shutdown=false`; stale reachability is never reused as a green decision. Desktop must render `safe_to_shutdown` from this backend report directly rather than deriving it from peer counts or connectivity UI.

The report separates:

- `state`: the player-facing safety category;
- `safe_to_shutdown`: true only for an actually proven green state (or an already sleeping world);
- `successor_peer_id`: a peer that can automatically recover after this host disappears;
- `handoff_candidate_peer_id`: a peer that is otherwise host-ready but requires an explicit authority transfer first;
- `world_data_replicated`: whether another peer has proven the exact current canonical snapshot;
- per-peer reachability, canonical-state match, authority eligibility, runtime readiness, server-mod readiness, conflict status, and surviving recovery-quorum status.

## Exact green calculation

For an active world on the current host, `safe_to_shutdown=true` requires at least one successor for which **all** of these are true:

1. The peer is an authenticated, currently reachable world member.
2. The peer reports the exact accepted epoch, sequence, snapshot hash, state root, and world compatibility fingerprint.
3. Canonical membership and the peer's current status both say it is authority eligible and it is not banned.
4. The peer's machine-local runtime is `ready`, meaning the configured Java/server/SwarmCraft Fabric artifacts still exist and match a runtime proof produced after a successful Fabric launch/compatibility handshake.
5. Required user server mods are `ready`. A signed world with user server-mod requirements is fail-closed until the machine-local mod manager has verified the exact compatibility fingerprint.
6. The peer reports no unresolved solo-history conflict.
7. From the successor's own authenticated/fresh view, a recovery quorum still exists **excluding the current authority**.

The last rule prevents a common false green. In a two-member world, Bob can have a perfect replica and runtime but Bob alone is not a quorum after Alice powers off. SwarmCraft therefore returns `blocked_by_quorum` and may expose Bob as `handoff_candidate_peer_id`. Alice must transfer authority before shutdown.

This report does not grant authority and cannot weaken recovery safety. Actual election, recovery ballot, quorum, fencing, lease, and migration checks remain authoritative even if a peer misreports its machine capability.

## States

| State | Meaning | `safe_to_shutdown` |
| --- | --- | --- |
| `safe` | A specific successor satisfies the full automatic-takeover calculation. | `true` |
| `sleeping` | The world is already durably sleeping; no Minecraft host is being interrupted. | `true` |
| `world_will_stop` | No other device currently proves it can keep hosting. A replica may still exist. | `false` |
| `syncing` | A reachable peer has not yet proven the exact canonical head. | `false` |
| `blocked_by_runtime` | A canonical authority-eligible peer lacks a verified compatible runtime. | `false` |
| `blocked_by_mods` | Runtime is ready, but required server mods are missing/incompatible/unverified. | `false` |
| `blocked_by_quorum` | A peer is host-ready, but automatic recovery would lose quorum. Transfer first. | `false` |
| `degraded_safety` | Solo/conflict-related safety is not suitable for an automatic green decision. | `false` |
| `conflict` | Conflicting history is preserved and requires resolution. | `false` |
| `not_current_host` | This device is not the current host and removing it may affect quorum; current implementation fails closed. | `false` |
| `unknown` | Required state is missing or stale. | `false` |

## Peer capability exchange

`HostCapabilityV1` is an ephemeral authenticated network message. It is intentionally separate from signed world state because Java files, installed mods, reachability and process readiness are machine-local facts that can change without changing the world history.

The capability contains:

```text
world_id
compatibility_fingerprint
runtime
server_mods
conflict_free
recovery_quorum_without_authority
```

The current host combines this capability with its own fresh `WorldStatusV1` observation. A capability by itself is never enough: the peer must independently match the exact canonical snapshot and accepted epoch.

## Runtime producer contract

`host_readiness::record_runtime_verified(...)` is the machine-local producer API.

Migration Core records this only after:

1. Minecraft/Fabric has actually launched;
2. Fabric IPC has reported the expected Minecraft version, loader version, world directory and compatibility fingerprint;
3. the local authority generation is still accepted.

The proof records the configured runtime paths plus hashes of the server JAR and SwarmCraft Fabric JAR. Reconfiguring runtime paths invalidates the prior proof before the new configuration is saved. Changed JAR bytes therefore fail closed as `incompatible`.

The Runtime Installer agent may call the same producer after its own authoritative `runtime_verify` succeeds. Merely downloading files or writing a config must not record `ready`.

## Server-mod producer contract

`host_readiness::record_server_mod_readiness(...)` is the integration point for the server-mod manager.

For a world with no user server-mod requirements (or only the legacy compatibility marker), readiness can be derived as `ready`. If the signed `WorldConfigV1.compatibility.required_server_mods` contains user server mods, the state is `unverified` until the mod manager checks the exact requirements and records one of:

```text
ready
missing
incompatible
unverified
```

The record is keyed to the signed compatibility fingerprint, so a changed requirement set cannot inherit an old green result.

Fabric Loader, Fabric API and SwarmCraft's own integration remain runtime/platform concerns; arbitrary third-party server mods are not silently downloaded by this subsystem.

## Safety boundaries

The implementation intentionally keeps these facts separate:

```text
replica present
!= authority eligible
!= runtime ready
!= mods ready
!= reachable
!= surviving recovery quorum
```

A shutdown banner may translate the final backend state into player language, but JavaScript must not recompute or relax the safety calculation.
