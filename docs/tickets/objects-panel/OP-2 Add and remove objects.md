# OP-2 — add / remove objects from the library

Status: 📋 planned

**Scope.** The object-library popover and add/remove wiring.

**What exists:** `scene_object_add_from_primitive`
(cube/cylinder/sphere/cone/torus), mesh/STL load
(`scene_load_mesh_from_path` / `scene_load_3mf`), `scene_object_delete`.

**Net-new / changes:**
- Library popover (mockup's `OBJECT_LIBRARY`): Primitives section wired to
  `scene_object_add_from_primitive`; an "Add STL…" entry → file dialog →
  mesh load. (Calibration / Imported sample-asset sections and
  drag-to-canvas are optional follow-ons, not blocking.)
- Per-row remove button → `scene_object_delete`; empty-state hint.
- New object auto-selects and lands at a tidy plate position (the
  existing add path already places it).

**Acceptance criteria:**
- Adding a primitive or an STL from the library puts it on the active
  plate, selected, visible in both the panel and the viewport.
- Removing an object clears it from both; selection updates sanely.
