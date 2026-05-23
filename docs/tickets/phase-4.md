# Phase 4 — tickets

Phase 4 (cascade-aware Settings UI, ~5 person-weeks) builds the
**primary differentiator** the PRD §5 calls out: settings that show
their source, that adapt to the active printer's capabilities, that
let any value be overridden per-object, and that make the cascade
visible rather than something the user has to mentally simulate.

Source: `docs/Execution_Plan.md` §6. Stated goal:

> Cascade-aware settings UI. Includes printer-aware visibility
> filtering, slot-adaptive layout for multi-extruder/toolchanger
> printers, hover-revealed cascade ladder, and first-class
> per-object override editing.

Individual tickets live one-per-file in `phase-4/`. This file is the
index plus phase-level status and notes.

## Status by deliverable

| Deliverable | Status | Ticket |
|-------------|--------|--------|
| Settings backend introspection enrichment (mode/scope/capability) | ✅ done | [PR-4-1](phase-4/PR-4-1%20Settings%20backend%20introspection.md) |
| Data-driven form component library (6 input types) | ✅ done | [PR-4-2](phase-4/PR-4-2%20Form%20components.md) |
| Category navigation + mode filter (Simple/Advanced/Expert) | ✅ done | [PR-4-3](phase-4/PR-4-3%20Category%20nav%20and%20mode%20filter.md) |
| Settings panel scaffold + editing-context tabs | ✅ done | [PR-4-4](phase-4/PR-4-4%20Settings%20panel%20scaffold.md) |
| Printer-aware visibility + build plate selector | ✅ done | [PR-4-5](phase-4/PR-4-5%20Printer-aware%20visibility%20and%20build%20plate.md) |
| Slot-adaptive layout (per-slot tab strip + sync-edit) | ✅ done | [PR-4-6](phase-4/PR-4-6%20Slot-adaptive%20layout.md) |
| Source-layer rule + authored-tier tint + override count badges | ✅ done | [PR-4-7](phase-4/PR-4-7%20Source-layer%20rule.md) |
| Hover cascade ladder (portal-rendered) | ✅ done | [PR-4-8](phase-4/PR-4-8%20Cascade%20ladder.md) |
| Per-object overrides + objects-overriding badge + reset | ✅ done (frontend) | [PR-4-9](phase-4/PR-4-9%20Per-object%20overrides.md) |
| Diff view (vs printer default, vs last save) | ✅ done | [PR-4-10](phase-4/PR-4-10%20Diff%20view.md) |
| Tooltips + inline validation | ✅ tooltip shipped; validation deferred | [PR-4-11](phase-4/PR-4-11%20Tooltips%20and%20validation.md) |
| Support toggle per object + first ~30 "why this matters" annotations | ✅ done | [PR-4-12](phase-4/PR-4-12%20Support%20toggle%20and%20annotations.md) |
| Phase 4 exit-criteria smoke | ❌ open | [PR-4-13](phase-4/PR-4-13%20Exit-criteria%20smoke.md) |

## Design reference + tweaks vs deliverables

A working visual mockup of the settings UX (and the broader app
shell that hosts it) lives in `docs/design/`. Open
`docs/design/index.html` (or the self-contained
`n3o-slic3r-standalone.html`) in a browser to interact with it —
React + Babel are loaded from CDNs; no build step.

**The mockup exposes designer A/B tweaks alongside the canonical
design — read the tweaks panel and the `TWEAK_DEFAULTS` block in
`docs/design/app.jsx` carefully before lifting anything to a
ticket.** The defaults are what ships; the alternative variants
are design experiments the designer can toggle in the mockup to
compare but are **not** Phase 4 deliverables.

Canonical defaults (from `TWEAK_DEFAULTS` in `docs/design/app.jsx`):

| Tweak | Canonical default (ship this) | Alternatives (do NOT lift) |
|---|---|---|
| `accountability` | **`rule`** — colored left rule on hover (and authored-tier background tint) | `breadcrumb` (inline chip trail), `ladder-only` (hover only, no rule) |
| `search` | **`instant`** — substring across all categories | `scoped` (current-category only), `fuzzy` |
| `theme` | **both `light` + `dark`** ship via `[data-theme]` | — |
| `accent` | **`cyan`** as starter | `ember` / `violet` / `mint` deferred to Phase 9 |
| `density` | **`regular`** | Compact variants deferred |

