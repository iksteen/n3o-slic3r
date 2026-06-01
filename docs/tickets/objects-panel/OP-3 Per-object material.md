# OP-3 — per-object material assignment

Status: 📋 planned

**Scope.** Make the row material badge interactive (the mockup's
`MaterialPicker`), reusing the existing materials / `material_to_slot`
concept (index scope decision 1) — no new material entity.

**What exists:** `material_to_slot` routing + `project_set_material_slot`
/ `project_clear_material_slot`; the "Materials" section in
`SlotBindingPanel`; the filament catalog for swatches/labels.

**Net-new / changes:**
- `scene_set_object_material(object_id, material_index)` — sets the
  object's `extruder_id` and emits the object-updated event. This is the
  one missing mutation (confirmed absent).
- Badge → picker popover: **Assign** lists the plate's existing materials
  (each showing its slot routing + filament); **Create new material →
  route to slot** mints the next material index, routes it via
  `project_set_material_slot`, and assigns it to the object.
- Stay consistent with `SlotBindingPanel`'s Materials surface (same
  numbering, same routing table) so the two views never disagree.

**Acceptance criteria:**
- Changing an object's material updates its badge + the viewport color,
  and the slice routes that object through the chosen material's slot.
- Creating a material routed to a slot appears in both the picker and the
  `SlotBindingPanel` Materials section.
