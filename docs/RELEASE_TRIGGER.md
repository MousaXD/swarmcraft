# Preview Release Trigger

Preview releases are cut from merged `main` only after the permanent CI matrix and release gates are green on the intended release commit.

Tagged release artifacts are rebuilt by `.github/workflows/release.yml` rather than reusing arbitrary local binaries.

Before tagging, verify that application/package version metadata is coherent across the Rust workspace, desktop/Tauri configuration, Fabric metadata and lockfile. The wire protocol version is independent and should change only for an actual protocol compatibility requirement.

See `docs/RELEASE_GATES.md` for the required executable evidence and signing/packaging expectations.
