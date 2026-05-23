# PR-4-5 — Printer-aware visibility filter + build plate selector

Status: ✅ shipped — the capability filter lands cleanly on PR-4-1's foundation: `slicer_options_for_printer` already pre-evaluates each option's `CapabilityPredicate` per printer and stamps a `hidden: bool` flag, so the frontend per-row hide/show is a single field read. `SettingsPanel`'s `filterRow` excludes hidden options in the default view and surfaces them in search view; capability-hidden rows in search show a `not applicable` chip in the Field's leadingBadge slot and the input is disabled. `categorize` is now generic over the option type so the `hidden` flag propagates through grouping without lossy projection. New `BuildPlateSelector` component: dropdown of `printer.supported_build_plates` + `printer default` badge when the user's selection matches the printer's documented default plate. The host (Phase 5's project model) wires it into the `cascade_resolve` ContextJson; no scene-state command needed — the active plate lives in the slice context, not in SceneState today.

**Scope.** Two related printer-driven concerns:

1. **Visibility filter (FR-UI-7):** hide options that aren't
   meaningful for the active printer. The capability predicates
   ship with PR-4-1; this ticket wires them into the panel's row
   render and the search results. Hidden options remain findable
   via search with a `not applicable to this printer` badge.
2. **Build plate selector (FR-CAS-9):** the dropdown listing the
   active printer's supported plates. Selection updates the
   `BuildPlate` cascade layer; printer-reported default plate (if
   the printer ships one) shows with a visible badge when the
   user overrides.

**Acceptance criteria.**

- New `src/settings/visibility.ts`:
  - `function applyCapabilityFilter(options: OptionSummary[],
     printer: PrinterProfile, mode: 'hide' | 'search'):
     FilteredOption[]` — wraps each option with `{ option, hidden,
     hiddenReason }`. In `'hide'` mode, hidden options are
     omitted; in `'search'` mode (used when search is active),
     they're included with the `hiddenReason` populated so the
     UI can badge them.
  - Predicate evaluation is pure / cheap (a `switch` over
    `CapabilityPredicate` variants); the panel's per-render
    filter pass should complete in microseconds.

- Wire into `SettingsPanel`:
  - Normal (non-search) view: `mode='hide'` — capability-filtered
    options are absent from the rendered list.
  - Search-active view: `mode='search'` — every match renders,
    but capability-hidden matches show a small chip
    `not applicable: <reason>` next to the row.

- New component `src/settings/BuildPlateSelector.tsx`:
  - Renders inline near the top of the panel (above the category
    nav). Dropdown lists the active printer's
    `supported_build_plates`.
  - Default selection: the printer's reported default (if any)
    is the initial value with a `printer default` badge. User
    selection clears the badge; switching back to the default
    value restores it.
  - On selection, dispatches `scene_set_active_plate(plate_id)`
    via Tauri — the existing scene command surface that PR-2-6
    shipped. The cascade resolver picks up the new plate on its
    next `cascade_resolve` call (already wired through
    `SlicingContext::plate`).

- Smoke check baked into the exit criteria:
  - Switching from A1 mini to a synthetic dual-extruder printer
    flips ~6 options' visibility (verify against a fixture
    capability list). Switching back hides them again.
  - Selecting a different plate changes the bed-temperature row's
    resolved value (the dimensional expansion adapter from
    PR-1-6 picks up `curr_bed_type`).

- vitest:
  - `applyCapabilityFilter` returns the right hidden/visible
    partition for A1 mini (no toolchanger, no purge tower,
    slot_count=4).
  - `BuildPlateSelector` lists the active printer's plates and
    surfaces the default-badge correctly.

**Effort.** ~2 days. The capability-mapping work is in PR-4-1; this
ticket is mostly wiring + the plate selector.

**Dependencies.** PR-4-1 (capability predicates),
PR-4-4 (panel scaffold), PR-2-6 (active plate in SceneState).

**Out of scope.** Per-option `condition` expression evaluation
beyond the typed `CapabilityPredicate` set (inter-field gating).
Printer profile editing — users hand-edit `profiles/printers/*.toml`
through the filesystem; the build plate selector reads from the
profile but doesn't write to it.

**Cut candidate.** The `printer default` badge on the build-plate
selector (~half day). Users can still pick a plate; the badge is
nice-to-have.

**Design reference.** The mockup's `.sp-config-row` shows the
build plate + printer + nozzle as `config-chip` buttons with
swap menus on click (`printer-picker-menu` for the printer
chip — Phase 5 ships that one). PR-4-5's build plate selector
mirrors this pattern: a `config-chip` with the active plate
name + a `ChevronChip`, opening a small menu listing the
printer's supported plates. The `printer default` badge slots
inside the chip's value area (right of the plate name) when the
user hasn't overridden — mockup convention is a small uppercase
muted-color marker.

The capability-driven hide doesn't have a mockup counterpart
(all settings show); document the absent-from-list behavior in
the PR-4-5 implementation comments so a future designer can
add a "hidden settings" affordance if needed.
