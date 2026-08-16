---
name: frontend-quality-gate
description: Provides a completion gate for SwarmCraft frontend changes. Use after or during HTML/CSS/JavaScript UI work, redesigns, component changes, accessibility fixes, layout work, or before declaring a desktop frontend task complete.
---

# SwarmCraft Frontend Quality Gate

Do not mark a meaningful frontend change complete until it passes the checks below. Fix failures rather than merely listing them, unless the environment genuinely prevents validation.

## 1. Scope and wiring

Review the final diff for accidental behavioral changes.

- preserve DOM IDs consumed by `apps/desktop/src/app.js`, or update all matching references together
- preserve Tauri command names and payload shapes unless the backend API is intentionally changing
- remove dead CSS/markup created by the refactor
- avoid unrelated formatting churn
- do not add a framework/dependency just to solve styling

## 2. Window-size gate

Render and inspect at least:

- `980x760` configured default window
- `720x560` configured minimum window

Also inspect one wider size when the change affects grids/navigation.

At every size verify:

- no horizontal page scrolling
- no clipped primary actions
- no overlapping text or controls
- readable line lengths
- useful use of vertical space
- sticky/fixed elements do not cover content
- dialogs/popovers fit inside the window

Long world IDs, paths, multiaddresses, and error text must wrap/truncate intentionally rather than break layout.

## 3. Interaction gate

Exercise every changed interactive control.

Check:

- pointer hover where relevant
- mouse click
- keyboard Tab navigation
- visible focus
- Enter/Space behavior for custom interactive items
- disabled state
- repeated-click behavior during async actions
- cancellation/close behavior for overlays/dialogs

Do not ship hover-only essential actions.

## 4. State gate

For each changed data/task region, consider all applicable states:

- initial
- empty
- loading/refreshing
- success
- error
- disabled/unavailable
- selected/unselected
- long-content/overflow
- degraded/solo
- conflict/danger
- unknown/not initialized

Do not fake protocol state in production code just to make screenshots look populated.

## 5. Safety semantics gate

SwarmCraft state colors and labels carry product meaning.

Verify the UI does not imply:

- solo state is as safe as canonical/quorum state
- a discovered peer is already a member
- a relay is an authority
- an incompatible storage-only peer can execute/host
- conflict state is routine/healthy
- a destructive recovery/membership action is harmless

Never trade correctness of these distinctions for cleaner copy or prettier grouping.

## 6. Accessibility gate

Check at minimum:

- all form controls have visible labels
- interactive controls have accessible names
- focus indicator is obvious against the dark background
- logical DOM/focus order matches visual order
- status is not communicated by color alone
- text contrast remains readable, especially muted/helper text
- controls have practical hit targets
- semantic HTML is used where available

If creating a custom clickable non-button element, justify why native `button`, `a`, `input`, `details`, or another native element does not fit.

## 7. Visual consistency gate

Compare the changed area with the rest of the application.

Reject:

- new one-off radius values without reason
- new arbitrary greens/greys when semantic tokens exist
- mismatched button heights/padding
- multiple unrelated spacing scales
- duplicated component CSS
- extra shadows/glows/gradients without functional purpose
- another generic card where a section/divider/layout would be enough

If the work introduces repeated values, consolidate them into tokens or reusable classes.

## 8. Copy gate

Read all changed user-facing text in context.

It should:

- be concise
- state consequences accurately
- use consistent capitalization/terminology
- avoid filler copy
- avoid leaking raw implementation errors when a useful explanation is possible
- retain technical terms when they encode important protocol meaning

## 9. Code gate

For HTML:

- use semantic structure
- avoid invalid nesting and duplicate IDs
- keep labels associated with controls

For CSS:

- prefer reusable classes and custom properties
- avoid `!important`
- avoid excessive selector specificity
- ensure focus/hover/disabled/error states are defined with the component
- remove obsolete selectors after structural changes

For JavaScript:

- keep rendering functions small enough to reason about
- use `textContent`/DOM APIs for untrusted dynamic text rather than raw HTML injection
- do not swallow actionable errors without an intentional local fallback
- avoid duplicating command/status logic unnecessarily

## 10. Repository checks

Run checks relevant to the changed files. When the desktop Rust shell is involved or the frontend/Tauri package needs validation, use:

`cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml`

If a check cannot run because of a missing external tool/dependency, report the exact blocker. Do not imply it passed.

## 11. Visual evidence

When a rendering/browser environment is available, apply the `visual-review` skill and inspect screenshots. Source-only review is not enough for visual changes when rendering is possible.

## Definition of done

A frontend task passes this gate only when the implemented UI is functionally correct, visually coherent, keyboard usable, safe in its protocol messaging, and usable at both configured window sizes.
