# U1 pressure advance: n3o vs Snapmaker Orca

Investigation into different corner extrusion on the Snapmaker U1 between n3o
and Snapmaker's Orca fork. Ground truth in the repo root:
`cubes.3mf` (4-colour project) + `cubes_snorca.gcode` (fork slice) +
`cubes_snorca.klippy.log` (printed: bed leveling on, flow calibration off,
corners **good**) + `src-tauri/cubes_n3o.gcode.3mf` (n3o slice of the same file).

## Root cause: n3o bakes a much higher pressure advance — CONFIRMED

Both slicers emit `SET_PRESSURE_ADVANCE` into the gcode; the values differ ~2.5×.

| | enable_pressure_advance | baked SET_PRESSURE_ADVANCE | source |
|---|---|---|---|
| **n3o** | `1,1` | `0.048` / `0.05` (271×) | filament profile `pressure_advance`, emitted because the flag is on |
| **snorca** | `0,0` | `0.02` (136×) | fork wipe-tower patch (`ramming_pressure_advance_value`), since the flag is off |

n3o resolves the project's filaments to upstream **generic** profiles —
`Generic PLA High Speed` (PA 0.048) and `Generic PLA` (PA 0.05) — whose pressure
advance is tuned for typical printers, ~2.5× the U1's ~0.02. Snapmaker's U1
filament profiles instead set `enable_pressure_advance = 0` (e.g. `Snapmaker PLA
SnapSpeed @U1`: flag 0, PA 0.02) so the slicer emits no filament PA, and the
fork's wipe-tower patch injects the conservative `ramming_pressure_advance_value`
(0.02). Net: n3o commands PA ≈ 0.05, snorca commands 0.02. Over-high pressure
advance over-retracts filament pressure into corners → the observed corner
underextrusion.

**Magnitude — large, because this is a high-speed profile.** PA retract at a
velocity change scales linearly with speed. Outer walls run at 200 mm/s (inner 300).
At 200 mm/s the 0.02→0.05 gap pulls back ~0.21 mm of filament (~0.5 mm³, ~6 mm of
outer-wall line) at *each* corner deceleration — severe, pervasive corner gaps on a
cube. The "0.02→0.05 is moderate" rule of thumb only holds near ~100 mm/s; check the
print speed before applying it. Not flow: n3o's `filament_flow_ratio` (0.98/0.98) is
equal-or-higher than snorca's (0.98/0.966), so overall extrusion is if anything
slightly *higher*, not lower — the deficit is corner-local, i.e. PA.

This is a **filament-profile divergence**, exactly parallel to the machine-profile
one we patched via `machine.override.toml`. Precise provenance: `0.05` is not a bug
or an insane default — it's the most-common `pressure_advance` across the 172
printer variants n3o consolidates into `generic-pla.toml` (a normal direct-drive
value). The fragment even carries printer-specific PA deltas (0.024, 0.025, 0.03,
0.031, …), but **none for the U1**, because upstream OrcaSlicer ships **no
`Generic PLA @U1` leaf**. Snapmaker's fork does (`Generic PLA @U1 0.N nozzle.json`,
`enable_pressure_advance=0`, PA `0.02`). So on the U1 the consolidated baseline
(0.05) leaks through unoverridden. n3o imports from upstream, which lacks the
U1-tuned filament leaves entirely.

### Theories disproven along the way (both by the ground-truth files)
- **`SET_PRINT_FILAMENT_CONFIG` → firmware `FLOW_RESET_K` → per-filament PA.**
  Wrong: the klippy.log shows both fired **0 times**. PA came purely from the
  gcode's `SET_PRESSURE_ADVANCE`.
- **The wipe-tower zeroing patch (`disable_linear_advance` → `ADVANCE=0`).**
  Real fork difference, but **not** the cause here: it only bites when
  `enable_pressure_advance = 0`, and n3o's profiles have it **1** — so n3o never
  zeroes PA, it bakes the (too-high) filament value. The wipe-tower patch is why
  snorca's `enable_pressure_advance=0` profiles still get 0.02 instead of 0; n3o
  wouldn't need it if its filament PA values were correct.

(Unrelated and still standing: the noodle-detection pre-print baseline finding,
`DEFECT_DETECT_NOODLE_FIRST` — see the other memory.)

## The regression (`print=true` → `SDCARD_PRINT_FILE_WITH_PARAMETERS`) — RULED OUT

Both printer logs now confirm PA is entirely **gcode-determined**, with no
firmware manipulation on either side:
- `cubes_n3o.klippy.log`: printer applied `pressure_advance: 0.048` / `0.050`
  (= the gcode), and `FLOW_RESET_K` / `SET_PRINT_FILAMENT_CONFIG` / `FLOW_CALIBRATE`
  appear **0 times**.
- `cubes_snorca.klippy.log`: printer applied `0.02` (= its gcode); same, 0 firmware
  PA activity.

Since PA comes purely from the sliced gcode, and the send path (`print=true` vs
`SDCARD_PRINT_FILE_WITH_PARAMETERS`) never touches the gcode, the send-method switch
**cannot** be the cause of the corner underextrusion. The cause is the baked PA
value (0.05 vs 0.02), a filament-profile matter, present regardless of how the print
is started. The original "since we switched" correlation is therefore a red herring
for this problem (likely a coincident profile/filament change, or different
pre-switch prints). Not chasing it further absent evidence of a *separate* symptom.

## Fix (for the extrusion difference)

**Implemented — a printer-owned PA table (a Rust variant of the §(d) idea).** PA is a
printer/kinematics property OrcaSlicer mis-stores on the filament, so each printer owns a
`pressure_advance.toml` keyed `[<material base_type>]` → `"<nozzle>" = K`
(`resources/profiles/snapmaker/printer/snapmaker-u1/pressure_advance.toml`, seeded from the
firmware `flow_k` values). It's loaded into `ProfileLibrary` alongside `machine.toml`, and
`profile_library::resolve_pressure_advance(slug, base_type, nozzle, calibrated)` (precedence:
on-printer calibration → table → `None`) is applied per slot in `composer.rs`
`assemble_filament_vectors`, overriding `pressure_advance` + setting `enable_pressure_advance=1`.
Nothing is edited in the filament fragments; printers with no table are unchanged.

It's done in Rust rather than as cascade `[[rule]]` blocks because PA/`enable_pressure_advance`
are Filament-bucket *vector* keys — a printer-side scalar rule gets clobbered by the per-slot
vector assembly or fails the vector-length check, and the clean `when.material.class` route
isn't built. The §(d) "Per-printer material-class authoring" plan is the eventual cascade-native
home; this resolver is the interim, and generalizes to any printer that ships the table.
- **Wipe-tower patch** (`enable_change_pressure_when_wiping` +
  `ramming_pressure_advance_value`, `WipeTower2.cpp`, tagged `// Snapmaker U1`) only
  matters if we move to `enable_pressure_advance = 0` (delegate PA to firmware);
  otherwise the profile value is what ships. Lower priority.
