# Server Mods and Runtime Profiles

SwarmCraft treats third-party server mods as part of a world's deterministic runtime compatibility profile.

## Canonical requirements

`WorldConfigV1.compatibility.required_server_mods` is the canonical requirement set. Each physical third-party mod requirement carries:

- Fabric mod ID;
- version;
- exact SwarmCraft artifact hash of the JAR bytes;
- server/both side classification.

The compatibility manifest is already fingerprinted into world identity and signed world configuration. Protocol v1 therefore does **not** permit changing the canonical mod set in place after world creation. A changed canonical profile would produce a different compatibility fingerprint and is rejected by the existing world/config validation rules.

For new CLI-created worlds, repeat `--server-mod <jar>` on `swarmcraft world create`. SwarmCraft validates `fabric.mod.json`, rejects client-only or duplicate IDs, hashes the exact JAR bytes, includes those requirements before calculating the world fingerprint, and stores the verified local copies under the world runtime profile.

## Local mod store

Third-party JARs live in:

`worlds/<world-id>/runtime-profile/mods/`

This directory is persistent and separate from the ephemeral Minecraft runtime directory. Launch preparation verifies it against the canonical profile before staging JARs into the runtime `mods/` directory.

Useful commands:

```text
swarmcraft world mods-status <world> --json
swarmcraft world mods-add <world> <jar>
swarmcraft world mods-remove <world> <mod-id>
swarmcraft world mods-path <world>
```

`mods-add` is remediation, not a canonical profile mutation. It only accepts the exact ID, version, and hash already required by the world. This is important for joined worlds and replacement of missing local artifacts.

## Managed runtime components

Fabric Loader, Fabric API, and the SwarmCraft Fabric integration are runtime/platform components rather than user server mods. Agent 3 does not download or own those artifacts. `fabric-api` and `swarmcraft` requirements are classified as `managed_runtime` so the user-mod readiness check does not claim they are missing from the third-party mod store. The runtime installer remains responsible for proving those components ready.

## Host readiness and migration

Before the Minecraft authority runtime is prepared, SwarmCraft evaluates local third-party server mods against the canonical compatibility manifest. Missing required mods, wrong versions, hash mismatches, duplicate/conflicting IDs, invalid JARs, client-only JARs, or unexpected user mods make the local server-mod profile not ready and block launch.

This closes the local safety hole where authority migration could reset the ephemeral runtime and launch without the world's required third-party mods.

The network authority-election status still exposes only the canonical compatibility fingerprint, not a peer's live machine-local mod readiness. A peer can therefore be selected by the existing recovery layer and then be blocked by runtime preparation if its local third-party mod set is incomplete. The host-readiness subsystem should consume the structured `ServerModReadiness` contract and propagate/aggregate live readiness rather than infer it from the canonical fingerprint.

## What is synchronized

Automatically synchronized:

- signed canonical runtime/mod requirements through the existing world configuration replication;
- requirement IDs, versions, sides, and exact artifact hashes.

Not automatically synchronized or downloaded in phase one:

- arbitrary third-party mod JAR bytes;
- third-party marketplace metadata;
- post-creation canonical mod-set changes.

Players must explicitly provide third-party JARs locally. SwarmCraft verifies them before use. No arbitrary third-party mod download policy is introduced by this work.

## JAR inspection safety

The local inspector reads root `fabric.mod.json` from standard stored or DEFLATE-compressed JAR entries. It bounds JAR size and metadata expansion, rejects encrypted/multi-disk/ZIP64 metadata entries, rejects duplicate metadata entries, verifies the ZIP CRC32, validates Fabric mod IDs, and does not execute downloaded scripts or JAR code during inspection.
