# OP-3 — per-object material assignment

Status: ✅ done 2026-06-01

**Scope.** Make the row material badge interactive (the mockup's
`MaterialPicker`), reusing the existing materials / `material_to_slot`
concept (index scope decision 1) — no new material entity.

**What exists:** `material_to_slot` routing + `project_set_material_slot`
/ `project_clear_material_slot`; the "Materials" section in
`SlotBindingPanel`; the filament catalog for swatches/labels.

**Net-new / changes:**
- `scene_set_object_material(id, material)` — sets the object's
  `extruder_id`, auto-binds the material to a slot if it had none
  (`ensure_default_material_slot_on_active`, mirroring the add path), and
  emits `ObjectUpdated` + `MaterialSlotChanged`. The one missing mutation.
- Badge → picker popover (`MaterialPicker.tsx`): **Assign** lists the
  plate's existing materials (each showing its slot routing + filament);
  **+ New material → route to slot** mints a material index, routes it via
  `project_set_material_slot`, and assigns it to the object.
- Stays consistent with `SlotBindingPanel`'s Materials surface (same
  `referencedMaterials` numbering, same routing table).

**Refinements (from review):**
- Swatches key on **filament identity**, not the raw spool colour — a
  cached colour with no identity (the Bambu external feed, no RFID, after
  unload) reads as the hollow/dashed empty orb, not a solid swatch.
- "+ New material" picks the **lowest unused** material index (reuses a
  freed gap), not always `max + 1`.
- "+ New material" is **hidden when the plate has a single object** — a
  new material would just orphan the old one.

**Acceptance criteria:**
- Changing an object's material updates its badge + the row colour, and
  the slice routes that object through the chosen material's slot.
- Creating a material routed to a slot appears in both the picker and the
  `SlotBindingPanel` Materials section.
