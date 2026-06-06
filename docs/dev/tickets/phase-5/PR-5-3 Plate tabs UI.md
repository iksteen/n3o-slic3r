# PR-5-3 — Plate tabs UI (add / remove / switch / rename)

Status: ❌ open.

**Scope.** Horizontal plate-tabs strip below the top bar. Each
tab shows the plate's name + assigned printer; clicking
switches the active plate (which switches the entire workspace
per PR-5-2's event scoping). `+` adds, `×` removes (with
single-plate guard), double-click on the name renames inline.

Owns FR-MP-1 (one or more plates, no upper limit), the user-visible side of FR-MP-2
(switchable assignment — the binding change itself ships with
PR-5-4).

**Acceptance criteria.**

- New `src/plates/PlateTabs.tsx` mirroring
  `docs/dev/design/PlateTabs.jsx` verbatim:
  - `.plate-tabs` container with horizontal scroll
  - `.plate-tab` per plate, `.active` modifier on the
    selected one
  - `.plate-tab-icon` shows the build-plate identity
    (textured / smooth / cool / engineering / supertack)
    via a small SVG glyph
  - `.plate-tab-name` is the editable label; double-click
    swaps to a focused `<input class="plate-tab-rename-input">`
    that commits on Enter / blur and cancels on Escape
  - `.plate-tab-close` (×) on hover; clicking removes the
    plate (with a confirm prompt if the plate has > 0
    objects)
  - `.plate-tab-add` (+) at the end of the strip

- New `src/plates/usePlates.ts`:
  - React hook reading the project model (or the per-plate
    snapshot from `scene_snapshot`)
  - `start(plate?)`, `switch(plateId)`, `rename(plateId, name)`,
    `add(printer)`, `remove(plateId)` actions wrapping the
    Tauri commands
  - localStorage-cached last-active plate id so reloads
    pick up where the user left off

- New Tauri commands (additive to PR-5-2's `scene_set_active_plate`):
  - `project_add_plate(printer: PrinterBinding) -> PlateId`
  - `project_remove_plate(id: PlateId)` — errors if last
    plate or if `Project.plates.len() == 1`
  - `project_rename_plate(id: PlateId, name: String)`

- Wire into `App.tsx` immediately below the existing header
  (where the slice panel lives) — the strip is
  always-visible, not togglable.

- vitest:
  - usePlates reducer composes the event stream into the
    expected state (mirror the slice reducer test pattern)
  - Pure helpers: plate-name default generator, "is this
    the last plate?" guard, scroll-into-view-on-add
    behavior

**Effort.** ~2 days. The mockup carries the layout +
behavior; the work is the Tauri wiring + react-hookification.

**Dependencies.** PR-5-2 (per-plate SceneState + active-plate
events).

**Out of scope.** Per-plate printer assignment (PR-5-4 owns
the binding-change wiring; PR-5-3 just renders the current
printer name in the tab). Drag-to-reorder plates — Phase 9
polish.

**Cut candidate.** Inline rename (~half day) — fall back to
a context-menu "Rename" action. Names default to "Plate N";
users can live without the rename for MVP.

**Design reference.** `docs/dev/design/PlateTabs.jsx` is the
canonical source for layout + behavior. The class names
(`.plate-tabs`, `.plate-tab`, `.plate-tab-icon`,
`.plate-tab-name`, `.plate-tab-rename-input`,
`.plate-tab-close`, `.plate-tab-add`) and the rename
behavior (double-click → focused input → Enter/blur commit
/ Escape cancel) lift verbatim.
