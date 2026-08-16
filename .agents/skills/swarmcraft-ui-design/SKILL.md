---
name: swarmcraft-ui-design
description: Guides visual design and frontend UX for the SwarmCraft desktop app. Use for UI redesigns, layout changes, styling, new screens, HTML/CSS work, dashboard improvements, visual polish, or when the interface feels generic, vibe-coded, cluttered, or inconsistent.
---

# SwarmCraft UI Design

Design SwarmCraft as a serious desktop tool for managing replicated Minecraft worlds, not as a marketing landing page or generic AI-generated SaaS dashboard.

## Read before editing

Inspect these files first:

- `apps/desktop/src/index.html`
- `apps/desktop/src/style.css`
- `apps/desktop/src/app.js`
- `apps/desktop/src-tauri/tauri.conf.json`

Understand the existing information and command wiring before changing visual structure. Preserve element IDs used by `app.js` unless the JavaScript is updated in the same change.

## Product character

SwarmCraft should feel:

- compact, calm, technical, and trustworthy
- native enough to belong in a desktop window
- legible at a glance during normal play/admin work
- restrained rather than decorative
- Minecraft-adjacent without imitating Minecraft's visual assets or turning the app into a themed novelty UI

Use visual hierarchy to communicate safety and action priority. The user should quickly answer:

1. Which world am I looking at?
2. Is it safe/canonical, degraded, or conflicted?
3. What replicas or authority state matter right now?
4. What is the next useful action?

## Anti-vibe-coded rules

Actively remove patterns that make the interface look auto-generated or template-driven.

Avoid:

- giant landing-page headings inside a utility app
- decorative hero sections that consume useful vertical space
- radial-gradient wallpaper or glow effects as the main visual identity
- putting every section inside a large floating card
- excessive rounded rectangles and mixed corner radii
- pill-shaped containers for ordinary metadata
- gratuitous gradients, glassmorphism, blur, or shadows
- every action being visually loud
- repeated eyebrow labels that add little information
- huge empty gaps between related controls
- centering content that should align to a working grid
- arbitrary one-off CSS values when an existing token can be reused
- fake sophistication through tiny uppercase text, excessive tracking, or muted-on-muted contrast
- decorative animations; motion must communicate state or spatial change

The current frontend contains several of these patterns. Improve them incrementally when touching the relevant area rather than preserving them simply because they already exist.

## Information architecture

Prefer a desktop application shell over a single long webpage.

Recommended hierarchy:

- persistent or clearly anchored world/navigation area
- main content focused on the selected world or current task
- secondary/advanced controls separated from everyday actions
- activity/status output available without dominating the viewport

Create and Join are tasks, not permanent equal-weight dashboard columns. Diagnostics are advanced controls and should not compete visually with Play, world safety, or replication state.

Keep dangerous operations such as leaving a world, stopping authority runtime, or conflict resolution visually distinct but not constantly screaming for attention.

## Density and spacing

Optimize primarily for the configured desktop sizes:

- default: `980x760`
- minimum: `720x560`

Use a consistent spacing scale. Prefer a small set such as 4, 8, 12, 16, 24, and 32px rather than unrelated values.

Use whitespace to group information, not to make the product look spacious. A desktop utility can be comfortably dense.

Keep controls near the data they affect. Avoid forcing users to scroll past setup forms before reaching the selected-world actions.

## Typography

Use the system UI font stack unless the project deliberately adds a bundled font later. Do not fetch remote fonts.

Use approximately three text roles:

- page/section title
- normal body/control text
- compact metadata/helper text

Use monospace only for machine identifiers, hashes, addresses, paths, logs, or protocol values.

Do not rely on tiny text or low contrast to create hierarchy.

## Color and surfaces

Dark mode is currently the product default. Keep it readable and restrained.

Use a small semantic palette:

- neutral surface/text
- primary action/accent
- safe/canonical
- warning/degraded/solo
- danger/conflict/destructive

Status colors are semantic. Do not use green everywhere just because SwarmCraft currently has a green theme. Neutral UI chrome should stay neutral so safety colors retain meaning.

Prefer borders and subtle surface contrast over large shadows. Avoid nesting multiple bordered cards unless the nesting has real information hierarchy.

## Controls

Buttons need clear priority:

- one primary action per local task region when possible
- secondary actions quieter
- destructive actions separated spatially or by confirmation

Do not use pills for standard buttons unless the component truly represents a compact toggle/filter/chip.

Inputs need visible labels. Placeholder text is supplemental, never the only label.

Keep technical defaults editable when they are genuinely user-facing, but advanced values such as multiaddresses, executable paths, and server jars belong in advanced configuration rather than the main flow.

## World cards and status

A world list should optimize scanning. Each item should emphasize:

- world name
- safety state
- Minecraft/version compatibility when relevant
- replication/authority summary
- recent/checkpoint state

World IDs are secondary machine data. Truncate them visually but keep a way to inspect/copy the full value when implementing richer interactions.

Do not represent every metadata field as a pill. Use compact rows, aligned metadata, or a small status cluster.

## State semantics

Never beautify away protocol distinctions.

- `canonical/quorum/replicated` should read as healthy/safe
- `solo` should read as degraded or cautionary, not equivalent to healthy
- `conflict` must be unmistakable and offer a clear resolution path
- storage-only incompatible peers must not look authority-capable
- discovery must not imply membership
- relay connectivity must not imply canonical authority

When simplifying copy, keep these meanings intact.

## Responsive desktop behavior

Treat `720x560` as a real supported desktop window, not a mobile breakpoint.

At narrow widths:

- reduce columns before shrinking readable text
- keep essential actions visible
- allow advanced sections to stack or collapse
- prevent horizontal scrolling
- preserve usable hit targets

Do not turn the desktop app into a phone layout unless a task explicitly adds mobile support.

## Implementation discipline

The current frontend is plain HTML, CSS, and JavaScript. Prefer improving that system instead of adding React/Vue/Svelte/Tailwind or a component framework for cosmetic reasons.

Before inventing a new style, ask whether an existing token/component can express it. If repeated visual values appear while implementing a change, consolidate them into CSS custom properties.

Separate structural improvements from gratuitous rewrites. Keep DOM changes understandable and preserve Tauri event/command bindings.

## Completion criteria

A UI change is not complete when it merely looks different. It is complete when it:

- improves information hierarchy or task flow
- uses the shared visual system
- remains usable at default and minimum window sizes
- preserves protocol/safety meaning
- has keyboard-visible focus and readable contrast
- handles empty, long-content, loading, error, and destructive states as relevant
- passes `frontend-quality-gate`
- has been rendered and reviewed with `visual-review` when rendering is available
