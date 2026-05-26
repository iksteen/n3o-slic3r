# PR-7 follow-up — Prime/wipe tower as a visible, movable scene object

Status: ❌ open (icebox, scheduled when plater UX work lands).

**Scope.** Today the prime/wipe tower has no scene-state
representation. libslic3r places it via `wipe_tower_x` / `wipe_tower_y`
config keys, which we now pin per-printer (back-left corner of the
bed) to keep the slice from putting it past the build-area boundary.
Users can't see where the tower will end up before slicing, can't
move it out of the way of an object, and can't tell whether their
plate layout collides with it.

Captured during Phase 7a real-print smoke: the A1 mini's compiled-in
`wipe_tower_y = 220` default put the tower 40 mm past the back wall
of the 180 × 180 bed; the print started, the Y axis ran into its
stop, and the steppers ground audibly. Band-aid fix in
`profiles/vendor/bbl/printer/bambu-lab-a1-mini/machine.toml` pins
`wipe_tower_x = 5`, `wipe_tower_y = 130`. The fix is per-printer and
silent — exactly the kind of correctness-by-magic-number this ticket
exists to replace.

**Acceptance criteria.**

- **Scene representation**: new `SceneObject` variant (or sibling
  type — `PlateFixture::PrimeTower { width, depth, position }`?)
  representing the tower's planned footprint. Visible in the
  Three.js viewport as a labeled volume distinct from print
  objects.

- **Single tower per (plate, printer)**. The tower is plate-scoped,
  not project-scoped — different plates can place it differently
  on the same printer.

- **Drag + snap**: standard transform gizmo for translation in
  X / Y only (Z is always at bed). Snap to bed grid like
  ordinary objects. Cannot rotate or scale — the dimensions are
  cascade-driven (`prime_tower_width`).

- **Cascade composer reads tower position from scene state**:
  emit `wipe_tower_x` / `wipe_tower_y` as a synthesized rule
  in the composer, taking values from the active plate's
  tower position. The per-printer band-aid in machine.toml goes
  away once this lands.

- **Pre-slice gate validates the tower fits the bed**: refuse
  the slice with a clear error if the tower (including its
  brim) extends past `printable_area`, exactly the way the gate
  refuses an object placed off-bed today.

- **Auto-place on first plate creation**: when a plate is bound
  to a printer for the first time, pre-position the tower in the
  back-left corner with brim margin — same default the band-aid
  pins now, but as the initial scene-state value rather than a
  cascade constant.

- **Visibility toggle on the cascade-level "enable_prime_tower"
  setting**: when disabled (e.g. single-material print), the
  tower object is hidden + non-interactive but the scene state
  is preserved so re-enabling restores the user's last position.

- **U1-specific**: the U1's `change_filament_gcode` doesn't use
  a prime tower for color changes — each toolhead has its own
  filament. But it does use a small primer for toolhead re-entry
  stabilization (per PRD §AD-1: priming + purging are independent
  capabilities). Decide whether the U1 surfaces a tower object
  or just hides the affordance. Probably the latter.

**Effort.** ~3–5 days. Touches scene state, renderer, transform
gizmo, cascade composer, pre-slice gate, and the binding panel.

**Dependencies.**

- Existing scene-object model + transform gizmo (Phase 2).
- Existing cascade composer's synthesized-rule pattern (PR-S-5).
- Existing pre-slice gate (PR-S-7).

**Out of scope.**

- Tower geometry customization beyond `prime_tower_width` —
  height tuning, infill density, ironing settings remain cascade-
  driven via existing process keys.
- Multi-tower (more than one tower per plate) — single tower
  matches libslic3r's model.
- Re-positioning the tower for the SAME plate on different
  printers — the scene-state position is plate-scoped (which
  matches the per-printer-config-key shape libslic3r expects per
  plate config in the 3MF).
