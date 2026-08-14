# Final v0.1.0-preview verification

This marker is intentionally small. It triggers the permanent pull-request CI matrix after temporary release-soak workflows have completed and been removed.

A merge is allowed only when the branch passes all permanent gates documented in `docs/RELEASE_GATES.md`, including cross-platform Rust checks, real loopback daemon acceptance tests, Fabric, RustSec, and native desktop packages.
