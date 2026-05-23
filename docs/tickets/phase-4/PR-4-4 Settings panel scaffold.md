# PR-4-4 — Settings panel scaffold + editing-context tabs

Status: ✅ shipped — `src/settings/SettingsPanel.tsx` ships the panel scaffold mounting CategorySidebar + ModeFilter + the form-component library; `src/settings/resolve.ts` ships `usePrinterOptions` (caches `slicer_options_for_printer` per stable printer-shape key) + `useCascadeResolve` (calls `cascade_resolve` per (handle, context) tuple). Editing-context tabs (Project / Object) follow the mockup's auto-fall-back pattern — Object tab disables when no object is selected and the panel snaps back to Project if the selection clears mid-edit. Search + mode filter compose into a single `filterRow` pure function (6 vitest cases). Project-scope settings are read-only on the Object tab per FR-3D-3 (PR-4-9 surfaces the "project-scope setting" badge; PR-4-4 enforces the disabled-input). Per-object override storage is stubbed via callback props — PR-4-9 wires the real `scene_object_override_set/clear` plumbing. The panel doesn't mount into App.tsx yet; that wiring lives with Phase 5's project model since the panel takes cascadeHandle + ContextJson + selection state that App.tsx currently doesn't carry. 63 frontend tests green.

**Scope.** The host panel that mounts category nav (PR-4-3) + form
components (PR-4-2) + the cascade-aware resolve/write loop. Includes
the Project / Object editing context tabs (FR-UI-9). This is the
**critical-path bottleneck** for the rest of Phase 4 — every PR-4-5
through PR-4-12 mounts inside this scaffold.

Owns FR-UI-9 (editing-context tabs) and FR-UI-3 (global search).

**Acceptance criteria.**

- New `src/settings/SettingsPanel.tsx` — top-level component
  mounted from `App.tsx`. Layout (Phase 2 neutral-900 palette):
  ```
  ┌──────────────────────────────────────────────────────────────┐
  │ [Project] [Object: <name>]   [Simple|Advanced|Expert]   [⌕]  │
  ├──────────────┬───────────────────────────────────────────────┤
  │ Category Nav │ Setting list (rows, virtualized if > 100)     │
  │ (PR-4-3)     │                                                │
  │              │ <row> [breadcrumb] [input] [reset] [badge]   │
  │              │ <row> ...                                      │
  └──────────────┴───────────────────────────────────────────────┘
  ```

- Editing context tabs (FR-UI-9):
  - **Project** tab is the default and active when nothing is
    selected in the viewport.
  - **Object: <name>** tab activates when one object is selected.
    Disabled (greyed out) when selection is empty; auto-switches
    back to Project when the selection clears.
  - Multi-select: the Object tab shows "Object: N selected" and
    edits land in all selected objects' override tiers
    simultaneously (per FR-3D-3's "first-class per-object").
  - The active tab determines the **write target**: edits made on
    Project tab go to `cascade_resolve_with_overrides`'s project
    tier; edits on Object tab go to the per-object override map
    (PR-4-9 ships the per-object storage).

- Resolve/write loop:
  - On mount, on printer/plate/filament/selection change, on
    user edit: call `cascade_resolve(handle, context, overrides)`
    via the existing Tauri command. Render the resolved values in
    each row.
  - User edit → optimistic local update → debounced commit (300
    ms) → `cascade_resolve` re-fetch. Optimistic update keeps the
    input responsive; the re-fetch reconciles in case the edit
    triggers a dimensional re-expansion (e.g. flipping
    `curr_bed_type` changes which `*_plate_temp` keys are active).

- Global search (FR-UI-3): the magnifying-glass icon opens a
  search input that filters rows across categories. Match against
  `key`, `label`, and `category` — **substring across all**
  (the `accountability === "instant"` default in
  `docs/design/app.jsx`'s `TWEAK_DEFAULTS`). The mockup's
  `scoped` (current-category only) and `fuzzy` modes are
  designer A/B tweaks and are NOT shipped. Each search hit
  shows its category name inline so the user knows where the
  option lives. Empty search returns to the normal
  category-filtered view.

- Performance gate (FR exit criterion):
  - Single-slot full re-render under **50 ms** measured via React
    Profiler. Virtualize the row list if naïve rendering blows
    the budget — `react-virtual` or hand-rolled (~150 rows visible
    at once is the typical category-pane case).
  - Add a vitest perf test that mounts the panel with a fixture
    cascade + 600+ options and asserts the resolve-and-render
    round-trip is < 50 ms in CI's debug build (10× the production
    budget; matches the headroom convention from
    `scene_state_perf` + `gcode_parser_perf`).

- vitest:
  - Mount the panel with a stub cascade, assert all visible rows
    render with their resolved values.
  - Switch the printer profile; assert visible options change per
    PR-4-5's capability filter (PR-4-5 ships the filter; PR-4-4
    just plumbs the printer prop through).
  - Click an object in a stub scene; assert the Object tab
    activates and the input writes go to the per-object tier (the
    per-object override storage is a stub here; PR-4-9 makes it
    real).

**Effort.** ~4 days. Layout + tabs is a day; resolve/write plumbing
is a day; perf gate + virtualization tuning is two days.

**Dependencies.** PR-4-1 (backend introspection), PR-4-2 (form
components), PR-4-3 (category nav + mode filter).

**Out of scope.** Source-layer breadcrumb (PR-4-7). Cascade ladder
(PR-4-8). Per-object override storage (PR-4-9 — scaffold renders
the tabs; the actual override-tier write path is PR-4-9's
responsibility). Diff view (PR-4-10). Tooltips + validation
(PR-4-11).

**Cut candidate.** None — this is the critical path. If perf is a
fight, ship without virtualization first and gate on the 50 ms
budget; introduce virtualization only if needed (typical
category-pane never breaches the budget).

**Design reference.** The entire panel layout follows the mockup
`docs/design/SettingsPanel.jsx`'s `SettingsPanel` root component.
Key markers:

- The config strip at the top (`.sp-config-row` ×2 in the
  mockup) renders printer / bed plate / nozzle as `config-chip`
  buttons + filaments in use as `filament-chip` rows. PR-4-4
  ships the strip's layout + the printer/bed/nozzle slots;
  PR-4-5 wires the bed plate selector to PR-2-6's
  `scene_set_active_plate`, and Phase 5 handles the printer
  swap menu (`printer-picker-menu`).
- The editing-context tabs (`.sp-tabs` + `.sp-tab`) are exactly
  as the mockup shows — Project + Object pill tabs with hue-
  colored dots (project hue 285, object hue 340). When the
  selected object becomes unavailable, the mockup's `useSPE`
  automatically falls back to Project — replicate that
  invariant.
- The search row (`.search-wrap` + `.search-input`) gets the
  ⌘F shortcut, the live `N matches across M categories` hint
  beneath, and the × clear button when query is non-empty.
- The body splits into `.cat-rail` (left, PR-4-3) and
  `.settings-scroll` (right, the scrollable list of
  `.cat-group` sections).

`contextLayer` is the mockup's name for what the ticket has been
calling the "active editing tier" — use the mockup's term in
code to match.
