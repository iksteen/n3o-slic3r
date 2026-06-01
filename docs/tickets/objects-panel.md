# Objects panel

The left-hand **Objects panel** — a per-plate object list with selection,
add/remove from a library, per-object material assignment, and grouping.
Designed in `docs/design/ObjectsPanel.jsx` (mockup) and a hard MVP gap:
the workspace today is viewport + settings only, with no way to see or
manage a plate's objects as a list. Implements the object-management half
of the Prepare workflow (FR-CAS-7b context).

Most of the backend already exists — per-object snapshot data, add
primitive / load mesh, delete, select/deselect, the filament catalog, the
`material_to_slot` routing, and per-object overrides. The work is mostly a
frontend port of the mockup + a 2→3-column layout, plus a few small
commands (per-object material set; the user-grouping model).

## Status by ticket

| Ticket | Scope | Status |
|--------|-------|--------|
| [OP-1](objects-panel/OP-1%20Layout%20and%20object%20list.md) | 3-column layout + read-only object list, selection sync, plate stats | 📋 planned |
| [OP-2](objects-panel/OP-2%20Add%20and%20remove%20objects.md) | Object library (primitives + STL load), add/remove | 📋 planned |
| [OP-3](objects-panel/OP-3%20Per-object%20material.md) | Per-object material badge + picker, set-object-material command | 📋 planned |
| [OP-4](objects-panel/OP-4%20Grouping.md) | Multi-select + group/ungroup/rename, user-group model + persistence | 📋 planned |

## Scope decisions

1. **"Material" is the existing concept, not a new entity** (2026-06-01).
   The object's material is its `extruder_id`, resolved to a filament
   through the plate's existing `material_to_slot` table — the same
   "Materials" the `SlotBindingPanel` already manages. The panel surfaces
   and edits that per-object; it does **not** introduce a separate
   material model. OP-3 adds only the missing per-object *set* command.

2. **All four tickets are MVP** (2026-06-01). Grouping (OP-4) is included,
   not deferred — split into its own ticket for sequencing, not a cut.

## Sequencing

OP-1 (foundation) → OP-2 (add/remove) → OP-3 (material) → OP-4 (grouping).
Each is independently shippable.
