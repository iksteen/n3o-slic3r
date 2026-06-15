# Design mockup — findings and reusability

> Status: review notes after the first read-through of `docs/dev/design/`,
> 2026-05-22. The mockup is a feature-incomplete React-in-browser
> prototype the project lead built earlier. This document captures
> what's reusable as-is, what needs porting, what needs replacing, and
> what design surfaces are still missing relative to the PRD.

## What's in `docs/dev/design/`

A self-contained React 18 + Three.js prototype, served from a single
`index.html` that loads React, Babel, and Three.js from unpkg, then
transpiles `.jsx` files in the browser:

```
index.html               39 lines    entry
n3o-slic3r.html          67 lines    shell variant
n3o-slic3r-standalone.html  201 lines    bundled standalone variant
styles.css            1,393 lines    full design language
app.jsx                418 lines    top-level shell, plate-centric state
data.jsx               343 lines    cascade layers, settings catalog, mock data
TopBar.jsx              50 lines    title bar
PlateTabs.jsx          107 lines    plate-tab strip
ObjectsPanel.jsx       181 lines    object library + plate object list
BuildPlate.jsx         473 lines    Three.js viewport with drag-arrangement
SettingsPanel.jsx      698 lines    categorized scrolling settings + cascade ladder
tweaks-panel.jsx       530 lines    runtime theme / variant switcher (prototype-only)
```

~6.3k lines total. The visual design is polished — Geist font, custom
hue-per-layer color tokens, cascade ladder rendered via portal so the
scroll container doesn't clip it, fuzzy search with category jump-rail.
Past the napkin-sketch stage.

## Reusability split

### Reuse directly

- **`styles.css` (1,393 lines).** Pure CSS, no framework dependency.
  Drop in as the production design language. Custom-property hue tokens
  per cascade layer (`--row-hue`), polished form controls, cascade-
  ladder portal styling, scroll-list affordances. This is the biggest
  single win in `docs/dev/design/` — the design system is already
  concretized.

- **Layout topology.** `TopBar + PlateTabs + ObjectsPanel + BuildPlate
  + SettingsPanel` maps 1:1 to the PRD: plate-centric workspace
  (FR-MP-1..2), per-plate printer (FR-MP-2), plate switching switches
  workspace (FR-MP-6). The component decomposition is right; carry it
  forward.

- **CascadeLadder interaction model** (`SettingsPanel.jsx`). Hover-
  portal showing per-layer values, winning-layer highlight, em-dash
  for undefined layers, per-object-overrides section with filament-
  color dots. This is the concrete UX for **FR-CAS-7** (source
  disclosure) and **FR-CAS-7b** (objects-overriding-this badge). The
  rendering pattern is what should ship.

- **ObjectsPanel sections** (`ObjectsPanel.jsx`): Primitives and
  Imported. Directly maps to **FR-UI-10**. (The mockup's Calibration
  section is out of MVP scope — see FR-UI-10.)

- **Settings search and jump-rail** (`SettingsPanel.jsx`). Fuzzy
  match with category breadcrumbs. **FR-UI-3**.

- **Object-overrides-on-row badge** (filament-color dots, overflow
  marker, hover-revealing object list). Directly **FR-CAS-7b**.

### Port (rewrite as TS+Vite-compatible, keep the design)

- **All `.jsx` → `.tsx` modules.** Per-file `useState`-aliasing
  patterns (`useStateOP`, `useStateSP`, `useSPS`) suggest the originals
  were globals via window-shared React; that goes away with real
  module imports. Component contracts stay; plumbing is rewritten.

- **`BuildPlate.jsx`'s Three.js setup.** Scene/renderer/orbit-controls
  boilerplate is portable. The `makeGeometry()` switch over mock
  `kind` strings needs replacing with real STL/3MF loaders (Phase 2 of
  the execution plan).

- **`ObjectsPanel.jsx`'s mock library** (placeholder STL names,
  primitive shapes). Becomes real importable geometry and primitives.

### Replace

- **`data.jsx`'s settings catalog.** The mockup's setting keys
  (`layer_height`, `wall_count`, `infill_density`, `print_temp`,
  `print_speed`, …) are *display-name guesses*, not libslic3r's actual
  option keys (`perimeters`, `fill_density`, `nozzle_temperature`,
  etc.). The real catalog comes from `option_defs()` via the FFI —
  737 options with labels, categories, modes, scopes, defaults, enum
  values. The mockup's rendering patterns are reusable; the data
  feeding them is replaced wholesale.

- **`tweaks-panel.jsx`.** Prototype-time variant switcher (theme /
  accent / accountability-mode toggles). Useful during design
  exploration; not for production. Pick the winning variant and ship
  that; drop the switcher.

- **In-browser Babel transpile + CDN-loaded React/Three.** Production
  uses Vite + tree-shaken NPM deps.

## Cascade-layer correction applied

The mockup originally had **7 linear-priority layers**:
`printer → toolhead → nozzle → filament → user → project → object`.
This doesn't match the PRD's formalized model.

