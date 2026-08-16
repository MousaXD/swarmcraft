# SwarmCraft 0.2.1 Release Notes

SwarmCraft 0.2.1 is a preview UX and documentation release built on the 0.2.0 distributed-runtime foundation.

Application version 0.2.1 does **not** imply wire protocol version 2. The current wire protocol remains version 1 unless an explicit protocol-breaking change says otherwise.

## Desktop UX overhaul

The Tauri desktop application now uses a clearer desktop information architecture centered on worlds rather than raw daemon controls.

The main navigation separates:

- Worlds;
- Create world;
- Join world;
- Activity;
- Diagnostics.

World rows and selected-world detail now make safety, membership, Minecraft compatibility and latest checkpoint information easier to scan.

## Safer player-facing semantics

The UI now avoids treating every apparently healthy replica state as equivalent.

It distinguishes:

- canonical history;
- solo/degraded history;
- preserved solo-history conflict;
- authority-eligible runtime state;
- storage-only/ineligible replicas.

Play/host controls are disabled when the selected node is not authority eligible or when preserved divergent history requires recovery attention.

## Better runtime feedback

Desktop actions now produce timestamped activity entries with clearer success/failure context.

World-specific controls follow the selected world consistently, and diagnostics explain why hosting is unavailable instead of leaving invalid actions enabled.

## Current runtime foundation

0.2.1 retains the 0.2.0 control-plane foundation, including:

- authenticated libp2p/QUIC networking;
- snapshot replication;
- signed membership/configuration;
- authority leases and fencing;
- durable recovery ballots/certificates;
- explicit solo history and conflict preservation;
- Fabric lifecycle/save integration;
- cross-platform CI and native desktop packaging.

## Documentation correction

Earlier repository docs described SwarmCraft as if it were still at the initial architecture/foundation stage. That no longer matched the executable repository.

The current docs now distinguish:

- features that are implemented in code;
- features with working control-plane support but incomplete product orchestration;
- roadmap-only/future research.

See:

- `README.md`;
- `docs/IMPLEMENTATION_STATUS.md`;
- `ROADMAP.md`;
- `docs/RELEASE_GATES.md`.

## Important remaining limitations

SwarmCraft remains preview software.

In particular, 0.2.1 does **not** yet provide:

- automatic Minecraft runtime launch on a newly elected crash-recovery successor;
- automatic player reconnection after authority migration;
- a complete player-facing manual authority-transfer workflow;
- automatic per-world Minecraft/Fabric/mod installation;
- a complete public/friends world lobby;
- representative field certification across home NAT, CGNAT, mobile and IPv6 environments;
- distributed Minecraft region/tick simulation.

The largest MVP integration target remains turning safe authority election/recovery into a seamless Minecraft host handoff.
