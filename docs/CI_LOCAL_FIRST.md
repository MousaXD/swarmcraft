# Local-first CI and GitHub Actions policy

Effective 2026-11-25. All development work runs locally, including formatting,
lockfile changes, lint, tests, integration/E2E, and builds. Batch and verify
changes before a milestone push. Never use hosted Actions for temporary
agent work or live third-party probes. No schedules or cron. Intermediate
pushes require a good reason and should use `[skip ci]` when appropriate;
skipped required statuses are not proof of validation.

## Reviewed workflow inventory

| Workflow | Trigger | Purpose |
| --- | --- | --- |
| ci.yml | push main; PRs targeting main, non-doc changes | One static policy/version metadata job |
| required-validation.yml | workflow_dispatch; workflow_call from manual release pipelines | Explicit final Windows/macOS verification after exact-SHA local attestation |
| main-installers.yml | workflow_dispatch only | Explicit main preview packaging/publication |
| release.yml | workflow_dispatch only | Explicit version-tag production packaging/publication |
| three-machine-certification.yml | workflow_dispatch only | Optional dedicated local field-lab preflight (not completed real E2E) |

Do not dispatch final checks or releases without approval. Linux/Fabric
release builders and the field-lab preflight are restricted to explicitly
approved, isolated self-hosted runners with the `swarmcraft-release` or
`swarmcraft-certification` label respectively. These runners are not created
or registered by the workflow cleanup. Do not run untrusted public PR code
on any such runner or use a personal workstation as a shared public runner.
All routine validation is directly via the local shell, not self-hosted
Actions. Prefer direct shell execution for field-lab tests too.

## Local verification

From the repository root, run the relevant checks and retain output with
the exact commit. Coordinate heavy builds with other local agents; use the
existing Cargo lock at `/tmp/swarmcraft-sabathon-cargo.lock` if shared.
For a release, execute all applicable prior gates, not just these examples.
Never attest unavailable, failed or skipped checks as passing.

```bash
python3 scripts/check_workflow_policy.py
python3 scripts/check_release_version.py
python3 -m unittest discover -s scripts/ci-release -p 'test_*.py'
python3 -m unittest discover -s scripts/ci-artifacts -p 'test_*.py'
cargo fmt --all -- --check
cargo metadata --locked --format-version 1 >/dev/null
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
cargo build --workspace --locked

cargo fmt --manifest-path apps/desktop/src-tauri/Cargo.toml -- --check
cargo metadata --manifest-path apps/desktop/src-tauri/Cargo.toml --locked --format-version 1 >/dev/null
cargo clippy --manifest-path apps/desktop/src-tauri/Cargo.toml --all-targets --all-features --locked -- -D warnings
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --all-features --locked
cargo clippy --manifest-path crates/swarm-provider/Cargo.toml --all-targets --all-features -- -D warnings
cargo test --manifest-path crates/swarm-provider/Cargo.toml --all-features
cargo audit --file Cargo.lock
cargo audit --file apps/desktop/src-tauri/Cargo.lock

(cd apps/desktop/frontend && npm ci && npm run check && npm test && npm run test:source && npm run build)
node --test apps/desktop/tests/*.test.mjs
(cd minecraft/fabric && gradle build)
python3 -m unittest discover -s scripts/e2e/three_machine -p 'test_*.py'
```

The Desktop native checks require the four packaged Rust sidecars staged
with the correct target suffix. Build and stage them locally using the
existing package conventions. Use locked dependencies; generate any needed
lockfile update locally and review its diff before committing.

For process-level acceptance, run the actual integration tests, including
private infrastructure smoke, network reconnect/impairment and multi-GiB
soak, delegated cgroup enforcement, the required non-skipped hung shutdown
barrier, and real installed GUI negative/positive controls. Run fuzzing
locally (`cargo +nightly fuzz`) and inspect real Fabric artifacts.
Use isolated test networking for any impairment (never change the host's
active network settings just to satisfy a test). The pre-policy workflows
at commit `49ca1f5` preserve the exact original gate recipes for reference:

```bash
git show 49ca1f5:.github/workflows/ci.yml
git show 49ca1f5:.github/workflows/network-soak.yml
git show 49ca1f5:.github/workflows/fuzz.yml
git show 49ca1f5:.github/workflows/specialist-validation.yml
git show 49ca1f5:.github/workflows/governance-regressions.yml
git show 49ca1f5:.github/workflows/player-journey-live.yml
```

These are historical recipes, not permission to restore workflows. Run
their applicable commands in a suitable disposable local environment.
Live catalog/provider and managed Minecraft acquisition/launch acceptance
must run locally, with any necessary explicit authorization and real
prerequisites. Never move live third-party probes back to hosted Actions.
Use the three-machine README for real lab setup, capture, and review.

## Final validation and publication

Once the exact commit has passed local validation, request permission for
the minimum necessary native final checks. The manual final-validation
workflow takes `local_validation_sha` and refuses a mismatch. A matching
input is an operator attestation, not cryptographic evidence of test
completion. Keep the actual logs. Existing signing, verified artifacts,
and publication dependencies remain required. Preview/production releases
now need explicit dispatch; a push or tag no longer publishes.

The original protected status `Required validation gate` remains separate
from the cheap `PR/main metadata gate`. Existing branch rules may require
the manual final result for a merge, and dispatch availability depends on
the default branch's workflow definitions. Review effective repository
rules before rollout. Do not change branch rules or bypass checks without
explicit approval. A docs-only change can intentionally skip metadata CI;
it does not automatically waive an existing required status.

Historical evidence, logs and workflows can be inspected in git history.
No old branch or workflow run may be deleted remotely without approval.