I edited `data.jsx`, `app.jsx`, and `SettingsPanel.jsx` to align the
mockup's layer set with the PRD:

```
default       (authored, base)
printer       (authored, includes toolhead + nozzle context state)
build_plate   (authored)
filament      (authored)
user          (override tier 1)
project       (override tier 2)
object        (override tier 3)
```

Migration of mock data:

- Old `toolhead:` values → `default:` (9 occurrences)
- Old `nozzle:` values → `printer:` (9 occurrences — line widths, min
  feature size, etc.; in the PRD model these flow from the printer's
  per-slot nozzle config which is printer-profile context state, not a
  separate cascade layer)
- Added `default` and `build_plate` layers to `CASCADE_LAYERS`
- Removed `toolhead` and `nozzle` from `CASCADE_LAYERS`
- Updated chip styling in `SettingsPanel` to pull the nozzle chip's
  hue from the printer layer (since nozzle isn't its own layer
  anymore)
- Updated `app.jsx`'s context-label switch to drop `toolhead` and
  `nozzle` branches, add `build_plate` and `default`

Note that the mockup's resolver is still a simple "highest defined
wins" linear walk — it does not implement specificity, source order,
or two-phase resolution. That's fine for the mockup. The real
resolver in the Rust backend (Phase 1 of the execution plan) does
proper two-phase resolution per `docs/dev/profiles.md`.

## Mismatches between mockup and PRD/profiles.md

Resolved by the edits above:
- ~~7-layer linear stack vs PRD's 4 authored + 3 override tiers~~

Still open (not blockers, but should be addressed at the right phase):

- **Resolution mechanism.** Mockup is single-phase "highest defined";
  PRD/profiles.md is two-phase with specificity in phase 1 and
  `!important`-style override tiers in phase 2. The mockup's display
  walks the same 7 layers but doesn't reflect the phase boundary or
  show "this would revert to X" when an override is active. Phase 4
  (Settings UI in the execution plan) needs to model this — the
  ladder should visually separate the authored tier from the override
  tiers, and display the "cascade fallback" the user would revert to.

- **Setting-source attribution.** Mockup attributes each layer to a
  single value; the real resolver attributes to a rule with a
  `file:line` and a specificity. The ladder display will eventually
  need that detail (hover a layer chip → "from
  `plate-PEI.toml:47`, specificity 2").

## Design gaps relative to the PRD

Surfaces the mockup doesn't cover. These need design work before their
respective phases start:

- **Plate-printer assignment UI.** `PlateTabs.jsx` switches plates but
  there's no surface for "this plate prints on A1 mini, that plate on
  U1." **FR-MP-2** says printer is a per-plate property; the UI needs
  to make this obvious and one-click changeable. Required for Phase 5
  (multi-printer project model).

- **Filament sync.** PRD §6.8 is substantial — 14 FRs covering live
  AMS state reads, model-material→slot binding per (plate, printer),
  slot-loaded/available validation, sync-on-send metadata. None of it
  is in the mockup. This is one of the project's main differentiators ("the
  integration that current slicers handle poorly for U1 and other
  multi-printer setups") and needs its own design pass. Required for
  Phase 7.

- **G-code preview.** PRD §6.6 is a hard MVP requirement with 12 FRs:
  layer slider, range mode, color modes, hover inspection, per-layer
  and full-job stats, drag-drop preview, `.gcode.3mf` unpacking. Not
  in the mockup. Required for Phase 6.

- **Plugin panel and plugin-declared settings UI.** PRD §6.9 names a
  Plugins category in the settings panel plus a plugin error/log
  surface. Not in the mockup. Required for Phase 8.

- **Per-printer capability adaptation.** PRD's **AD-1** (printer-aware
  setting visibility) and **AD-2** (single- vs multi-slot UI layout)
  need the settings panel to adapt per active printer (single nozzle
  pane for A1 mini, 4-tab strip for U1, hidden purge volumes on
  toolchangers, etc.). Mockup is single-printer-shaped throughout.

## Recommendation for Phase 4 kickoff

When Phase 4 (Settings UI, ~5 person-weeks) starts:

1. Stand up `crates/n3o-slic3r-frontend/` or similar with Vite + TS +
   the existing React/Three already in `src-tauri`. Drop `styles.css`
   in. Port the layout shell (TopBar, PlateTabs, ObjectsPanel,
   SettingsPanel) component-by-component to `.tsx`.

2. Replace `data.jsx`'s settings catalog with a Tauri command-fed
   feed from `option_defs()` (the FFI's existing surface).

3. Replace `data.jsx`'s `resolveValue` with a Tauri command into the
   real resolver (Phase 1's deliverable). Trace metadata flows into
   the ladder.

4. Treat the four design gaps above as separate design passes, each
   sized at maybe a half-day of mockup work before its phase starts.

The visual design carries forward; the data plumbing underneath is
replaced. Net effect: most of the mockup's UX survives unchanged in
the production build, with the cascade ladder pointing at real rules
in real files at real line numbers.
