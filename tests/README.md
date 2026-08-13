# Integration test suites

Unit tests live beside the crates they validate. These directories are reserved for the multi-process
peer harness required by the preview plan:

- `integration/` peer join/replication and cross-platform scenarios
- `recovery/` crash restore, corruption, and stale-authority scenarios
- `partition/` split-brain and election scenarios
- `fixtures/` deterministic world-directory fixtures

The harness should not depend on Minecraft until the protocol simulator is reliable.
