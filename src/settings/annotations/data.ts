// "Why this matters" annotations (PR-4-12).
//
// Authored text per high-impact libslic3r option, surfaced by
// PR-4-11's SettingTooltip in the "💡 tip" section beneath
// libslic3r's own tooltip text. Aim for 2-4 sentences: what the
// setting controls in physical terms, the trade-off, and a quick
// rule of thumb.
//
// Authoring guidance:
// - Don't restate libslic3r's tooltip; it renders above the
//   "💡 tip" section already.
// - Prefer mechanical language ("0.2 mm layers print 2× faster
//   than 0.1 mm") over preset advice ("use 0.2 for PLA"). Users
//   own their own preset choices.
// - Cap each entry at ~4 sentences. Long explanations hurt
//   tooltip readability.
//
// The catalog covers the ~30 highest-impact options across the
// categories users touch most (Quality, Strength, Speed, Walls,
// Support, Adhesion). Phase 9 polish can extend further per
// PRD §6 cut-candidate language.

export const ANNOTATIONS: Record<string, string> = {
  // ─── Quality / Layers ─────────────────────────────────────────
  layer_height:
    "Vertical thickness of each printed layer. Lower = finer surface " +
    "detail and slower print (halving the layer height roughly doubles " +
    "print time). 0.2 mm is the typical FDM default; 0.1-0.12 mm for " +
    "visible-surface parts, 0.28-0.32 mm for fast drafts on a 0.4 mm nozzle.",

  initial_layer_print_height:
    "Thickness of the first layer specifically. Slightly higher than " +
    "the rest helps bed adhesion (more squish) and forgives a not-quite-" +
    "level bed. 0.2-0.3 mm is a safe default; bump 50 % above your " +
    "layer height for first-layer reliability on tricky surfaces.",

  // ─── Walls / Strength ─────────────────────────────────────────
  wall_loops:
    "Number of perimeter loops printed around the part's outline before " +
    "infill. More walls = stronger part with negligible filament cost. " +
    "2 is the default; bump to 3-4 for parts that need to withstand " +
    "side-loading or screw threads.",

  top_shell_layers:
    "How many solid layers cap the top of the part. Too few = pillowing " +
    "(infill prints visible through the surface). Five layers at 0.2 mm " +
    "= 1 mm total top thickness — adjust higher if your infill is sparse " +
    "(< 20 %) or you see top-surface artifacts.",

  bottom_shell_layers:
    "How many solid layers form the bottom of the part. Too few = the " +
    "first-layer pattern shows through into the second layer. 3 is the " +
    "safe default at 0.2 mm; bump for parts where the bottom is visible.",

  sparse_infill_density:
    "Percentage of the part's interior that gets filled with infill. " +
    "Higher = stronger but more filament + slower. 15-20 % is a strong " +
    "all-purpose default for non-load-bearing parts; bump to 40-60 % " +
    "for functional parts under stress. Walls + top/bottom shells carry " +
    "most of the strength under bending; infill mostly resists buckling.",

  sparse_infill_pattern:
    "Infill geometry. Grid + lines are fastest; gyroid + cubic are " +
    "stronger per gram but slower; honeycomb is between. For functional " +
    "parts use cubic or gyroid at 30-40 %; for cosmetic prints any " +
    "pattern is fine — the difference is mostly weight + print time.",

  // ─── Speed ────────────────────────────────────────────────────
  outer_wall_speed:
    "Speed of the outermost visible wall — the one the eye sees. Lower " +
    "than the inner walls (typically half) because slower = better " +
    "surface finish and corners that don't round off. 30-60 mm/s for " +
    "most printers; modern Klipper rigs can push it higher.",

  inner_wall_speed:
    "Speed of the inner perimeters — these don't show on the surface, " +
    "so push them higher than the outer wall (typically 1.5-2×). " +
    "Bottleneck is usually the hotend's flow rate; if you see " +
    "underextrusion on inner walls, drop this.",

  sparse_infill_speed:
    "Infill speed. Hidden inside the part, so push as fast as the hotend " +
    "+ kinematics allow. 100-150 mm/s on a tuned printer; 60-80 mm/s on " +
    "a stock Ender. Watch for the corners of infill lines fattening — " +
    "that's pressure overshoot and means you're past your flow ceiling.",

  travel_speed:
    "Speed during non-printing moves (jumps between islands, layer " +
    "changes). Higher = less stringing time + faster total print, but " +
    "limited by the printer's stepper acceleration. 200-300 mm/s on a " +
    "Voron-class rig; 120-180 mm/s on a stock bed-slinger.",

  // ─── Support ─────────────────────────────────────────────────
  enable_support:
    "Auto-generates support structures under overhangs steeper than the " +
    "threshold angle. Use when your part has overhangs > ~50° from " +
    "vertical or large bridges. Adds print time + filament; harder to " +
    "remove cleanly than no-support designs, so prefer reorienting the " +
    "part when possible.",

  support_threshold_angle:
    "Maximum overhang angle (from vertical) the printer can bridge " +
    "without support. Above this, supports get generated. Most FDM " +
    "printers handle 45-55° cleanly with good cooling; tune lower (45°) " +
    "for safety or higher (60°) on well-tuned printers to reduce " +
    "support volume.",

  support_top_z_distance:
    "Gap between the top of the support and the supported overhang. " +
    "Larger = easier to remove but worse surface finish underneath. " +
    "0.15-0.2 mm is the typical sweet spot; PETG often needs 0.25 mm " +
    "because it sticks harder.",

  // ─── Adhesion / First layer ───────────────────────────────────
  brim_type:
    "Type of brim drawn around the first layer to improve bed adhesion. " +
    "Outer-only is enough for most parts. Use outer-and-inner when the " +
    "part has internal cavities likely to lift. Skip brim entirely for " +
    "parts where bed adhesion isn't a worry — saves trim time.",

  brim_width:
    "How wide the brim is around the part footprint. 3-5 mm is enough " +
    "for most plates; bump to 8-10 mm for small footprints on PEI where " +
    "adhesion is the limiting factor. Wider brim = more material to " +
    "cut off afterward.",

  raft_layers:
    "Number of raft layers between the part and the bed. Raft trades " +
    "adhesion + level-bed-forgiveness for a longer print + worse " +
    "first-layer surface finish underneath. Skip unless you genuinely " +
    "need it (warpy materials like ABS on unheated beds).",

  // ─── Bed temperature ─────────────────────────────────────────
  bed_temp:
    "Heated-bed temperature for steady-state layers. Drives adhesion + " +
    "warp resistance. PLA: 55-65 °C; PETG: 70-80 °C; ABS/ASA: 100-110 °C. " +
    "Too high = bottom-layer elephant foot + sagging on overhangs near " +
    "the bed; too low = first-layer lift and warping on shrinking " +
    "materials.",

  // ─── Tool / nozzle / filament ─────────────────────────────────
  nozzle_temperature:
    "Hotend temperature in printing layers (post-initial). Higher = " +
    "better layer adhesion + less stringing tolerance; lower = sharper " +
    "details + less oozing. PLA: 200-215 °C; PETG: 230-245 °C; ABS: " +
    "240-250 °C. Run a temp tower per filament for the brand-specific " +
    "sweet spot.",

  nozzle_temperature_initial_layer:
    "Hotend temperature for the first layer only. 5-10 °C above the " +
    "rest helps wet the bed for adhesion. Drop back down once the part " +
    "lifts off, otherwise oozing dominates the wall finish.",

  filament_type:
    "Material chemistry. Drives temp ranges, retract distance, fan " +
    "behavior, bed surface compatibility. Start with PLA; PETG for " +
    "outdoor/heat-resistant; TPU for flexible; ABS only with an enclosed " +
    "chamber. Each material wants its own tuned filament profile.",

  fan_max_speed:
    "Part-cooling fan speed during the body of the layer. Higher = " +
    "better overhang quality + sharper details; lower = better layer " +
    "adhesion. PLA wants 100 %; PETG 30-50 %; ABS often 0 (cooling " +
    "warps it). Bridges always run at 100 % regardless.",

  // ─── Retraction / oozing ─────────────────────────────────────
  retraction_length:
    "How far the extruder pulls filament back during travel moves to " +
    "stop oozing. Direct drive: 0.5-1.5 mm; Bowden: 4-6 mm. Too much = " +
    "grinds the filament + clogs the nozzle; too little = strings " +
    "between parts. Tune via a stringing test print.",

  retraction_speed:
    "Speed of the retract motion. Faster = less ooze during the " +
    "retract; too fast = grinds the filament. 30-50 mm/s on direct " +
    "drive; 25-40 mm/s on Bowden because the longer filament path " +
    "absorbs less speed.",

  // ─── Surface finish / quality ─────────────────────────────────
  ironing:
    "After the top layer prints, run the nozzle (at slow speed, no " +
    "extrusion) over the surface to melt-smooth it. Looks great on " +
    "large flat tops; adds noticeable print time. Tune the flow at " +
    "8-12 % of the line width; higher = bumpy ironing.",

  // ─── Print sequence / multi-color ────────────────────────────
  print_sequence:
    "Whether to print multiple parts layer-by-layer (default) or " +
    "complete each part before starting the next. By-object skips " +
    "tool changes between parts but risks nozzle collisions on tall " +
    "prints — verify clearance for your hotend geometry first.",

  // ─── Wall / outer geometry ───────────────────────────────────
  line_width:
    "Width of each extruded line. Convention is to match the nozzle " +
    "diameter (0.4 mm by default); going wider increases flow + strength " +
    "but loses fine detail. 100-120 % of nozzle diameter is a safe band. " +
    "Some advanced flow tuning sets line_width > nozzle to push more " +
    "filament per pass; only do it if you've tuned max flow rate first.",

  wall_filament:
    "Which filament slot prints the part's walls. Default is filament 1; " +
    "override per-region to make text or surface details print in a " +
    "different color. Multi-material support is the headline use case — " +
    "pair with `solid_infill_filament` if you want the color to follow " +
    "through to the top/bottom shells.",

  skirt_loops:
    "Loops of an outline drawn around (but not touching) the first layer. " +
    "Used to prime the nozzle and confirm the bed is level before the " +
    "real part starts. 1-2 loops is enough for priming; 5-6 if you want " +
    "a visual indicator that the first 30 seconds of the print are " +
    "tracking the right path.",

  // ─── Wipe tower / purge ──────────────────────────────────────
  enable_prime_tower:
    "On multi-material printers, prints a small tower the slicer " +
    "purges into during filament changes. Mandatory on AMS-style " +
    "printers; skip on toolchangers (which swap heads instead of " +
    "purging). Tower volume scales with the number of changes — " +
    "expect ~1-3 g per change.",
};
