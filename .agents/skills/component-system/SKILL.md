---
name: component-system
description: Defines reusable CSS tokens and UI component patterns for the SwarmCraft desktop frontend. Use when adding or refactoring buttons, fields, badges, navigation, panels, lists, dialogs, status displays, spacing, typography, or repeated HTML/CSS patterns.
---

# SwarmCraft Component System

Build a small, coherent component system inside the existing plain HTML/CSS/JS frontend. The goal is consistency without importing a framework.

## Start with tokens

Use CSS custom properties in `apps/desktop/src/style.css` for repeated values. Prefer semantic names over raw color names.

Keep the token set intentionally small. Typical groups:

- surfaces: app background, raised surface, inset surface, hover/selected surface
- borders: subtle, strong, focus
- text: primary, secondary, muted
- semantic status: safe, warning, danger
- accent/action
- spacing: 4, 8, 12, 16, 24, 32px
- radius: preferably 2 or 3 sizes total
- control heights
- focus ring

Do not create a token for every one-off value. Tokens should represent decisions that recur.

## Geometry

Use consistent geometry across the app.

- ordinary controls should share height, padding, and radius
- panels should share a small set of radii
- metadata tags should not automatically be pill-shaped
- nested surfaces should get progressively quieter, not progressively more decorated
- use shadows sparingly; borders/surface contrast should do most separation

If two components serve the same role but look unrelated, normalize them.

## Buttons

Use a small hierarchy:

1. Primary: the recommended action in the current task region.
2. Secondary: common but lower-priority actions.
3. Ghost/quiet: low-emphasis utility actions where appropriate.
4. Destructive: actions with damaging or membership/runtime consequences.

Rules:

- avoid multiple primary buttons in one local action group
- destructive color communicates consequence, not importance
- icon-only buttons require an accessible name; avoid them unless the icon is unambiguous
- disabled buttons must look disabled and remain readable
- loading buttons should preserve width where possible and prevent duplicate submission

## Fields

All editable controls need a visible label.

Create consistent states for:

- default
- hover
- keyboard focus
- invalid/error
- disabled/read-only

Helper text should sit close to the field it explains. Error text should state what the user can do next.

Machine values such as world IDs, multiaddresses, executable paths, hashes, and logs may use monospace. Normal names and descriptions should not.

## Status indicators

Status is part of SwarmCraft's safety model, so components must encode meaning consistently.

Use semantic variants for:

- healthy/canonical/safe
- warning/degraded/solo
- conflict/danger
- neutral/unknown/not initialized

Color alone is insufficient. Include readable text and, when adding icons, a distinct shape/icon where useful.

Do not turn every metadata field into a badge. Badges are for state, category, or compact exceptional metadata.

## World list item

Treat the world entry as a first-class component. It should have predictable regions:

- identity: world name first, world ID secondary
- safety/status
- replication/authority summary
- compatibility/checkpoint metadata
- local actions, usually one primary plus quiet details

Selection must be visually obvious and keyboard accessible. Do not depend only on hover.

If the whole row/card is interactive, use correct semantics or explicit keyboard handling and a visible focus style. Avoid nested click behavior that produces accidental selection/action conflicts.

## Panels and sections

Not every section needs a card.

Use a bordered/raised panel only when it creates a meaningful boundary, such as a task workspace, selected-world detail, modal/dialog, or advanced settings region.

Prefer headings, separators, aligned grids, or background changes for lighter grouping.

Avoid card-inside-card-inside-card structures.

## Dialogs and confirmations

When a task introduces confirmations or focused multi-step flows, prefer a real dialog pattern rather than expanding another permanent card into the page.

Dialogs need:

- clear title
- concise consequence/context
- sensible initial focus
- keyboard escape behavior when cancellation is safe
- focus containment/restoration when implemented as modal
- primary and cancel actions in stable positions

Destructive confirmation text should name the affected world/action.

## Navigation

If the app grows beyond the current single-page structure, prefer compact desktop navigation such as a sidebar, segmented task switcher, or clear selected-world workspace.

Navigation is not a collection of giant buttons. The selected location must remain visually obvious without relying on accent color alone.

## Empty, loading, and error states

Every reusable data region should consider:

- empty: explain why it is empty and give the next useful action
- loading: show which region is busy without blanking unrelated UI
- error: preserve context and offer retry/recovery where possible
- stale/degraded: distinguish from hard failure when the protocol still has usable local state

Avoid generic `Something went wrong` when a specific actionable message is available.

## CSS discipline

Prefer class-based reusable styles over element-specific chains and IDs for appearance. IDs exist primarily for JavaScript bindings.

Avoid `!important` unless resolving a documented exceptional integration problem.

When adding a new component:

1. check whether an existing pattern already serves the role
2. reuse tokens
3. add all relevant interaction states together
4. test long text and narrow windows
5. remove obsolete styles after migration

## Accessibility baseline

Components must provide:

- visible keyboard focus
- sufficient contrast
- usable hit targets
- semantic labels/names
- no information conveyed by color alone
- no hover-only essential actions

Native HTML controls and semantics are preferred when they solve the problem cleanly.
