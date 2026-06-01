# OP-4 — grouping

Status: 📋 planned

**Scope.** Manual object grouping in the panel: multi-select →
group/ungroup, rename, collapsible group blocks.

**Design note — user groups vs multi-volume `group_id`.** The existing
`SceneObject.group_id` is reserved for 3MF multi-volume objects (volumes
of one logical object), **not** user grouping. User groups likely need a
distinct field (e.g. `user_group_id`) plus a per-plate `group → name`
map, so the two concepts don't collide. Settle this in the ticket before
coding.

**Net-new / changes:**
- Backend: per-plate user-group model — group id allocation, a
  `group_id → name` map, and commands `scene_group_objects` /
  `scene_ungroup_objects` / `scene_rename_group`. Persist in the `.3mf`
  (the `n3o_project.json` side). Orphan-group dissolve (a group of one
  isn't a group) on delete/ungroup.
- Snapshot: surface the user-group id + name per object.
- Panel: multi-select (⌘/Ctrl/Shift-click), the group action bar, group
  blocks (caret/collapse, member count, swatch stack, rename, ungroup);
  the group header selects all members.

**Acceptance criteria:**
- Multi-select two-plus objects → Group makes a named group; it
  collapses, renames, and ungroups; it survives save/reload.
- Grouping never clobbers 3MF multi-volume objects.
