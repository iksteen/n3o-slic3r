# OP-1 — layout + object list (foundation)

Status: 📋 planned

**Scope.** Add the left Objects panel as a third workspace column and
render the active plate's objects as a list, with two-way selection sync
to the 3D viewport. Material/override display is **read-only** here
(editing lands in OP-3); no add/remove (OP-2); no grouping (OP-4).

**What exists to build on:**
- Per-object snapshot data — `core/scene/commands.rs` `PlateSnapshot`
  (`objects: Vec<SceneObject>`, `object_overrides`, `material_to_slot`,
  `selection`); `SceneObject` carries id, name, transform, extruder_id.
- Selection — `scene_select` / `scene_deselect`; snapshot `selection`.
- Filament resolution — `useFilamentCatalog()` + `material_to_slot` +
  the bound instance's slots (as `SlotBindingPanel` already does).

**Net-new / changes:**
- No backend change. No row thumbnail — dropped entirely (the mockup's
  `kind`-derived badge isn't needed; the name identifies the object).
- `ObjectsPanel.tsx` ported from the mockup (read-only-capable): rows
  lead with the object **name + filament colour tag**, then material
  badge [read-only], position, override-count badge; plus the
  plate-stats footer. (No filament-name text in the row — the colour tag
  + material badge carry it.)
- Layout: `.workspace` grid 2→3 columns (add ~240px left); insert the
  panel in `App.tsx`; responsive adjustments. Read-only in Preview.
- Wire row click → `scene_select`; row highlight follows snapshot
  `selection` so viewport ↔ panel stay in sync both ways.

**Acceptance criteria:**
- The active plate's objects appear in the left panel; object count +
  plate size show in the footer.
- Selecting in the panel highlights in the viewport and vice-versa.
- Each row shows the object's resolved filament color/label + override
  count. Preview renders the panel read-only.
- No regression to the settings panel column.
