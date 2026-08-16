---
name: desktop-app-ux
description: Guides native-feeling desktop UX for SwarmCraft's Tauri application. Use when changing navigation, window layout, workflows, keyboard behavior, menus, dialogs, activity/status presentation, advanced settings, or interactions that should feel like a desktop app rather than a webpage.
---

# SwarmCraft Desktop App UX

SwarmCraft runs in a resizable Tauri desktop window. Design interactions for sustained desktop use, not for a scrolling web landing page.

## Window reality

The configured window is:

- default `980x760`
- minimum `720x560`
- resizable

Treat both sizes as first-class. The minimum size must remain functional without horizontal scrolling or unreachable primary actions.

Do not assume a maximized monitor or browser viewport.

## App shell over webpage

Prefer a stable application workspace with clear regions. The user should not need to scroll through unrelated setup forms to reach their selected world's primary controls.

As the UI evolves, favor patterns such as:

- compact sidebar/world switcher plus selected-world workspace
- task-oriented create/join dialogs or focused panels
- collapsible advanced settings/diagnostics
- activity/status region that can be inspected without permanently consuming a large fraction of the window

Avoid a marketing-style hero, stacked feature cards, and full-page narrative flow.

## Task priority

Everyday tasks should dominate:

- pick a world
- see safety/replication state
- play
- understand degraded/conflicted state
- create or join when needed

Advanced tasks should be available but visually quieter:

- daemon multiaddress configuration
- runtime executable/jar paths
- verification/recovery/export
- raw identity/protocol data

Do not make advanced diagnostics look equally important to Play or safety state.

## Create and Join

Treat Create World and Join World as user-initiated workflows rather than permanent dashboard furniture.

A good flow:

1. user explicitly starts Create or Join
2. focused form appears
3. validate required fields near the input
4. disable/prevent duplicate submission while working
5. report success/failure in context
6. on success, select/open the affected world

Keep expert fields available when needed, but default paths should be short and understandable.

## Selected world behavior

Selection should be persistent and obvious. When a user selects a world, avoid surprising automatic scrolling if the app can instead update a stable workspace.

Primary world actions should remain near the world status they affect.

If a world is conflicted or unsafe, adjust the action hierarchy. For example, conflict resolution may need to outrank Play depending on protocol safety semantics.

Do not silently map a visually simple action to a surprising advanced requirement. If Play requires runtime configuration, communicate that before bouncing focus to an unrelated field.

## Keyboard behavior

Desktop users should be able to navigate efficiently.

At minimum:

- all interactive controls are reachable by Tab
- focus order follows visual/task order
- focus is visibly distinct
- Enter/Space semantics match the control type
- dialogs restore focus to the invoking control when closed
- Escape closes cancellable dialogs/overlays when safe

Do not create custom keyboard behavior that conflicts with native text editing or platform conventions.

If adding shortcuts, make them discoverable and avoid taking common OS/app shortcuts without strong reason.

## Menus and contextual actions

When the action count grows, do not keep adding buttons to horizontal rows.

Use progressive disclosure for low-frequency actions, such as an overflow/context menu, details panel, or advanced section. Keep destructive actions discoverable but separated from routine actions.

Do not hide safety-critical status or recovery paths behind hover-only controls.

## Status and activity

The current Activity console is useful for raw command output but should not become the primary feedback mechanism for every action.

Prefer local feedback:

- field validation next to fields
- task progress near the triggering action
- world status updates in the world workspace
- non-blocking global status for background work

Raw logs and protocol output belong in an inspectable activity/diagnostics surface.

Avoid clearing useful context when a new command runs unless the user is clearly entering a new task.

## Loading and concurrency

Tauri commands can take time. For user-triggered async work:

- identify the busy region/action
- prevent accidental duplicate destructive submissions
- keep unrelated areas usable when safe
- preserve the previous meaningful state while refreshing data
- distinguish refreshing from empty

Do not flash the whole interface into a generic loading screen for a local operation.

## Errors

Desktop errors should be actionable.

Prefer messages that identify:

- what action failed
- which world/resource was affected when relevant
- whether local data remains safe/usable
- what the user can try next

Technical details can be expandable/copyable rather than dumped into the primary interface.

## Destructive actions

Leaving a world, stopping runtime/daemon services, discarding/conflict decisions, and destructive recovery operations deserve consequence-aware interaction.

Use confirmation when an action is difficult to reverse or meaningfully changes membership/data/runtime state. The confirmation should name the affected world or service.

Do not use danger styling for ordinary Stop buttons simply because they are red in the current UI; decide whether the action is actually destructive or just a routine state transition.

## Platform feel

Use system fonts, native controls where appropriate, normal text selection, standard cursors, and predictable focus behavior.

Avoid browser-site affordances that feel alien in an installed app, such as oversized CTA buttons, landing-page heroes, decorative scroll journeys, or link-like controls for core app actions.

Do not fake platform chrome. Let Tauri/OS own the actual window unless a specific custom titlebar task requires otherwise.

## Copy

Prefer concise operational language.

Good labels are verbs or concrete nouns: `Play`, `Create world`, `Join world`, `Replicas`, `Verify storage`.

Helper text should explain consequences, safety, or uncommon technical concepts. It should not narrate obvious UI.

Keep protocol terms when they carry real meaning, but pair them with plain-language context where a player could otherwise misunderstand them.

## Completion check

Before finishing a desktop UX change, verify:

- the main task is apparent within a few seconds
- the selected world and safety state are always clear
- advanced controls do not crowd routine play
- focus/keyboard behavior is predictable
- long tasks and failures provide local feedback
- destructive actions cannot be triggered casually
- default and minimum window sizes are genuinely usable
