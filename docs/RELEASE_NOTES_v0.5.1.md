# SwarmCraft 0.5.1 Release Notes

SwarmCraft 0.5.1 is a **technical-preview patch release** focused on resolving the installed desktop startup blocker and enforcing a real packaged-GUI acceptance gate. The wire protocol remains **version 1**; this is not a protocol-breaking release.

## Fixed: installed desktop startup (#82)

- Removed the false initial **Ready** indicator. The desktop reaches Ready only after device setup, identity verification, and world enumeration.
- Fixed the native Tauri startup crash caused by attempting to overwrite a read-only `window.__TAURI__.core.invoke`. Provider calls now adapt arguments without mutating the native bridge.
- Added a pre-module watchdog, bounded startup steps, a visible failure banner, and **Retry startup** instead of indefinite Loading / Checking.
- Aligned canonical GUI world creation and CLI sidecar world listing with the same `SWARMCRAFT_DATA_DIR` override.
- Restrained Minecraft/Fabric choices to versions supported by the *shipped* adapter while keeping newer official catalog entries visible but unavailable.

## Added: real packaged desktop CI (#83)

- Required exact-commit validation extracts the actual Linux `.deb`, verifies all four bundled sidecars, and drives the installed Tauri/WebKitGTK GUI with native WebDriver—without mocked Tauri IPC.
- Checks clean first launch, GUI world creation, relaunch, and persistence of device identity and world selection.
- A deliberate missing-sidecar negative control must visibly fail with **Startup failed** and **Retry startup**. The normal acceptance cannot pass with the broken installed package.
- Retains sanitized GUI evidence for troubleshooting.

## Platform and release boundaries

The real installed-GUI acceptance is currently **Linux/WebKitGTK only**. Windows `.exe` and macOS `.dmg` building is validated, but their installed interactive GUI sessions are not certified by this gate.

This version bump synchronizes the Rust workspace, excluded Desktop and provider crates, both Cargo lockfiles, Tauri package metadata, and Fabric mod version. It does **not** create a signed production release or a `v0.5.1` tag. Production tagging remains subject to the repository's exact-SHA validation and signing/notarization requirements.

Implemented and reviewed in [PR #88](https://github.com/MousaXD/swarmcraft/pull/88); issue [#82](https://github.com/MousaXD/swarmcraft/issues/82) and [#83](https://github.com/MousaXD/swarmcraft/issues/83) are closed.
