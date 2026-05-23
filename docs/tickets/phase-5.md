# Phase 5 — tickets

Phase 5 (multi-printer project model, ~3 person-weeks) makes
**multi-plate, multi-printer projects the default workflow** —
the PRD's "primary differentiator alongside Phase 4's
cascade-aware settings" claim. Source: `docs/Execution_Plan.md`
§7. Stated goal:

> Project model with N plates, each bound to a printer. Plate
> tab UI. Per-printer cascade re-resolution. Plate-level
> metadata (cycle counts). Model material → slot bindings.
> Project save/load via `.3mf`. Bed visualization per plate.
> Move-object-between-plates. Autosave + recovery.

Phase 5 is **the integration phase** — it's where the pieces
the earlier phases shipped in isolation become a real
application:

- **Phase 2**'s `SceneState` becomes `Vec<PlateState>`.
- **Phase 3**'s `.3mf` writer + sliced-3MF writer become the
  project-save format + the per-plate send-to-printer wrapper.
- **Phase 4**'s `SettingsPanel` finally mounts into `App.tsx`
  (the wiring was explicitly deferred there) and re-resolves
  per-plate when the active plate changes.

Individual tickets live one-per-file in `phase-5/`. This file is
the index plus phase-level status and notes.

## Status by deliverable

| Deliverable | Status | Ticket |
|-------------|--------|--------|
| Project / Plate / Binding domain types | ❌ open | [PR-5-1](phase-5/PR-5-1%20Project%20types.md) |
| Per-plate SceneState refactor + command migration | ❌ open | [PR-5-2](phase-5/PR-5-2%20Per-plate%20scene%20state.md) |
| Plate tabs UI (add / remove / switch / rename) | ❌ open | [PR-5-3](phase-5/PR-5-3%20Plate%20tabs%20UI.md) |
| Per-plate printer assignment + cascade re-resolution | ❌ open | [PR-5-4](phase-5/PR-5-4%20Per-plate%20printer%20assignment.md) |
| Plate-level metadata (cycle count + composition order) | ❌ open | [PR-5-5](phase-5/PR-5-5%20Plate%20metadata.md) |
| Model material → slot binding model | ❌ open | [PR-5-6](phase-5/PR-5-6%20Material%20bindings.md) |
| Per-object override Tauri backend (deferred from PR-4-9) | ❌ open | [PR-5-7](phase-5/PR-5-7%20Per-object%20override%20backend.md) |
| Project `.3mf` save/load (extended namespace) | ❌ open | [PR-5-8](phase-5/PR-5-8%20Project%20save%20and%20load.md) |
| `App.tsx` integration — mount SettingsPanel + PlateTabs | ❌ open | [PR-5-9](phase-5/PR-5-9%20App%20integration.md) |
| Autosave + recovery on launch | ❌ open | [PR-5-10](phase-5/PR-5-10%20Autosave.md) |
| Move-object-between-plates (cut candidate) | ❌ open | [PR-5-11](phase-5/PR-5-11%20Move%20object%20between%20plates.md) |
| Phase 5 exit-criteria smoke | ❌ open | [PR-5-12](phase-5/PR-5-12%20Exit-criteria%20smoke.md) |

## Architecture invariant — plate-bound state is per-plate; project state is project-wide

Phase 5 splits the single-plate-implicit model the earlier phases
shipped against. The invariant the rest of the codebase must
honor going forward:

- **Per-plate state** lives in `PlateState`: scene objects,
  printer binding, build plate, filaments-in-use, material →
  slot bindings, project-tier overrides, cycle count,
  composition order. Each plate is independent — slicing plate
  3 doesn't see plate 1's overrides.
- **Project-wide state** lives in `Project`: the plate list +
  ordering, the loaded cascade handle, user-tier overrides
  (which apply across all plates), project metadata (title,
  designer, license), the path the project came from / saves to.
- **Tauri command boundary** takes a plate id (or implies
  "active plate" for legacy commands that already had a
  single-plate worldview — those keep working unchanged but
  the new commands are explicit).

Resist short-circuits like "this command edits *every* plate's
state in one go" — that's exactly the Phase 2 single-plate
worldview Phase 5 is dismantling. Per-plate is the contract.

## Design reference

`docs/design/`'s mockup carries Phase 5 surfaces:

- **`docs/design/PlateTabs.jsx`** is the canonical reference for
  PR-5-3's plate-tab strip — horizontal scroll, add (`+`),
  close (`×`), inline rename (double-click → input). Class
  hooks: `.plate-tabs`, `.plate-tabs-scroll`, `.plate-tab`,
  `.plate-tab-icon`, `.plate-tab-name`, `.plate-tab-rename-input`.
- **`docs/design/app.jsx`** at the `App` component is the
  canonical reference for PR-5-9's `App.tsx` integration —
  state is plate-centric (each plate owns its printer / bed /
  nozzle / objects / overrides); switching plate tabs switches
  the entire workspace. The mockup's `INITIAL_PLATES`
  fixture shows the per-plate shape.
