# SwarmCraft Desktop

The SwarmCraft desktop application is the current player-facing technical-preview shell for the Rust runtime.

It is a **Tauri 2** application with a deliberately small frontend stack:

```text
apps/desktop/
├── src/
│   ├── index.html   # semantic structure
│   ├── style.css    # visual system
│   └── app.js       # DOM behavior and Tauri command wiring
└── src-tauri/
    ├── src/         # Rust commands/runtime process management
    └── tauri.conf.json
```

The desktop bundles SwarmCraft runtime sidecars and invokes the same CLI/runtime functionality used outside the GUI.

## Current capabilities

The 0.2.1 desktop preview includes:

- local peer initialization and identity display;
- world listing and safety state;
- world creation;
- signed invite creation and invite-based joining;
- membership leave requests;
- compatibility/authority-eligibility checks;
- play/host startup;
- graceful world sleep/host stop;
- background replica seeding controls;
- conflict inspection;
- peer/membership inspection;
- snapshot verification;
- export and recovery tools;
- replication daemon controls;
- networking/runtime diagnostics;
- activity/error history.

The UI deliberately distinguishes canonical state, solo/degraded state and preserved conflict state. It must also keep authority eligibility separate from merely holding a storage replica.

## Current limitations

The desktop is not yet a one-click Minecraft launcher.

Hosting may still require the user to provide:

- a Java runtime path/configuration;
- a compatible Fabric server JAR;
- the SwarmCraft Fabric mod JAR;
- explicit Minecraft EULA acceptance.

The app also does not yet automatically start Minecraft on a newly elected recovery successor or automatically reconnect players after host migration.

Those are major remaining product milestones rather than hidden completed features.

## Development constraints

Read the repository-level [AGENTS.md](../../AGENTS.md) before changing the desktop UI.

In particular:

- keep the existing HTML/CSS/JavaScript frontend unless an architectural change is explicitly required;
- do not add a framework, CSS framework, icon pack, font dependency or frontend build pipeline merely for styling;
- preserve protocol/safety semantics when simplifying UI;
- validate the configured default and minimum window sizes;
- preserve Tauri command names/payloads unless changing the Rust API intentionally.

## Relevant docs

- [Project status](../../docs/IMPLEMENTATION_STATUS.md)
- [Product vision](../../docs/PRODUCT_VISION.md)
- [Roadmap](../../ROADMAP.md)
- [Network validation](../../docs/NETWORK_VALIDATION.md)
