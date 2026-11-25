# Release trigger (local-first)

Neither a push to `main` nor a version tag automatically builds or publishes
installers. The only automatic workflow is the small PR/main metadata check.

1. Run the complete applicable local validation described in
   [CI_LOCAL_FIRST.md](CI_LOCAL_FIRST.md) and [RELEASE_GATES.md](RELEASE_GATES.md)
   against the exact intended commit. Keep its logs and SHA.
2. Obtain explicit approval for any final native GitHub Actions verification.
3. For a preview, manually dispatch `main-installers.yml` on `main`.
   For a production release, manually dispatch `release.yml` on the exact
   version tag. In either case provide the already locally validated SHA
   in `local_validation_sha`. A tag push alone must not publish.
4. Final native verification, signed production credentials, platform
   artifact checks, checksums and publication dependencies remain mandatory.

Linux/Fabric packaging in the manual publication pipeline requires an isolated
operator-approved local release runner. Windows/macOS use native runners.
No self-hosted runner is provisioned by this policy change; no release should
be dispatched until the approved environment is ready.
