# Agent 1 — Minecraft + Fabric catalog

## Recovery status

`INTEGRATED — FORMAT-ONLY LIVE TAIL SUPERSEDED`

- Branch: `agent/minecraft-fabric-catalog`
- Live head: `a581195bfff3fd3a050e1978910fe77288237cbc`
- Merge base with final integration: `b7128c83b83208d7c1d8a82df915766fc7abb3ec`
- The sole Agent 1 commit after that merge base is bot-authored `style(agent1): satisfy Desktop rustfmt`. It changes only Desktop formatting; it contains no catalog behavior.
- The final integration branch subsequently received Rustfmt over the fully integrated Desktop tree, so that old formatting-only tail is intentionally not cherry-picked.

## Integrated contract

- Official Mojang version catalog.
- Fabric Meta loader resolution constrained by selected Minecraft version.
- Authoritative Desktop selectors; invalid/loading/out-of-order selections fail closed.
- Canonical world creation revalidates the selected Minecraft/Fabric pair in Rust rather than trusting browser text.

## Validation evidence

On exact integration head `1cca925b44a51aef019f31ada77aaca88fcf4177`, dedicated Agent 1 validation passed workspace format, locked metadata, workspace check, strict Clippy, workspace tests, deterministic catalog tests, and live Mojang/Fabric source validation before entering Desktop sidecar/stage checks. The previous Desktop rustfmt-only failure was repaired by the repository runner at `9c7816dea8371cfab34203acbd0c8a492894e785`.

Final acceptance is owned by Agent 8 on `integration/player-launcher-v1`; no further Agent 1 feature work is pending.
