# Phase 4 exit-criteria smoke

Walks the project's Phase 4 deliverables (cascade-aware Settings
UI) end-to-end on a clean checkout. Mirrors `phase-3-smoke.md` and
its predecessors — half automated (Rust + frontend tests), half
human-driven (UX verification of the panel itself, which needs a
real GUI session + an external study to honor the 5-user gate).

Phase 4's UX exit criterion is **5/5 users identify the source
layer of a non-default setting within 10 seconds**. That's a
documented activity, not an in-CI assertion. The structural
exit gates the smoke can mechanize: A1 mini + U1 panel coverage,
introspection completeness, < 50 ms render budget for typical
category-pane changes.

## Automated half — runs in CI

```
$ cargo test --workspace
$ npm test
```

Expected counts:

| Suite | Tests | Notes |
| --- | --- | --- |
| (Phase 0-3 baseline) | 259 | unchanged |
| capability predicates | 6 | PR-4-1 |
| OptionSummary printer-aware | 4 | PR-4-1 |
| phase4_smoke | 3 | this file |
| frontend vitest (Phase 4 helpers) | 25 | helpers + categorize + filter + diff + layers + slots + annotations + tooltip + ladder |

Total: ~265 Rust tests + ~88 frontend tests, all green.

## What `phase4_smoke.rs` exercises

Three Rust integration tests cover the structural exit gates:

1. **Backend introspection coverage** (PR-4-1): asserts
   `OptionSummary` carries `mode` + `scope` + `tooltip` + `capability`
   for representative options (`layer_height` is Simple-mode, object-
   scope, has a tooltip, no capability; `wall_filament` is
   region-scope).
2. **A1 mini + U1 capability filter** (PR-4-1 + PR-4-5): asserts
   `slicer_options_for_printer` hides toolchanger geometry on the
   A1 mini and hides purge-tower geometry on the U1, with the
   correct `CapabilityPredicate` attribution. Cross-checks the
   inverse cases (A1 mini SHOWS purge tower; U1 SHOWS toolchanger).
3. **Render-budget gate** (PR-4-4): asserts
   `slicer_options_for_printer` returns ≥ 400 options in < 500 ms
   in CI's debug build — 10× headroom on the FR-UI 50 ms panel
   re-render budget so a single backend invocation never dominates.

## What the frontend vitest covers

Per-ticket pure-helper tests (no DOM render) since the project's
vitest pattern is pure-logic-only (matches the
`src/slice/reducer.test.ts` precedent):

- `inputs/helpers.ts` — `commitNumber` / `commitPercent` /
  `commitFloatOrPercent` / `commitColor` / `commitVectorEdit` /
  `padVector` / `parseBool` round-trips with the bounds + sync +
  wrap-extend semantics.
- `nav/categories.ts` — `categorize` order + `passesMode`
  inclusion + `categoryCounts` derivation.
- `SettingsPanel`'s pure `filterRow` — mode + search + capability
  hide combinations.
- `layers.ts` — `winningLayerFor` object-beats-project semantics
  + LAYER_HUE palette pinning.
- `diff.ts` — `computeDiff` + `passesDiff` All / from-default /
  from-save.
- `slots/SlotTabStrip.tsx` — vector commit at active slot with
  sync ON / OFF.
- `annotations/data.ts` — catalog floor of 30 entries, per-entry
  length cap, canonical-key coverage.

## Manual half — human verification

PR-4-13's panel isn't yet mounted into App.tsx (Phase 5's project
model owns the wiring). The visual gates below are reviewable
against the docs/design/ mockup until App.tsx integration lands:

1. **Tweaks vs deliverables.** Open
   `docs/design/n3o-slic3r-standalone.html` in a browser. Confirm
   the production tickets only lift the `accountability === "rule"`
   and `search === "instant"` variants — the breadcrumb and
   scoped/fuzzy modes stay in the mockup as comparison tweaks.

2. **Printer-aware visibility.** Switch the mockup's printer chip
   between Bambu A1 mini and Snapmaker U1; confirm the toolchange
   geometry options elide on A1 mini and the purge tower options
   elide on U1. (Production behavior verified by
   `phase4_smoke.rs`'s second test.)

3. **Slot-adaptive layout.** With a `slot_count >= 2` printer
   active, confirm the slot tab strip renders and the Sync
   toggle defaults ON. Edit a vector option in slot 2: with sync
   ON, all four slots take the same value; with sync OFF, only
   slot 2 changes.

4. **Source disclosure (the rule + ladder).** On a project where
   2-3 settings are overridden:
   - The overridden rows have a soft purple (project tier) or
     rose (object tier) background tint at rest.
   - Hovering an overridden row shows the 3 px inset rule on
     the left edge in the winning layer's hue.
   - Hovering for ~200 ms opens the cascade ladder portal to
     the left of the row. The ladder shows the seven cascade
     layers, the winning one highlighted + checkmarked, others
     showing em-dash for undefined.
   - Hovering the row label opens the SettingTooltip with
     libslic3r's text + (where present) a "💡 tip" annotation.

5. **Editing-context tabs.** With an object selected, switch to
   the Object tab. Override a setting; the row shows the rose
   tint + bold name. Switch back to the Project tab; the same
   row's objects-overriding badge appears with the object's
   filament color dot.

6. **Build plate selector.** With a printer that declares a
   default plate, switching plates clears the `printer default`
   badge; selecting the default value restores it.

7. **5-user UX study (the actual exit gate).** Recruit 5 users
   unfamiliar with the project. Show each a fixture project with
   ~5 settings overridden (mix of project + object tiers). Ask:
   *"This setting is `value`. What changed it from the default?"*
   The study passes when 5/5 users correctly identify the source
   layer within 10 seconds — typically via the breadcrumb-or-
   ladder pairing.

   Record results back into this doc under `## Study results`
   when run.

## What's not covered by the smoke

- **App.tsx integration.** PR-4-4 ships the panel scaffold but
  doesn't mount it in App.tsx; that wiring lives with Phase 5's
  project model (cascadeHandle, ContextJson, and selected-object
  state aren't carried at the App.tsx level today).
- **Inline validation.** PR-4-11 ships the tooltip surface; the
  `slicer_validate_option` Tauri command + per-row error wiring
  defer to a follow-up.
- **Backend per-object override storage.** PR-4-9's frontend
  surface is in place; SceneState's `object_overrides` map +
  `scene_object_override_set/clear/clear_all` commands defer to
  Phase 5.
- **Cascade-layer tagging for the rule's full palette.** PR-4-7
  ships the rule + tint for the override tiers (project / object).
  Cascade-tier layers (printer / build_plate / filament / user)
  all share the "cascade" treatment until Phase 5's profile
  registry tags the authored cascade files with their source
  layer.

## If a step fails

- **Rust tests red:** `cargo test --workspace -- --nocapture`
  surfaces the failure messages.
- **Vitest red:** `npm test -- --reporter=verbose`.
- **Panel mismatched with mockup:** open `docs/design/index.html`
  to compare. The phase-4.md design reference section pins which
  CSS class names, hue palette, and tweak defaults the production
  code mirrors.
- **5-user UX study fails:** the failure mode the panel was
  designed to prevent is "user can't find the source." If 1-2
  users miss the breadcrumb-or-ladder, the disclosure may not be
  prominent enough — Phase 9 polish iterates. If 3+ miss, the
  cascade architecture is at odds with user mental models and
  needs deeper rework.
