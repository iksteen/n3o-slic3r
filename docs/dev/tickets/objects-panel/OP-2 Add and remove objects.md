# OP-2 — add / remove objects from the library

Status: ✅ done 2026-06-01

**Scope.** The object-library popover and add/remove wiring.

**What exists:** `scene_object_add_from_primitive`
(cube/cylinder/sphere/cone/torus), mesh/STL load
(`scene_load_mesh_from_path`), 3MF geometry load (`scene_load_3mf`),
`scene_object_delete`.

**Net-new / changes:**
- Panel header `+` → library popover: a Primitives section wired to
  `scene_object_add_from_primitive`, plus an **"Add model…"** entry →
  file dialog → load `.stl`/`.obj` via the mesh loader, or `.3mf` via
  `scene_load_3mf` (**geometry only** — objects + transforms + per-part
  extruder hints, *not* the project settings; that stays the separate
  "open project" import). `objectCommands.ts` holds the wrappers.
- Per-row remove button → `scene_object_delete`; empty-state hint.
- New primitive / single mesh auto-selects.
- `scene_object_add_from_primitive`'s `params` made optional — the
  quick-add omits it and the backend fills `defaults_for(kind)`, so the
  default sizes live in one place.
- Removed the now-redundant viewport **"+ Cube"** and **"Load…"**
  buttons — the panel is the single add/load surface (no duplicated
  format list / routing).

**Acceptance criteria:**
- Adding a primitive or loading a model (`.stl`/`.obj`/`.3mf` geometry)
  from the panel puts it on the active plate, visible in both the panel
  and the viewport.
- Removing an object clears it from both; selection updates sanely.
