# SwarmCraft Agent Progress

This directory is the shared coordination ledger for the player-launcher work.

Every implementation agent owns exactly one progress file:

- `agent1.md` — Minecraft + Fabric Catalog
- `agent2.md` — Modrinth Integration
- `agent3.md` — CurseForge Integration
- `agent4.md` — Canonical Modpack + Import
- `agent5.md` — Invite + Internet Bootstrap
- `agent6.md` — Friend + Public Discovery
- `agent7.md` — Player Setup + Migration UX
- `agent8.md` — Integration + Acceptance

## Mandatory protocol

1. Before changing code, read this file and your own `progress/agentN.md`.
2. If your file lists upstream dependencies, read every required upstream `progress/agentN.md` before designing or changing a shared contract.
3. Record the exact upstream commit SHA(s) you consumed under **Dependencies consumed**. Do not rely on memory, chat summaries, or assumptions about another branch.
4. Do not invent a second competing contract when an upstream agent already owns that contract. If the upstream contract is insufficient, record the requested change in your own log and coordinate through the integration agent.
5. Update your own progress file after every meaningful implementation milestone and before every handoff/PR. Keep it accurate enough that another agent can resume from the repository alone.
6. Never edit another implementation agent's progress file to make their work appear complete. Agent 8 may add integration observations, but the original implementation record stays owned by its agent.
7. `DONE` means implementation plus owned tests are green on the exact recorded head. Anything less is `IN PROGRESS`, `BLOCKED`, or `READY FOR INTEGRATION`.
8. Every contract/API/schema change must be listed explicitly, including JSON shapes, Rust types, Tauri commands, protocol fields, environment variables, source/provider rules, and migration behavior.
9. Every progress file must list tests actually executed and their result. Do not write “tests pass” without naming them.
10. Do not merge feature branches directly into `main`. Agent 8 owns final integration into the designated integration branch unless an explicit later instruction changes that rule.

## Required status values

Use one of:

- `NOT STARTED`
- `IN PROGRESS`
- `BLOCKED`
- `READY FOR INTEGRATION`
- `INTEGRATING`
- `DONE`

## Required progress-file sections

Each agent file must maintain these sections:

- **Status**
- **Branch / exact head**
- **Mission**
- **Dependencies to read**
- **Dependencies consumed**
- **Work completed**
- **Contracts / APIs added or changed**
- **Files changed**
- **Tests and evidence**
- **Decisions / invariants**
- **Known issues / blockers**
- **Handoff for dependent agents**
- **Activity log**

The activity log should be append-only. Put newest entries at the bottom and include the date, commit SHA, and a compact summary.

## Dependency graph

```text
Agent 1  Minecraft/Fabric Catalog ───────────────┐
Agent 2  Modrinth ───────────────────────────────┤
Agent 3  CurseForge ─────────────────────────────┤
                                                 ├─> Agent 4 Canonical Modpack + Import ─┐
Agent 5  Invite + Internet Bootstrap ────────────┤                                      │
                                                 └─> Agent 6 Discovery ──────────────────┤
                                                                                        ├─> Agent 7 Player Journey
Agents 1,2,3,4,5,6 ─────────────────────────────────────────────────────────────────────┘

Agents 1–7 ─────────────────────────────────────────────────────────────────────────────> Agent 8 Integration + Acceptance
```

Agent 4 must consume the provider/runtime contracts from Agents 1–3 before freezing canonical modpack identity.

Agent 6 must consume Agent 5's connectivity/invite advertisement contract before publishing discovery records.

Agent 7 must consume Agents 1–6 before wiring the final create/join/setup/reconnect experience.

Agent 8 must read every progress file and integrate only exact recorded heads that are ready for integration.

## Shared safety rules

- Minecraft/Fabric/runtime compatibility belongs in backend-owned contracts, not duplicated JavaScript rules.
- Canonical world requirements must pin exact artifact identity and hashes. Never sign “latest”.
- Provider licensing/redistribution rules must be respected. Do not peer-to-peer redistribute arbitrary Modrinth/CurseForge JARs unless the provider/project terms explicitly permit it.
- A missing or unavailable required artifact must fail closed with a remediation path rather than silently substituting another version.
- Discovery never grants membership. Signed canonical membership remains authoritative.
- Connectivity/relay reachability never grants authority.
- Runtime/mod readiness and Host Readiness remain backend decisions.
- Host migration must preserve fencing, canonical snapshot restore, and runtime verification. Reconnection UX must not bypass authority safety.
