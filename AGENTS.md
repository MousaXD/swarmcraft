# SwarmCraft agent guidance

These instructions apply to the entire repository.

## Desktop frontend

SwarmCraft's desktop UI is a Tauri 2 application with a deliberately small frontend stack:

- `apps/desktop/src/index.html` — semantic UI structure
- `apps/desktop/src/style.css` — all visual styling
- `apps/desktop/src/app.js` — DOM behavior and Tauri command wiring
- `apps/desktop/src-tauri/` — Rust/Tauri desktop shell

Do not introduce a JavaScript framework, CSS framework, icon pack, font dependency, or build pipeline merely to restyle the UI. Prefer the existing HTML/CSS/JS stack unless the task explicitly requires an architectural change.

For work that creates, changes, reviews, or critiques desktop UI, use the project skills under `.agents/skills/` as applicable:

- `swarmcraft-ui-design` — visual language, hierarchy, density, and anti-vibe-coded rules
- `component-system` — reusable tokens and component patterns
- `desktop-app-ux` — native-feeling desktop interaction and window behavior
- `frontend-quality-gate` — accessibility, states, sizing, and implementation checks
- `visual-review` — screenshot-driven review and iteration

For meaningful frontend changes, `swarmcraft-ui-design` and `frontend-quality-gate` are expected. Use `visual-review` before declaring a visual task complete whenever the environment can render the app or static frontend.

## Product constraints

Preserve SwarmCraft's protocol and safety semantics. Visual simplification must not blur distinctions such as canonical vs. solo/conflicted state, authority vs. replica, membership vs. discovery, or compatible vs. storage-only peers.

Prefer clear, compact desktop information architecture over landing-page presentation. The primary job of the UI is to help a player understand worlds, safety, replication, and the next useful action quickly.

## Validation

When frontend files change:

1. Check the UI at the configured default window size (`980x760`) and minimum window size (`720x560`).
2. Check keyboard focus, overflow, loading/error/empty/disabled states, and long world IDs/text.
3. Keep Tauri command names and payload shapes intact unless the task explicitly changes the Rust API.
4. Run relevant repository checks. For the desktop Rust shell, use `cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml` when the environment has the required toolchain/dependencies.
5. Do not claim visual completion from source inspection alone when rendering is available.
