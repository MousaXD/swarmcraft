# Final Preview Verification Marker

This marker exists to exercise the permanent pull-request CI matrix after temporary or one-off validation work has completed.

A preview merge is allowed only when the final branch commit passes the permanent gates documented in `docs/RELEASE_GATES.md`, including cross-platform Rust checks, process-level daemon/recovery acceptance tests, Fabric, RustSec and native desktop packaging.

This file is version-neutral on purpose. Historical release-specific evidence belongs in release notes or `docs/RELEASE_GATES.md`, not in the permanent marker.
