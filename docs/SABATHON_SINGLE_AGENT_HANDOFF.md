# SwarmCraft Sabathon P0 — Single-agent continuation

> Historical consolidation snapshot. The later integrated host/remote proof,
> localhost gateway, native Desktop status and locally verified results are
> recorded in [`SABATHON_P0_LIVE_INTEGRATION_STATUS.md`](SABATHON_P0_LIVE_INTEGRATION_STATUS.md).
> **P0 remains NOT CERTIFIED** until real three-machine Minecraft acceptance.

**Consolidated branch:** `integration/sabathon-p0-20261123`

This branch contains the original runtime + authenticated game-route baseline
and all four completed Sabathon agent branches. Work **only on this branch**
from now on. Do not continue independently on the old agent branches and do
not merge to `main` without explicit approval. Retain the old branches as
recovery history; their worktrees do not need to be deleted.

## Integrated history

| Workstream | Branch | Merge commit |
| --- | --- | --- |
| Game routing baseline | `agent/sabathon-game-route-20261123` | Already in base `c8362fe` |
| Runtime/plan baseline | `agent/sabathon-runtime-20261123` | Already in base `c8362fe` |
| Safe server runtime | `agent/sabathon-server-20261123` | `a802ddc` |
| Live authority verification | `agent/sabathon-authority-20261123` | `9a966bd` |
| Daemon transfer + fresh quorum | `agent/sabathon-daemon-20261123` | `66a8318` |
| Desktop UX + field QA | `agent/sabathon-desktop-20261123` | `bec3c23` |

The merges were normal non-fast-forward merges, with no conflicting file
resolution or cherry-pick overwrites. All six source branches are ancestors
of the consolidated branch. The Git worktree was clean after merging.

## Current truth — still NOT a working P0 game route

The source components are present, but **do not claim production authorization
or working three-machine gameplay from a successful merge or unit tests**:

1. The daemon in `crates/swarm-cli/src/daemon.rs` intentionally sends
   deny-only game-route responses. It has an authenticated fresh-quorum
   collector in `daemon_live_quorum.rs`, but the real route proof provider
   has not been connected. Do not replace the refusal with a cached
   `migration Ready`, discovery state or a fabricated positive proof.
2. Agent 1's `live_authority.rs` and `live_fabric_readiness.rs` are
   standalone verified adapter modules. The ordinary `swarm-cli/src/lib.rs`
   has not yet registered them. The authority test compiles them through a
   scoped test entrypoint. Integrate the modules deliberately with the
   daemon and keep all quorum, world, checkpoint, consent and fencing checks.
3. The Java Fabric bridge implements a nonce-bound runtime challenge, but the
   Rust `swarm-ipc/src/transport.rs::FabricSession` still needs a bounded
   challenge method on the **same live authenticated socket**. Pair it with
   managed process identity, private loopback listener and a current permit.
   Keep failures, expiry, malformed replies and process replacement fail-closed.
4. Desktop `game_route_commands.rs` intentionally refuses a trustworthy
   route until the backend exposes real, current proof. Do not manufacture
   a green route or reuse `game_endpoint` as an authenticated connection.
5. The signed staged transfer, safe old-host fencing and gateway reconnect
   still require **combined local integration testing and real client field
   acceptance**. The standalone agent test results are not a substitute.

**Next agent's focus:** Connect one genuinely live, nonce-bound
`FreshAuthoritySource` and daemon route responder, complete live Fabric IPC
request/response lifecycle, and expose a trusted read-only route status to the
Desktop. Then verify the real game stream/gateway, safe transfer/recovery and
crash/withdrawal behavior without weakening any denial path. Treat
`crates/swarm-cli/src/LIVE_AUTHORITY_INTEGRATION_HANDOFF.md` and
`crates/swarm-cli/DAEMON_STAGED_TRANSFER_HANDOFF.md` as implementation
contracts, not declarations of completion.

## Verification at consolidation

- `git merge-base --is-ancestor` confirmed all six branches are included.
- `git diff --check c8362fe HEAD` passed.
- `cargo fmt --all -- --check` passed.
- `node --test apps/desktop/tests/game-route-status.test.mjs apps/desktop/tests/host-readiness-proof.test.mjs apps/desktop/tests/transfer-wizard-races.test.mjs apps/desktop/tests/client-continuity.test.mjs apps/desktop/tests/manual-transfer.test.mjs`: **34/34 passed**.
- `PYTHONPATH="$PWD/scripts/e2e/three_machine" python3 -m unittest discover -s scripts/e2e/three_machine -p 'test_*.py'`: **112/112 passed**.
- No real three-machine Minecraft field verdict, installed-client routing
  evidence, or release certification was claimed at merge.

## Operating rules for the sole continuing agent

- Commit and push each verified increment to
  `integration/sabathon-p0-20261123`; do not work on old agent branches.
- Run Cargo builds/tests using
  `flock -x /tmp/swarmcraft-sabathon-cargo.lock`, with
  `CARGO_BUILD_JOBS=2` and `--jobs 2`. Do not overlap Rust builds with
  another local process. Build caches were recently cleaned, so a cold build
  may take time and disk. Confirm available space before starting.
- Preserve the exact signed quorum and membership, authority
  epoch/fence, hosting consent, process/runtime readiness, private Minecraft
  listener policy and fail-closed route checks.
- Re-run focused Rust tests and actual source checks on the consolidated
  commit; then run a genuine three-peer installed-client field campaign.
  Keep missing hardware / live WAN evidence **BLOCKED**, not passed.
- Keep `main`, unrelated workspaces, test data and the old branches intact.

Refer to `docs/SABATHON_IMPLEMENTATION_PLAN.md` and
`scripts/e2e/three_machine/SABATHON_DESKTOP_FIELD_RUNBOOK.md` for the
full acceptance definitions.