- **Printer-picker menu** (`SettingsPanel.jsx` config strip):
  the post-PR-4-5 enhancement that lets users swap the active
  plate's printer from the settings panel config strip. PR-5-4
  ships the binding-change wiring; the menu component PR-4-5
  stubbed in (`config-chip-printer`) gets its real handler.

The same tweak-vs-deliverable discipline from
`phase-4.md` applies — see that file for the
`TWEAK_DEFAULTS` enumeration.

## Dependency graph

```
PR-5-1 (project + plate + binding types)
  └── PR-5-2 (per-plate SceneState refactor)  ← critical path
       ├── PR-5-3 (plate tabs UI)
       ├── PR-5-4 (per-plate printer assignment)
       ├── PR-5-5 (plate metadata)
       ├── PR-5-6 (material bindings)
       ├── PR-5-7 (per-object override Tauri backend)
       └── PR-5-11 (move object between plates — cut candidate)

PR-5-3 + PR-5-4 + PR-5-5 + PR-5-6 + PR-5-7 + 3MF writer (PR-3-9)
  └── PR-5-8 (project .3mf save/load)
       └── PR-5-10 (autosave + recovery)

PR-5-3 + Phase 4 SettingsPanel
  └── PR-5-9 (App.tsx integration)

All of PR-5-1..-10 ─► PR-5-12 (exit smoke)
```

The critical path is **PR-5-2** (per-plate SceneState refactor).
Everything else either depends on it directly or depends on
something that does. Land PR-5-1 + PR-5-2 first; the rest can
fan out.

## Exit criteria for the phase (from Execution Plan §7)

> Create a 3-plate project, assign Plate 1 to A1 mini and
> Plates 2-3 to U1, slice all three, save and reload with all
> settings preserved including per-plate cycle counts and
> material bindings.

The smoke (PR-5-12) mechanizes the slice + save + reload chain
on this exact 3-plate fixture. Per-plate cycle counts +
material bindings + printer bindings must round-trip
byte-equivalent through the `.3mf` writer.

## Cut candidates (from Execution Plan §7)

If pressed for time:

- **Move-object-between-plates** (PR-5-11) → saves ~2 days.
  Cut first; users can delete + re-add an object on the target
  plate as a workaround. The PRD requirement (FR-MP-6) becomes
  Phase 9 polish.
- **Autosave recovery wizard** (sub-deliverable of PR-5-10) →
  saves ~1 day. Autosave still runs; the recovery on launch
  becomes a manual "Open recovery file" menu item.
- **Per-plate cycle count UI** (sub-deliverable of PR-5-5) →
  saves ~1 day. Plates default to cycle=1 in the project
  file; user can't change. **Cuts the PlateCycler value
  prop** — the platecycler plugin (Phase 8) needs varying
  cycle counts to be useful. Cut LAST.

## What's *not* in Phase 5

- **Cascade-layer tagging in cascade files** — Phase 9 polish.
  PR-4-7's rule + tint surfaces the override tiers; cascade-tier
  attribution (printer / filament / plate / default) lights up
  the rest of the seven-hue palette once profile files are
  labeled with their source layer.
- **Filament sync** (live polling from connected printers) —
  Phase 7c. PR-5-6 ships the binding *model*; Phase 7c wires
  the slot-availability check.
- **G-code preview** — Phase 6. Phase 5 ships the slice loop
  per-plate; the preview reads what we sliced.
- **Driver UX** (send-to-printer affordances per driver) —
  Phase 7. Phase 5's project save format is `.3mf` for
  storage; per-printer send wrapping happens at slice time.
- **Composition plugin host** — Phase 8. PR-5-5 stores
  composition order; Phase 8 builds the plugin API that
  consumes it.
- **Calibration tower fixtures** as 3MF/STL (task #102) —
  Phase 9 / unblocked when an upstream source emerges. Phase
  5 doesn't move the needle.

## Open questions seeded for the implementer

- **Cascade re-resolve granularity** (PR-5-4). When the active
  plate's printer changes, every cascade-resolved value
  potentially shifts. PR-4-4's `useCascadeResolve` already
  re-runs on (handle, context) tuple change; verify that
  per-plate context changes flow through cleanly without
  redundant resolves when only the *non-active* plate
  changes.
- **Cycle count UX surface** (PR-5-5). Mockup doesn't show
  one. Options: per-plate-tab badge + inline number input,
  per-plate settings panel section, or a plate-properties
  modal. The PlateCycler value prop needs the cycle count to
  be authorable but the panel surface is open.
- **`.3mf` namespace name** (PR-5-8). The PRD says "extended
  3MF in our own namespace." Pick something distinct enough
  that BBS / Orca / PrusaSlicer won't accidentally parse it
  but recognizable for our own tooling. Suggest
  `n3o-slic3r-project` or similar; document in
  `docs/3mf-format-notes.md`.
- **Autosave path collision** (PR-5-10). On Linux,
  `~/.local/share/n3o-slic3r/autosave/` is the obvious
  location, but if the user has multiple n3o-slic3r
  instances open simultaneously they'd clobber each other.
  Either per-instance UUID in the filename or a per-project
  hash key — pick during implementation.
