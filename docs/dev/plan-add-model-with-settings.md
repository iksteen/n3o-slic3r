# Plan: settings-aware 3MF model import ("Add model + settings…")

Import the models of a foreign (Orca/BBS) `.3mf` into the open project
*with* their effective settings: the source project's plate-level config,
diffed against the current plate's resolved cascade, applied to every
imported object; the source's per-object settings merged over that.

Concrete driving case: with `m5sticks3_click_case_named_badges.3mf` open,
"Add model + settings…" `FloW_Arcspin_ORANGECON_orange.3mf` and get the
badge with its proper walls/support/layer settings — not the plate's.

## Semantics

- **"Add model…"** (existing): geometry only — objects, transforms,
  groups, paint, extruder hints. Stops importing `model_settings.config`
  object overrides (today it silently applies them). Drag-drop
  (`ModelDropZone`) follows this plain path.
- **"Add model + settings…"** (new, `.3mf`-only picker): per imported
  object, the effective override map is
  `diff(source project_settings vs current plate's resolved cascade)`
  restricted to object-applicable keys, overlaid by that object's own
  `model_settings.config` entries (object's key wins). Object-scope keys
  route to the imported group (multi-volume models), region-scope keys to
  members — same invariant as `object_override_set`'s routing.

## Backend

1. `core/scene/commands.rs` — `scene_load_3mf`: delete the
   `apply_imported_object_overrides` call (plain add becomes
   settings-free). `orca_import` (Open project) keeps its own call.
2. New `import_settings: bool` param on `scene_load_3mf`. When set:
   - Parse `Project3mf.embedded_settings` with the existing
     `OrcaProjectSettings::parse` (`core/orca_import`). No embedded
     settings → load geometry anyway, return a warning flag (a vanilla
     non-Orca 3MF must not hard-fail the picker).
   - **Baseline**: resolve the active plate's cascade via the same
     internal path `plate_cascade_resolve` uses (instance + bed +
     nozzles + quality profile) → `key → value` map.
   - **Candidate keys**: source project-settings keys passing
     `schema::is_object_overridable`, minus filament-index-valued keys
     (`support_filament`, `wall_filament`, …) whose values point into
     the *source* project's filament table.
   - **Diff**: normalize both sides through an FFI config round-trip
     (`slic3r_config_set` → `opt_serialize`) so `0.2` vs `0.20` doesn't
     create phantom overrides; keep keys whose normalized values differ
     from the baseline.
   - **Merge + route per object**: diffed plate map, then
     `obj.overrides` over it (gated). Object-scope-only keys →
     `groups[g].overrides` for grouped imports (entry created on
     demand); everything else → member `object_overrides`. Solo objects:
     all to the member map.
   - Emit `GroupOverridesChanged` / `ObjectOverridesChanged` alongside
     the existing `ObjectAdded` events; log a one-line applied/dropped
     report.
3. Verify `model_settings.config` layering (`core/threemf/bbs_meta.rs`):
   source ModelObject-level config should land on the imported *group*,
   per-part config on members. Fix the gate's routing there if it
   currently flattens onto members.

## Frontend

4. `src/objects/objectCommands.ts`: `loadModelWithSettingsFromDialog()`
   — `openFile` filtered to `["3mf"]`, invoke with the flag.
5. `src/objects/ObjectsPanel.tsx`: menu item "Add model + settings…"
   directly under "Add model…".

## Tests

6. Pure unit test for diff+merge+route: (baseline, project settings,
   object settings, grouped?) → expected group/member maps — covers
   normalization, filament-key drop, print-scope drop.
7. Integration: fixture Orca 3MF with `import_settings` into a
   bambi-bound project → group carries the diffed object-scope keys,
   members their own; plain path asserts NO overrides stored
   (regression for step 1).
8. Manual acceptance: the driving case above — check the group's
   overrides in the panel, slice.

## Decisions

- **Filament settings are dropped, on principle.** n3o owns the printer
  and the filament (instance + slot bindings + fragments); a 3MF has
  nothing to say about either. The object-applicable scope gate already
  excludes the Filament/Machine buckets structurally — this is the
  model, not a v1 limitation.
- **Filament-index-valued keys dropped too** (`support_filament`,
  `wall_filament`, …): object-scope keys whose *value* points into the
  source project's filament table, which doesn't exist here. Check the
  FFI option defs for a structural "value is a filament index" signal
  before resorting to a named suffix rule (no hardcoded
  classifications); if none exists, a small suffix rule with a
  `ponytail:` comment.
- **Diff, not pin**: keys matching the plate at import time follow later
  process changes on the plate. Accepted.
- **Plain-add behavior change is user-visible**: a 3MF that used to
  arrive with its object settings now arrives bare. Intentional.
