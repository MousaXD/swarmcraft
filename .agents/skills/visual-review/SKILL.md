---
name: visual-review
description: Performs screenshot-driven visual QA for the SwarmCraft desktop frontend. Use after styling, layout, component, navigation, responsive, or other visible UI changes, and when asked to critique or improve how the app looks.
---

# SwarmCraft Visual Review

Visual work must be reviewed visually when the environment can render the frontend. Do not decide that a redesign is good from CSS source alone.

## Render the actual interface

Prefer the real Tauri app when the environment supports it. If the Tauri dev tooling is unavailable and the task is purely visual, the static frontend can still be inspected by serving `apps/desktop/src`, for example with a simple local HTTP server. Clearly distinguish static visual review from full Tauri interaction testing.

Do not edit production behavior to inject fake demo data merely for screenshots.

## Required viewport/window passes

Capture and inspect at least:

1. `980x760` — configured default desktop window
2. `720x560` — configured minimum desktop window
3. one wider desktop size when layout/navigation changes

For changes to a contained component, also capture a close enough view to judge spacing, alignment, states, and text hierarchy.

## Review order

Review each screenshot in this order.

### 1. Five-second read

Ask what the eye notices first.

The intended order should usually be:

- current/selected world or main task
- safety/health state
- primary action
- important replication/authority context
- secondary controls

If branding, a giant title, glow, empty space, diagnostics, or destructive buttons dominate before the user's task, revise the hierarchy.

### 2. Geometry

Inspect:

- left/right alignment
- baseline alignment
- repeated spacing
- control heights
- panel boundaries
- radius consistency
- density
- unused/dead space

Look for the small asymmetries that make generated interfaces feel assembled rather than designed.

### 3. Surface noise

Count visual containers.

Ask whether each border, card, shadow, gradient, chip, or badge has a job. Remove decoration that only says “this is a UI element.”

Pay special attention to card nesting and pill proliferation.

### 4. Typography

Check:

- titles are not oversized for a desktop utility
- body text remains readable
- muted text is not washed out
- helper text is close to what it explains
- machine data uses monospace only where useful
- uppercase/tracked labels are rare and purposeful

### 5. Action hierarchy

Scan buttons without reading paragraphs.

There should be an obvious primary action for the current task. Secondary and destructive actions should not compete equally.

If every button has a strong filled background, the hierarchy has failed.

### 6. Status hierarchy

Healthy, warning/solo, conflict/danger, and unknown states must be distinguishable by more than hue.

Status decoration should be strong enough to notice but not paint the whole app in semantic colors.

### 7. Desktop feel

Ask whether the screenshot resembles an installed utility or a responsive SaaS landing page.

Red flags:

- huge hero/header
- long scroll before core actions
- floating card gallery
- marketing-like CTA arrangement
- excessive glow/glass effects
- mobile-looking stacked layout at normal desktop width

### 8. Narrow-window behavior

At `720x560`, inspect what was sacrificed.

Good degradation:

- secondary columns stack/collapse
- labels remain readable
- advanced content becomes less prominent
- primary world/action context stays visible

Bad degradation:

- tiny text
- clipped buttons
- horizontal scrolling
- essential status pushed far below setup forms
- sticky elements covering content

## State screenshots

When the changed UI has meaningful states and the environment can reach them, inspect representative screenshots for relevant variants:

- no worlds/empty
- selected healthy world
- warning/solo world
- conflict/error state
- create/join workflow open
- loading/disabled action
- diagnostics/advanced content expanded

Do not claim state coverage for states you could not actually render.

## Critique before editing again

After each visual pass, write a short internal critique grouped by impact:

- hierarchy/task-flow problems
- layout/density problems
- consistency/component problems
- polish problems

Fix high-impact issues before polishing tiny details. A perfectly aligned bad hierarchy is still a bad UI.

## Iteration rule

For substantial redesign work, expect at least one screenshot → critique → edit → screenshot loop.

Do not stop after the first render if obvious issues are visible. Continue until the screenshots satisfy the `swarmcraft-ui-design` principles and `frontend-quality-gate`.

## Screenshot integrity

Use screenshots as evidence, not decoration.

- capture the app at the dimensions being evaluated
- avoid crops that hide overflow/layout problems
- do not use browser zoom to disguise density issues
- do not present a single ideal state as proof that all states work

## Completion report

When reporting completion of visual work, state which window sizes and states were actually inspected and note any rendering/tooling limitation precisely. Do not say “looks good” without describing what was verified.
