# PR-5-9 — `App.tsx` integration: mount SettingsPanel + PlateTabs

Status: ❌ open.

**Scope.** Wire the Phase 4 settings panel and the Phase 5
plate tabs into `App.tsx`. PR-4-4 explicitly deferred the
mount because the panel takes cascade handle + ContextJson +
selection state that App.tsx didn't carry. Phase 5's project
model provides all of that.

Owns the **integration glue** — no new business logic; just
the prop-drilling + state-management that ties the panels
to the project model.

**Acceptance criteria.**

- `App.tsx` evolution:
  - Top-level state: `Project | null` (with loader for the
    initial empty-default project).
  - Loads the bundled A1 mini cascade on mount + creates a
    1-plate default project bound to A1 mini (matches the
    Phase 4 fixture).
  - Renders the new top bar (already has slice panel) →
    PlateTabs (PR-5-3) → main layout:
    - **Left:** existing ObjectsPanel (when Phase 2 polishes
      it — for now skipped).
    - **Center:** existing ViewportCanvas.
    - **Right:** SettingsPanel (Phase 4) as a togglable
      right-side sidebar overlay.

- SettingsPanel host wiring:
  - Pass `cascadeHandle = project.cascade_handle`.
  - Build `ContextJson` from
    `project.plates[project.active_plate]` (printer +
    plate + filaments + active_slot + user + project
    overrides).
  - Wire `setProjectOverride` / `clearProjectOverride`
    callbacks to the per-plate override mutation via the
    new `scene_set_project_override` Tauri command (small
    addition — mirror PR-5-7's per-object pattern).
  - Wire `setObjectOverride` / `clearObjectOverride` to
    PR-5-7's commands.
  - Pass `selectedObject` from the scene's selection state.
  - Pass `allObjects` (per PR-4-9's `PlateObjectStub[]`)
    derived from the active plate's `objects` map +
    material bindings (for filament-color dots).

- PlateTabs host wiring:
  - Pass `plates` from `project.plates` (sliced down to
    `{ id, name, printer.identity }` per the PlateTabsProps
    shape).
  - Wire `switch` / `add` / `remove` / `rename` to
    PR-5-3's commands.

- Active-plate sync: when the user clicks a different
  tab, App.tsx re-runs the cascade resolve for the new
  plate (already automatic via `useCascadeResolve`'s
  context-keyed effect).

- Settings panel toggle: header button to show / hide the
  right-side panel; persisted to localStorage so the
  preference survives reloads. Defaults to visible.

- Tests:
  - Existing App tests pass (the viewport still mounts;
    the slice panel still works).
  - New: integration test for the project-state flow —
    create default project, add plate, switch plate,
    verify cascade resolves against the new plate's
    printer.

**Effort.** ~2 days. The wiring is mechanical; the friction
is making sure no Phase 4 prop has the wrong shape after the
project-model translation.

**Dependencies.** PR-5-3 (PlateTabs), PR-5-7 (per-object
override backend), PR-4-4 (SettingsPanel scaffold),
PR-5-1/2 (project model).

**Out of scope.** Project-load file dialog in the menu
(PR-5-8 ships the Tauri command; PR-5-9 doesn't surface a
File menu — that's Phase 9 polish). Drag-to-resize the
settings panel (Phase 9). Multi-window support (post-MVP).

**Cut candidate.** Settings-panel toggle (~half day) —
ship visible-always. Cuts a UX nice-to-have; no PRD
requirement at stake.

**Design reference.** `docs/dev/design/app.jsx`'s `App`
component shows the canonical 4-pane layout (TopBar /
PlateTabs / ObjectsPanel + BuildPlate + SettingsPanel /
status bar). Production follows the same layout; the
ObjectsPanel slot stays empty until Phase 2's polish
(or skipped for MVP).
