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
`profiles/bbl/printer/bambu-lab-a1-mini/machine.toml` pins
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

## Tower-dimension semantics — pick a visualization path

The tower's X width is a cascade constant (`prime_tower_width`,
35 mm in both bundled printers today). The Y depth is **computed
per layer at slice time** from purge volume — see
`external/OrcaSlicer/src/libslic3r/GCode/WipeTower2.cpp:2317`
(`required_depth = ramming_depth + depth_to_wipe`, where
`depth_to_wipe` is a function of `max(filament_minimal_purge_on_wipe_tower,
wipe_volume_total - savings)` and divided by the tower width).

The mode driver is `single_extruder_multi_material`:

| | A1 mini (SEMM = 1) | U1 (SEMM = 0) |
|---|---|---|
| X (width) | `prime_tower_width = 35` | `prime_tower_width = 35` |
| Y (depth) | computed per layer; driven by `flush_volumes_matrix` (cross-color purge volume — substantial) — ends up **~width** (looks square) | computed per layer; driven by `filament_minimal_purge_on_wipe_tower` only (no cross-color purge on a toolchanger) — ends up a **small fraction of width** (looks like a thin strip in Y) |
| Wall geometry | rectangle (default) | rib (4 stabilizing ribs around a hollow walled tower; `wipe_tower_wall_type = "rib"`) |
| Brim | `prime_tower_brim_width = 3` | `prime_tower_brim_width = 5` |

Other shape knobs that exist but neither bundled printer uses:
`wipe_tower_wall_type = "cone"` + `wipe_tower_cone_angle` for a
tapered tower, and `wipe_tower_extra_rib_length` to scale rib
length explicitly.

### Three visualization paths to decide between

1. **Conservative bounding box (pre-slice)**: render
   `prime_tower_width × prime_tower_width` regardless of mode.
   Over-reserves on the U1 (most of the bounding-box Y is empty)
   but never under-reserves; pre-slice gate validates against
   this conservative box.

2. **Mode-aware estimate (pre-slice)**: branch on
   `single_extruder_multi_material`. SEMM = 1 → estimate Y ≈ X.
   SEMM = 0 → estimate Y from
   `filament_minimal_purge_on_wipe_tower × N_filaments /
   prime_tower_width`. More accurate but still an estimate; user
   may find actual tower a few mm different from the indicator.

3. **Post-slice rendering**: parse `WIPE_TOWER_START` /
   `WIPE_TOWER_END` markers from the produced gcode and draw the
   real bounding box. Accurate but only useful after slicing —
   no help during plate layout.

**Likely best UX**: (1) + (3) combined. Conservative box during
layout (gives the user a "stay away from here" hint), then
redraw with the actual rectangle after slicing so they can
confirm placement before sending. Defer (2) unless pre-slice
accuracy becomes a UX complaint.

The cascade composer needs to emit `wipe_tower_x` / `wipe_tower_y`
based on the scene-state position regardless of which visualization
path we pick — those values land in the slice-time config either
way.

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