Concretely: PR-4-7 ships the **rule** (a 3px inset left-edge
`box-shadow` that's transparent at rest and reveals the winning
layer's hue on hover, plus the authored-tier background tint).
PR-4-7 does **not** ship the breadcrumb chip strip — that was an
earlier ticket title that lifted the wrong variant. The cascade
ladder (PR-4-8) is the always-available companion to the rule;
together they cover FR-CAS-7's "show the source" requirement.

The implementer must consult the mockup before writing each
ticket's component. The mockup is the canonical source for:

Implementer must consult the mockup before writing each ticket's
component. The mockup is the canonical source for:

- **Cascade-layer vocabulary** (`docs/design/data.jsx`): the seven
  layers (`default` / `printer` / `build_plate` / `filament` /
  `user` / `project` / `object`), each with a stable `id`, a
  three-letter `short` code (used only when the breadcrumb tweak
  is active; not shipped in default UI), an HSL `hue` for the
  rule + tint + ladder, and a `desc` string. The Phase 4 tickets
  reuse the `id`s and `hue`s verbatim — match `data.jsx`, don't
  reinvent.
- **Settings-row anatomy** (`docs/design/SettingsPanel.jsx`,
  `SettingRow` function): label + (tweak-only, NOT shipped)
  breadcrumb chip strip + objects-overrides badge + reset
  button + value control + the hover cascade ladder rendered via
  React portal. Ship the **rule** (3 px inset left-edge
  `box-shadow` revealing the winning layer's hue on hover, plus
  the `.authored-project` / `.authored-object` background tints
  that mark the row at rest). Keep the CSS class hooks
  (`.set-row`, `.objs-badge`, `.cascade-ladder`, `.ladder-row`,
  etc.) from `docs/design/styles.css` so designers can iterate
  on styling against the mockup. Do NOT carry `.set-breadcrumb`
  / `.crumb` into production — they're gated behind the
  `accountability === "breadcrumb"` tweak.
- **Panel layout** (`SettingsPanel.jsx`, root component): config
  strip (printer / bed-plate / nozzle / filament chips) → editing-
  context tabs (Project / Object) → search bar → category rail
  on the left with overrides count badges per category → scroll
  body of `cat-group` sections with their settings.
- **Theming and palette** (`docs/design/styles.css`): `oklch()`
  color space, `--accent` / `--accent-soft` / `--accent-text`
  custom properties, `--row-hue` per-row CSS var bound to the
  cascade layer's `hue`. Light and dark themes are both supported
  via `[data-theme]` attribute; PR-4-4's scaffold inherits this.

What the mockup does **not** capture (intentionally — it's a
fidelity reference, not the implementation):

- Wire-format types (mockup uses ad-hoc JS objects; production
  uses the typed `OptionSummary` from PR-4-1 + the cascade
  resolver from Phase 1).
- Tauri command surface (mockup is all in-browser; production
  routes through `cascade_resolve`, `cascade_trace`,
  `scene_object_override_set` etc.).
- Capability-driven hide/show (mockup shows all settings;
  PR-4-5 enforces the printer-aware filter).
- Validation against libslic3r `config_validate` (mockup accepts
  any value).
- Cascade size: the mockup carries a small hand-authored
  `ALL_SETTINGS` fixture; production drives off the ~800 libslic3r
  options surfaced through `slicer_options`.

When a ticket reads "follows the mockup's …" — that means: clone
the mockup's class names, prop shapes, and visual structure
literally where possible; the divergences above are the only
parts where production legitimately differs.

## Architecture invariant — the cascade is visible, not simulated

Phase 1 built the cascade resolver that produces a **trace** for every
setting (winning rule + losers + override-tier source). Phase 4 is
where that trace becomes user-facing: every settings row carries the
breadcrumb, every hover opens the ladder, every override surfaces a
reset action that reveals what the cascade would resolve to without it.

The invariant this establishes for all future phases: **no setting
shall appear in the UI without its source attribution.** A
"quick-edit fast path" that lets the user mutate a value without
visible cascade context erodes the primary differentiator. If the
ladder is too slow for some hot path, optimize the ladder; do not
hide it.

The settings panel is also the **first major UI surface** beyond
Phase 2's viewport. It establishes the project's component
conventions (form inputs, category navigation, contextual
disclosure) that Phase 5+ will reuse.

## Dependency graph

```
PR-4-1 (backend introspection — mode, scope, capability flags)
  └── PR-4-2 (form components — consume schema metadata)
       └── PR-4-3 (category nav + mode filter)
            └── PR-4-4 (panel scaffold + editing-context tabs)
                 ├── PR-4-5 (printer-aware visibility + build plate)
                 ├── PR-4-6 (slot-adaptive layout)
                 ├── PR-4-7 (source-layer breadcrumb)
                 │    └── PR-4-8 (cascade ladder — extends breadcrumb)
                 ├── PR-4-9 (per-object overrides + badge + reset)
                 ├── PR-4-10 (diff view)
                 ├── PR-4-11 (tooltips + validation)
                 └── PR-4-12 (support toggle + annotations)

PR-4-1..-12 ──► PR-4-13 (exit smoke needs the full surface)
```

The single critical-path bottleneck is **PR-4-4** (panel scaffold).
PR-4-5 through PR-4-12 can land in any order once the scaffold is up.

## Exit criteria for the phase (from Execution Plan §6)

- **5-user UX test passes:** given a project where a value differs
  from default, 5/5 users identify the source layer within 10
  seconds — by reading the inline breadcrumb or by hovering for the
  ladder.
- **A1 mini and U1 both render their full settings panel correctly:**
  A1 mini hides toolchange options, U1 hides purge volumes matrix;
  both show priming tower geometry settings; U1 shows 4-slot tab
  strip while A1 mini shows single pane.
- **Per-object overrides:** editing a setting in the Object tab
  affects only that object; the project tab's row for the same
  setting shows the objects-badge with the object's color dot.
- **Render perf:** settings panel re-renders under **50 ms** on
  cascade change (single-slot) and under **100 ms** (4-slot).

## Cut candidates (from Execution Plan)

If pressed for time:

- **Diff view (PR-4-10)** → saves ~2 days. Smoke still demonstrates
  override visibility via the breadcrumb + ladder.
- **"Why this matters" annotations beyond the first 30** (PR-4-12
  sub-deliverable) → saves ~3 days. First 30 ship; remainder lands
  iteratively in Phase 9.
- **Synchronized-edit affordance on multi-slot tab strip** (PR-4-6
  sub-deliverable) → users edit each tab independently. Saves ~2
  days. Hurts UX for the common "configure all toolheads identically"
  case; cut last.
- **Objects-overriding-this badge with per-object click-through in
  ladder** (PR-4-9 sub-deliverable) → keep the badge, cut the
  click-through. Saves ~1 day.

## What's *not* in Phase 4

- **Profile registry + cascade authoring UI** — Phase 5/9. Phase 4
  consumes the cascade-resolved values; cascade *editing* (authoring
  TOMLs, importing OrcaSlicer profiles, validating predicates in a
  GUI) is post-MVP. Users hand-edit `profiles/cascades/*.toml`
  through the file system for now.
- **Multi-plate UI** — Phase 5. Phase 4 operates on a single plate;
  the settings panel binds to the active plate's printer.
- **Paint-on supports / paint-on seam** — post-MVP (PRD §10).
  PR-4-12 ships the simple on/off support toggle only.
- **Filament profile editor** — Phase 7c. Phase 4 lists the active
  filament's resolved settings as a tier in the cascade ladder; it
  doesn't let users edit the filament profile itself.
- **G-code preview integration** — Phase 6. Settings changes don't
  trigger a preview re-render in Phase 4; that's deferred to when
  the preview exists.
- **Send-to-printer / driver UI** — Phase 7. Settings panel ends at
  the slice button (still Phase 3's button); driver UX is later.

## Open questions seeded for the implementer

- **Capability flags vocabulary (PR-4-1, PR-4-5).** The
  printer-aware visibility filter (FR-UI-7) needs a typed list of
  printer capabilities (`has_toolchanger`, `has_purge_tower`,
  `slot_count`, `supports_relative_e`, `has_chamber_heater`, etc.)
  to drive per-option hide/show rules. Audit `external/OrcaSlicer/
  src/libslic3r/PrintConfig.cpp` for the union of `ConfigOptionDef::
  condition` predicates — those are the GUI's pre-existing
  capability tests we want to mirror.
- **Cascade trace cost (PR-4-8).** The cascade ladder needs the
  full trace per row, not just the resolved value. PR-1-5's
  `cascade_trace` returns `Vec<ResolvedEntryWithTrace>` in
  ~milliseconds for the canonical A1 mini cascade; verify it holds
  for the much larger OrcaSlicer-derived cascades (~338 keys per
  PR-0.5-1) before relying on per-row hover.
- **Setting scope enforcement (PR-4-9).** PR-3-11's PrintConfig
  scope work surfaced the project/object/region scope bitmask
  (`slic3r-ffi: expose option scope`). Use it to disable Object-tab
  editing for project-scope settings (per FR-3D-3); render with a
  `project-scope setting` badge.
- **Form-component perf at 800+ rows (PR-4-2, PR-4-4).** A
  fully-expanded category like "Speed" can have 50+ options;
  the full schema has ~800. Virtualized lists vs. plain-DOM render
  is the call to make once the scaffold is up; the 50ms re-render
  budget is the gate.
