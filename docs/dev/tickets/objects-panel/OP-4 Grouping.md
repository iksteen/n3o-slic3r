# OP-4 — grouping (unified with multi-volume)

Status: ✅ done 2026-06-01

**Design decision (2026-06-01): user groups ARE multi-volume objects.**
`SceneObject.group_id` already means "these objects are volumes of one
logical print object" — the writer emits a shared `group_id` as a single
`ModelObject` with multiple `ModelVolume`s, and the slice path treats it
as one object. That is exactly "assemble selected objects into one
multi-part object". So user grouping **reuses `group_id`** rather than
adding a parallel field: 3MF multi-volume objects and user-created groups
are the same thing and render identically in the panel.

The only genuinely new state is a **group name** (the model has
per-object names but no per-group name).

**What exists:** `SceneObject.group_id`; the writer/slice path that
treats a shared `group_id` as one ModelObject; project save/load that
round-trips objects (incl. `group_id`) through `n3o_project.json`.

**Net-new / changes:**
- Per-plate `group_names` map (`group_id → name`), serialized in
  `n3o_project.json` (default empty) and surfaced in `PlateSnapshot`.
  (The grouping itself already persists as multi-volume geometry; this
  just carries the user label.)
- Mutations + commands:
  - `scene_group_objects(ids, name)` — allocate a new project-scoped
    `group_id`, set it on the selected objects, store the name.
  - `scene_ungroup_objects(group_id)` — clear `group_id` from members,
    drop the name.
  - `scene_rename_group(group_id, name)`.
- Snapshot: surface `group_id` per object (the TS `SceneObject` type is
  currently missing it) + the per-plate `group_names`.
- Panel: multi-select (⌘/Ctrl/Shift-click) + a group action bar; objects
  sharing a `group_id` render as a collapsible block (caret, member
  count, swatch stack, rename, ungroup); the group header selects all
  members. A group left with one member dissolves.

**Acceptance criteria:**
- Multi-select two-plus objects → Group makes a named group that
  collapses, renames, and ungroups; it survives save/reload.
- A 3MF multi-volume object loads as a group in the panel (same render
  path).
- A grouped set slices as one object — the existing `group_id`
  behaviour, unchanged.
