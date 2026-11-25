# Final Preview Verification Marker

This historical marker describes the permanent release gate. The current integration branch uses the [local-first policy](CI_LOCAL_FIRST.md); do not dispatch hosted validation without explicit approval.

A preview release requires exact-source local evidence plus approved final native validation under [RELEASE_GATES.md](RELEASE_GATES.md), including cross-platform Rust checks, process-level recovery acceptance, Fabric, RustSec and native packaging. A lightweight PR metadata check or historical green run is not this proof.

This file is version-neutral on purpose. Historical release-specific evidence belongs in release notes or `docs/RELEASE_GATES.md`, not in the permanent marker.
