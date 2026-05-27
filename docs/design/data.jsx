// data.jsx — cascade layers + categories + settings model.
// Each setting carries a `cascade` object with values at zero or more layers.
// The "active" value is whichever layer is highest-priority and defined.

const CASCADE_LAYERS = [
  { id: "printer",  label: "Printer",   short: "PRN", hue: 18,  desc: "Mechanical limits, kinematics, dimensions" },
  { id: "toolhead", label: "Toolhead",  short: "TOO", hue: 55,  desc: "Extruder hardware, hotend block" },
  { id: "nozzle",   label: "Nozzle",    short: "NOZ", hue: 95,  desc: "Diameter, material, flow geometry" },
  { id: "filament", label: "Filament",  short: "FIL", hue: 175, desc: "Material chemistry, temps, retract" },
  { id: "user",     label: "Profile",   short: "USR", hue: 235, desc: "Personal defaults across projects" },
  { id: "project",  label: "Project",   short: "PRJ", hue: 285, desc: "This .3mf file" },
  { id: "object",   label: "Object",    short: "OBJ", hue: 340, desc: "Per-mesh overrides" },
];

const LAYER_BY_ID = Object.fromEntries(CASCADE_LAYERS.map(l => [l.id, l]));
const LAYER_INDEX = Object.fromEntries(CASCADE_LAYERS.map((l, i) => [l.id, i]));

// Resolve the active value of a setting from its cascade.
function resolveValue(setting) {
  for (let i = CASCADE_LAYERS.length - 1; i >= 0; i--) {
    const id = CASCADE_LAYERS[i].id;
    if (setting.cascade[id] !== undefined && setting.cascade[id] !== null) {
      return { value: setting.cascade[id], layer: id };
    }
  }
  return { value: setting.default, layer: "printer" };
}

// Detect a conflict: object overrides project (or any higher layer overrides lower).
// Returns the list of layers that are being overridden.
function getOverriddenLayers(setting) {
  const defined = CASCADE_LAYERS
    .filter(l => setting.cascade[l.id] !== undefined && setting.cascade[l.id] !== null)
    .map(l => l.id);
  if (defined.length <= 1) return [];
  return defined.slice(0, -1); // all but the winning layer
}

// ───────────── Settings catalog ─────────────
// Helper: build a setting where cascade defaults to printer-level only.
const s = (id, name, type, unit, cascade, opts = {}) => ({
  id, name, type, unit, cascade, ...opts,
});

const CATEGORIES = [
  {
    id: "quality", name: "Quality", icon: "Q",
    desc: "Layer geometry that controls visible surface fidelity",
    settings: [
      s("layer_height",       "Layer height",            "number", "mm", { printer: 0.2, filament: 0.16, project: 0.12 }, { min: 0.04, max: 0.6, step: 0.01 }),
      s("initial_layer_h",    "Initial layer height",    "number", "mm", { printer: 0.3, project: 0.24 }, { min: 0.1, max: 0.5, step: 0.02 }),
      s("line_width",         "Line width",              "number", "mm", { nozzle: 0.4 }, { min: 0.1, max: 1.2, step: 0.01 }),
      s("wall_line_width",    "Wall line width",         "number", "mm", { nozzle: 0.4, project: 0.42 }, { min: 0.1, max: 1.2, step: 0.01 }),
      s("infill_line_width",  "Infill line width",       "number", "mm", { nozzle: 0.4 }, { min: 0.1, max: 1.2, step: 0.01 }),
      s("init_line_width",    "Initial layer line width","number", "%",  { printer: 100 }, { min: 50, max: 200, step: 5 }),
      s("seam_position",      "Seam position",           "select", "",   { filament: "aligned" }, { options: ["aligned", "nearest", "random", "sharpest"] }),
      s("seam_visibility",    "Hide seam in overhangs",  "toggle", "",   { project: true }),
      s("z_seam_x",           "Z seam X offset",         "number", "mm", { printer: 0 }, { min: -200, max: 200, step: 1 }),
      s("z_seam_y",           "Z seam Y offset",         "number", "mm", { printer: 0 }, { min: -200, max: 200, step: 1 }),
      s("xy_offset",          "Horizontal expansion",    "number", "mm", { printer: 0 }, { min: -1, max: 1, step: 0.01 }),
      s("xy_offset_hole",     "Hole horizontal expansion","number","mm", { printer: 0 }, { min: -1, max: 1, step: 0.01 }),
      s("z_offset",           "Z offset",                "number", "mm", { printer: 0, user: -0.04 }, { min: -1, max: 1, step: 0.01 }),
      s("slicing_tolerance",  "Slicing tolerance",       "select", "",   { printer: "middle" }, { options: ["middle", "exclusive", "inclusive"] }),
      s("min_feat_size",      "Minimum feature size",    "number", "mm", { nozzle: 0.1 }, { min: 0, max: 1, step: 0.01 }),
      s("min_thick_wall",     "Minimum thin-wall size",  "number", "mm", { nozzle: 0.34 }, { min: 0, max: 2, step: 0.01 }),
      s("res_avoid_cross",    "Resolution",              "number", "mm", { printer: 0.05 }, { min: 0.001, max: 1, step: 0.005 }),
    ],
  },
  {
    id: "walls", name: "Walls", icon: "W",
    desc: "Perimeter count, alternation, ironing",
    settings: [
      s("wall_count",         "Wall line count",         "number", "",   { filament: 3, project: 4 }, { min: 0, max: 16, step: 1 }),
      s("wall_thickness",     "Wall thickness",          "number", "mm", { filament: 1.2 }, { min: 0.1, max: 8, step: 0.1 }),
      s("outer_before_inner", "Outer before inner walls","toggle", "",   { filament: false, project: true }),
      s("alt_extra_wall",     "Alternate extra wall",    "toggle", "",   { filament: false }),
      s("min_wall_flow",      "Minimum wall flow",       "number", "%",  { printer: 25 }, { min: 0, max: 100, step: 5 }),
      s("fill_gaps",          "Fill gaps between walls", "select", "",   { filament: "everywhere" }, { options: ["nowhere", "everywhere", "infill only"] }),
      s("filter_out_tiny",    "Filter out tiny gaps",    "toggle", "",   { printer: true }),
      s("compensate_wall",    "Compensate wall overlaps","toggle", "",   { filament: true }),
      s("print_thin_walls",   "Print thin walls",        "toggle", "",   { project: true }),
      s("overhang_speed",     "Overhang wall speed",     "number","mm/s",{ filament: 30, project: 22 }, { min: 1, max: 200, step: 1 }),
      s("overhang_thresh",    "Overhang angle",          "number", "°",  { filament: 45 }, { min: 0, max: 90, step: 1 }),
      s("wall_print_order",   "Wall printing order",     "select", "",   { project: "inside-out" }, { options: ["inside-out", "outside-in"] }),
      s("ext_wall_acc",       "Outer wall acceleration", "number","mm/s²",{ toolhead: 1500, filament: 800 }, { min: 100, max: 10000, step: 50 }),
      s("ext_wall_jerk",      "Outer wall jerk",         "number","mm/s",{ toolhead: 8, filament: 5 }, { min: 0, max: 50, step: 0.5 }),
      s("ironing_enable",     "Enable ironing",          "toggle", "",   { project: false }),
      s("ironing_flow",       "Ironing flow",            "number", "%",  { filament: 10 }, { min: 0, max: 100, step: 1 }),
      s("ironing_inset",      "Ironing inset",           "number", "mm", { nozzle: 0.38 }, { min: 0, max: 2, step: 0.01 }),
    ],
  },
  {
    id: "topbottom", name: "Top / Bottom", icon: "T",
    desc: "Solid layer geometry on horizontal surfaces",
    settings: [
      s("top_layers",         "Top layers",              "number", "",   { filament: 4, project: 5 }, { min: 0, max: 30, step: 1 }),
      s("bottom_layers",      "Bottom layers",           "number", "",   { filament: 3 }, { min: 0, max: 30, step: 1 }),
      s("top_thickness",      "Top thickness",           "number", "mm", { filament: 0.8 }, { min: 0, max: 5, step: 0.1 }),
      s("bottom_thickness",   "Bottom thickness",        "number", "mm", { filament: 0.6 }, { min: 0, max: 5, step: 0.1 }),
      s("top_pattern",        "Top pattern",             "select", "",   { filament: "lines", project: "concentric" }, { options: ["lines", "concentric", "zig-zag"] }),
      s("bottom_pattern",     "Bottom pattern",          "select", "",   { filament: "lines" }, { options: ["lines", "concentric", "zig-zag"] }),
      s("monotonic_top",      "Monotonic top order",     "toggle", "",   { project: true }),
      s("skin_overlap",       "Skin overlap",            "number", "%",  { filament: 10 }, { min: 0, max: 100, step: 1 }),
      s("skin_removal",       "Skin removal width",      "number", "mm", { nozzle: 0.8 }, { min: 0, max: 5, step: 0.1 }),
      s("expand_skins",       "Expand top/bottom skins", "toggle", "",   { printer: false }),
      s("top_init_line",      "Initial top line width",  "number", "%",  { printer: 100 }, { min: 50, max: 200, step: 5 }),
      s("ironing_only_top",   "Iron only the topmost",   "toggle", "",   { project: true }),
      s("extra_skin_walls",   "Extra skin wall count",   "number", "",   { printer: 0 }, { min: 0, max: 4, step: 1 }),
      s("skin_edge_support",  "Skin edge support",       "toggle", "",   { printer: false }),
      s("interface_skin",     "Interface skin thickness","number", "mm", { printer: 0.4 }, { min: 0, max: 2, step: 0.1 }),
      s("top_dir_rot",        "Skin line directions",    "select", "",   { project: "45° / -45°" }, { options: ["0° / 90°", "45° / -45°", "rotating"] }),
    ],
  },
  {
    id: "infill", name: "Infill", icon: "I",
    desc: "Interior fill — pattern, density, structure",
    settings: [
      s("infill_density",     "Infill density",          "number", "%",  { filament: 15, project: 22 }, { min: 0, max: 100, step: 1 }),
      s("infill_pattern",     "Infill pattern",          "select", "",   { filament: "gyroid", project: "honeycomb" }, { options: ["grid", "lines", "triangles", "cubic", "gyroid", "honeycomb", "lightning"] }),
      s("infill_line_dist",   "Infill line distance",    "number", "mm", { printer: 4.0 }, { min: 0.1, max: 50, step: 0.1 }),
      s("infill_speed",       "Infill speed",            "number","mm/s",{ filament: 80, project: 120 }, { min: 1, max: 500, step: 1 }),
      s("infill_acc",         "Infill acceleration",     "number","mm/s²",{ toolhead: 2500 }, { min: 100, max: 20000, step: 50 }),
      s("infill_overlap",     "Infill overlap %",        "number", "%",  { filament: 15 }, { min: 0, max: 100, step: 1 }),
      s("infill_wipe_dist",   "Infill wipe distance",    "number", "mm", { nozzle: 0.1 }, { min: 0, max: 2, step: 0.01 }),
      s("infill_z_step",      "Infill Z step",           "number", "mm", { printer: 0 }, { min: 0, max: 2, step: 0.05 }),
      s("min_infill_area",    "Minimum infill area",     "number", "mm²",{ printer: 0 }, { min: 0, max: 100, step: 1 }),
      s("connect_infill",     "Connect infill lines",    "toggle", "",   { filament: true }),
      s("connect_polys",      "Connect infill polygons", "toggle", "",   { filament: true }),
      s("skin_to_infill",     "Skin removal at infill",  "toggle", "",   { printer: false }),
      s("infill_before_walls","Infill before walls",     "toggle", "",   { project: false }),
      s("gradual_infill",     "Gradual infill steps",    "number", "",   { printer: 0 }, { min: 0, max: 8, step: 1 }),
      s("gradual_step_h",     "Gradual infill step height","number","mm",{ printer: 1.5 }, { min: 0.1, max: 10, step: 0.1 }),
      s("infill_support",     "Infill support",          "toggle", "",   { project: false }),
      s("lightning_density",  "Lightning support density","number","%",  { printer: 60 }, { min: 0, max: 100, step: 1 }),
    ],
  },
  {
    id: "material", name: "Material", icon: "M",
    desc: "Temperature, flow, retraction governed by filament chemistry",
    settings: [
      s("print_temp",         "Printing temperature",    "number", "°C", { filament: 215, project: 220 }, { min: 150, max: 320, step: 1 }),
      s("init_print_temp",    "Initial layer temp",      "number", "°C", { filament: 220 }, { min: 150, max: 320, step: 1 }),
      s("bed_temp",           "Build plate temperature", "number", "°C", { filament: 60, project: 65 }, { min: 0, max: 130, step: 1 }),
      s("init_bed_temp",      "Initial bed temp",        "number", "°C", { filament: 65 }, { min: 0, max: 130, step: 1 }),
      s("chamber_temp",       "Chamber temperature",     "number", "°C", { printer: 0 }, { min: 0, max: 70, step: 1 }),
      s("flow",               "Flow",                    "number", "%",  { filament: 100, user: 98 }, { min: 50, max: 200, step: 1 }),
      s("init_flow",          "Initial layer flow",      "number", "%",  { filament: 100, project: 105 }, { min: 50, max: 200, step: 1 }),
      s("retract_enable",     "Enable retraction",       "toggle", "",   { filament: true }),
      s("retract_dist",       "Retraction distance",     "number", "mm", { toolhead: 4, filament: 0.8 }, { min: 0, max: 20, step: 0.05 }),
      s("retract_speed",      "Retraction speed",        "number","mm/s",{ toolhead: 35, filament: 40 }, { min: 1, max: 200, step: 1 }),
      s("retract_min_travel", "Retract min travel",      "number", "mm", { filament: 1.5 }, { min: 0, max: 10, step: 0.1 }),
      s("retract_count_max",  "Retract count max",       "number", "",   { filament: 90 }, { min: 0, max: 200, step: 1 }),
      s("density",            "Material density",        "number","g/cm³",{ filament: 1.24 }, { min: 0.5, max: 3, step: 0.01 }),
      s("diameter",           "Filament diameter",       "number", "mm", { filament: 1.75 }, { min: 1, max: 3, step: 0.01 }),
      s("cost_per_kg",        "Cost per kg",             "number", "$",  { user: 24 }, { min: 0, max: 1000, step: 1 }),
      s("standby_temp",       "Standby temperature",     "number", "°C", { filament: 175 }, { min: 0, max: 320, step: 1 }),
      s("purge_volume",       "Purge volume on toolchange","number","mm³",{ filament: 70 }, { min: 0, max: 500, step: 1 }),
    ],
  },
  {
    id: "speed", name: "Speed", icon: "S",
    desc: "Per-feature feedrates and global travel limits",
    settings: [
      s("print_speed",        "Print speed",             "number","mm/s",{ filament: 60, project: 80, user: 70 }, { min: 1, max: 500, step: 1 }),
      s("wall_speed",         "Wall speed",              "number","mm/s",{ filament: 50, project: 60 }, { min: 1, max: 500, step: 1 }),
      s("outer_wall_speed",   "Outer wall speed",        "number","mm/s",{ filament: 30, project: 35 }, { min: 1, max: 500, step: 1 }),
      s("inner_wall_speed",   "Inner wall speed",        "number","mm/s",{ filament: 60 }, { min: 1, max: 500, step: 1 }),
      s("top_bottom_speed",   "Top/bottom speed",        "number","mm/s",{ filament: 40 }, { min: 1, max: 500, step: 1 }),
      s("support_speed",      "Support speed",           "number","mm/s",{ filament: 80 }, { min: 1, max: 500, step: 1 }),
      s("travel_speed",       "Travel speed",            "number","mm/s",{ printer: 250, project: 300 }, { min: 1, max: 800, step: 5 }),
      s("init_layer_speed",   "Initial layer speed",     "number","mm/s",{ filament: 20 }, { min: 1, max: 200, step: 1 }),
      s("skirt_speed",        "Skirt/brim speed",        "number","mm/s",{ printer: 20 }, { min: 1, max: 200, step: 1 }),
      s("z_hop_speed",        "Z hop speed",             "number","mm/s",{ printer: 10 }, { min: 1, max: 100, step: 1 }),
      s("max_accel",          "Max acceleration",        "number","mm/s²",{ printer: 3000, toolhead: 4000 }, { min: 100, max: 20000, step: 100 }),
      s("max_jerk",           "Max jerk",                "number","mm/s",{ printer: 10 }, { min: 0, max: 50, step: 0.5 }),
      s("classic_jerk",       "Use classic jerk",        "toggle", "",   { printer: false }),
      s("first_layer_acc",    "First layer acceleration","number","mm/s²",{ printer: 500 }, { min: 100, max: 10000, step: 50 }),
      s("ext_speed_mult",     "External wall speed mult","number", "%",  { filament: 50 }, { min: 10, max: 100, step: 5 }),
      s("min_layer_time",     "Minimum layer time",      "number", "s",  { filament: 5 }, { min: 0, max: 60, step: 1 }),
    ],
  },
  {
    id: "travel", name: "Travel", icon: "→",
    desc: "Non-printing moves, z-hop, avoidance",
    settings: [
      s("avoid_crossing",     "Avoid crossing perimeters","toggle","",   { project: true }),
      s("avoid_crossing_supports","Avoid crossing supports","toggle","", { printer: false }),
      s("z_hop_enable",       "Enable Z hop",            "toggle", "",   { filament: true }),
      s("z_hop_height",       "Z hop height",            "number", "mm", { printer: 0.4, filament: 0.2 }, { min: 0, max: 5, step: 0.05 }),
      s("z_hop_only_print",   "Hop only over printed",   "toggle", "",   { project: true }),
      s("z_hop_type",         "Z hop type",              "select", "",   { printer: "normal" }, { options: ["normal", "slope", "spiral"] }),
      s("travel_avoid_dist",  "Avoidance distance",      "number", "mm", { printer: 0.625 }, { min: 0, max: 10, step: 0.05 }),
      s("retract_at_layer",   "Retract at layer change", "toggle", "",   { filament: true }),
      s("wipe_on_retract",    "Wipe on retract",         "toggle", "",   { filament: false }),
      s("wipe_dist",          "Wipe distance",           "number", "mm", { filament: 0.8 }, { min: 0, max: 10, step: 0.1 }),
      s("coast_at_end",       "Coast at end",            "toggle", "",   { printer: false }),
      s("coast_volume",       "Coasting volume",         "number","mm³", { printer: 0.064 }, { min: 0, max: 5, step: 0.005 }),
      s("retract_extra_at_layer","Retract extra at layer change","number","mm",{ printer: 0 }, { min: 0, max: 5, step: 0.05 }),
      s("zhop_during_travel", "Hop during travel only",  "toggle", "",   { printer: true }),
      s("ramming_volume",     "Ramming volume",          "number","mm³", { filament: 8 }, { min: 0, max: 50, step: 0.5 }),
    ],
  },
  {
    id: "cooling", name: "Cooling", icon: "❄",
    desc: "Part fan, bridge fan, layer-time minima",
    settings: [
      s("fan_enable",         "Enable part cooling",     "toggle", "",   { filament: true }),
      s("fan_speed",          "Fan speed",               "number", "%",  { filament: 100, project: 80 }, { min: 0, max: 100, step: 1 }),
      s("init_fan_speed",     "Initial fan speed",       "number", "%",  { filament: 0 }, { min: 0, max: 100, step: 1 }),
      s("fan_full_at_layer",  "Regular fan at layer",    "number", "",   { filament: 3 }, { min: 1, max: 20, step: 1 }),
      s("min_speed_cool",     "Min print speed (cooling)","number","mm/s",{ filament: 10 }, { min: 1, max: 100, step: 1 }),
      s("min_layer_time_cool","Minimum layer time",      "number", "s",  { filament: 5 }, { min: 0, max: 60, step: 1 }),
      s("fan_lift_at_min_t",  "Fan lift speed",          "number", "%",  { filament: 100 }, { min: 0, max: 100, step: 1 }),
      s("bridge_fan_speed",   "Bridge fan speed",        "number", "%",  { filament: 100 }, { min: 0, max: 100, step: 1 }),
      s("overhang_fan_speed", "Overhang fan speed",      "number", "%",  { filament: 100, project: 100 }, { min: 0, max: 100, step: 1 }),
      s("disable_fan_first",  "Disable fan first N layers","number","",  { filament: 1 }, { min: 0, max: 10, step: 1 }),
      s("aux_fan_speed",      "Aux fan speed",           "number", "%",  { printer: 0 }, { min: 0, max: 100, step: 1 }),
      s("chamber_fan",        "Chamber fan speed",       "number", "%",  { printer: 0 }, { min: 0, max: 100, step: 1 }),
    ],
  },
  {
    id: "support", name: "Support", icon: "⌐",
    desc: "Where, how, and when to support overhangs",
    settings: [
      s("support_enable",     "Generate support",        "toggle", "",   { project: false }),
      s("support_type",       "Support type",            "select", "",   { project: "tree" }, { options: ["normal", "tree", "snug", "organic"] }),
      s("support_angle",      "Overhang threshold",      "number", "°",  { filament: 45 }, { min: 0, max: 90, step: 1 }),
      s("support_density",    "Support density",         "number", "%",  { project: 15 }, { min: 0, max: 100, step: 1 }),
      s("support_xy_dist",    "Support XY distance",     "number", "mm", { nozzle: 0.8 }, { min: 0, max: 5, step: 0.05 }),
      s("support_z_dist",     "Support Z distance",      "number", "mm", { filament: 0.16 }, { min: 0, max: 2, step: 0.02 }),
      s("support_interface",  "Enable interface",        "toggle", "",   { project: true }),
      s("interface_density",  "Interface density",       "number", "%",  { project: 90 }, { min: 0, max: 100, step: 1 }),
      s("interface_layers",   "Interface layers",        "number", "",   { project: 2 }, { min: 0, max: 10, step: 1 }),
      s("tree_branch_dia",    "Tree branch diameter",    "number", "mm", { project: 2.0 }, { min: 0.5, max: 10, step: 0.1 }),
      s("tree_angle",         "Tree branch angle",       "number", "°",  { project: 40 }, { min: 0, max: 80, step: 1 }),
      s("support_on_build",   "Support only on buildplate","toggle","",  { project: false }),
      s("support_pattern",    "Support pattern",         "select", "",   { project: "zigzag" }, { options: ["lines", "grid", "triangles", "concentric", "zigzag", "cross"] }),
      s("support_brim",       "Support brim",            "toggle", "",   { printer: false }),
      s("support_floor",      "Support floor layers",    "number", "",   { project: 0 }, { min: 0, max: 10, step: 1 }),
    ],
  },
  {
    id: "adhesion", name: "Adhesion", icon: "⌂",
    desc: "First-layer adhesion strategy",
    settings: [
      s("adhesion_type",      "Adhesion type",           "select", "",   { project: "brim" }, { options: ["none", "skirt", "brim", "raft"] }),
      s("skirt_lines",        "Skirt line count",        "number", "",   { project: 2 }, { min: 0, max: 10, step: 1 }),
      s("skirt_dist",         "Skirt distance",          "number", "mm", { project: 3 }, { min: 0, max: 20, step: 0.5 }),
      s("skirt_min_length",   "Skirt minimum length",    "number", "mm", { project: 250 }, { min: 0, max: 2000, step: 10 }),
      s("brim_width",         "Brim width",              "number", "mm", { project: 8 }, { min: 0, max: 50, step: 0.5 }),
      s("brim_line_count",    "Brim line count",         "number", "",   { project: 20 }, { min: 0, max: 100, step: 1 }),
      s("brim_only_outside",  "Brim only on outside",    "toggle", "",   { project: true }),
      s("raft_margin",        "Raft extra margin",       "number", "mm", { project: 5 }, { min: 0, max: 50, step: 0.5 }),
      s("raft_smoothing",     "Raft smoothing",          "number", "mm", { project: 5 }, { min: 0, max: 20, step: 0.5 }),
      s("raft_layers",        "Raft layers",             "number", "",   { project: 2 }, { min: 1, max: 6, step: 1 }),
      s("prime_tower",        "Enable prime tower",      "toggle", "",   { project: false }),
      s("prime_tower_size",   "Prime tower size",        "number", "mm", { project: 20 }, { min: 5, max: 100, step: 1 }),
      s("draft_shield",       "Draft shield",            "toggle", "",   { printer: false }),
      s("draft_shield_h",     "Draft shield height",     "number", "mm", { printer: 10 }, { min: 1, max: 200, step: 1 }),
    ],
  },
  {
    id: "multiext", name: "Multi-Extruder", icon: "⫶",
    desc: "Toolchange routing for multi-material prints",
    settings: [
      s("prime_volume",       "Prime volume on swap",    "number","mm³", { toolhead: 30, filament: 45 }, { min: 0, max: 500, step: 1 }),
      s("toolchange_temp",    "Toolchange temperature",  "number", "°C", { filament: 200 }, { min: 0, max: 320, step: 1 }),
      s("toolchange_retract", "Toolchange retract",      "number", "mm", { toolhead: 6 }, { min: 0, max: 20, step: 0.1 }),
      s("toolchange_zhop",    "Toolchange Z hop",        "number", "mm", { toolhead: 1 }, { min: 0, max: 5, step: 0.1 }),
      s("wipe_tower_enable",  "Enable wipe tower",       "toggle", "",   { project: true }),
      s("wipe_tower_x",       "Wipe tower X",            "number", "mm", { project: 170 }, { min: 0, max: 400, step: 1 }),
      s("wipe_tower_y",       "Wipe tower Y",            "number", "mm", { project: 140 }, { min: 0, max: 400, step: 1 }),
      s("wipe_tower_w",       "Wipe tower width",        "number", "mm", { project: 60 }, { min: 10, max: 200, step: 1 }),
      s("wipe_tower_rot",     "Wipe tower rotation",     "number", "°",  { project: 0 }, { min: 0, max: 180, step: 5 }),
      s("ooze_shield",        "Ooze shield",             "toggle", "",   { printer: false }),
      s("interface_only_first","Interface for first only","toggle", "",  { printer: false }),
      s("ext_print_seq",      "Print sequence",          "select", "",   { project: "all at once" }, { options: ["all at once", "one at a time"] }),
    ],
  },
  {
    id: "meshfix", name: "Mesh Fixes", icon: "△",
    desc: "Topology repair before slicing",
    settings: [
      s("union_overlapping",  "Union overlapping volumes","toggle","",   { printer: true }),
      s("remove_all_holes",   "Remove all holes",        "toggle", "",   { printer: false }),
      s("extensive_stitch",   "Extensive stitching",     "toggle", "",   { printer: false }),
      s("keep_disconnect",    "Keep disconnected faces", "toggle", "",   { printer: false }),
      s("merge_overlaps",     "Merge overlapping volumes","toggle","",   { printer: true }),
      s("simplify_mesh",      "Maximum resolution",      "number", "mm", { printer: 0.04 }, { min: 0.001, max: 1, step: 0.005 }),
      s("max_deviation",      "Maximum deviation",       "number", "mm", { printer: 0.025 }, { min: 0.001, max: 0.5, step: 0.005 }),
      s("walking_min_arc",    "Walking min arc segment", "number", "°",  { printer: 1.0 }, { min: 0.1, max: 30, step: 0.1 }),
      s("alt_carve_order",    "Alternate carve order",   "toggle", "",   { printer: false }),
      s("remove_empty_first", "Remove empty first layers","toggle","",   { project: true }),
    ],
  },
  {
    id: "special", name: "Special Modes", icon: "✦",
    desc: "Vase mode, sequential printing, surface modes",
    settings: [
      s("print_sequence",     "Print sequence",          "select", "",   { project: "all-at-once" }, { options: ["all-at-once", "one-at-a-time"] }),
      s("surface_mode",       "Surface mode",            "select", "",   { project: "normal" }, { options: ["normal", "surface", "both"] }),
      s("spiralize_outer",    "Spiralize outer contour", "toggle", "",   { project: false }),
      s("smooth_spiralized",  "Smooth spiralized",       "toggle", "",   { project: true }),
      s("relative_extrusion", "Relative extrusion",      "toggle", "",   { printer: false }),
      s("fuzzy_skin",         "Fuzzy skin",              "toggle", "",   { project: false }),
      s("fuzzy_thickness",    "Fuzzy skin thickness",    "number", "mm", { printer: 0.3 }, { min: 0.05, max: 1, step: 0.05 }),
      s("fuzzy_density",      "Fuzzy point density",     "number","1/mm",{ printer: 1.25 }, { min: 0.1, max: 5, step: 0.05 }),
    ],
  },
  {
    id: "experimental", name: "Experimental", icon: "β",
    desc: "Caveat emptor — these may break your print",
    settings: [
      s("adaptive_layers",    "Adaptive layers",         "toggle", "",   { printer: false }),
      s("adaptive_topo",      "Adaptive topography",     "number", "mm", { printer: 0.2 }, { min: 0, max: 1, step: 0.01 }),
      s("input_shaping",      "Input shaping (klipper)", "toggle", "",   { printer: true }),
      s("input_shaper_x",     "Input shaper X freq",     "number", "Hz", { printer: 38.4 }, { min: 0, max: 200, step: 0.1 }),
      s("input_shaper_y",     "Input shaper Y freq",     "number", "Hz", { printer: 42.1 }, { min: 0, max: 200, step: 0.1 }),
      s("pressure_advance",   "Pressure advance",        "number", "s",  { filament: 0.04 }, { min: 0, max: 2, step: 0.001 }),
      s("smooth_time",        "Pressure advance smooth", "number", "s",  { printer: 0.04 }, { min: 0, max: 1, step: 0.001 }),
      s("flow_compensation",  "Flow rate compensation",  "toggle", "",   { printer: false }),
      s("arc_fitting",        "Arc fitting",             "toggle", "",   { printer: false }),
    ],
  },
];

// flatten for global search
const ALL_SETTINGS = CATEGORIES.flatMap(c =>
  c.settings.map(s => ({ ...s, _catId: c.id, _catName: c.name }))
);

// stat: number of settings overridden at object level (used as a quick badge)
function countOverridesAtLayer(layerId) {
  return ALL_SETTINGS.filter(s =>
    s.cascade[layerId] !== undefined && s.cascade[layerId] !== null
  ).length;
}

// ───────────── Slot / material model ─────────────
// Filaments live in physical *slots*. Slots come from the printer's hardware
// configuration:
//   - one or more direct extruders ("ext", "ext:2", …)
//   - zero or more AMS units, each with 4 slots ("AMS-A:1"…"AMS-A:4",
//     "AMS-B:1"…"AMS-B:4", …; letter = unit identity).
//
// Objects on the plate carry a *material id* (M1, M2, …) which the project
// maps to a slot. Resolution goes: object.materialId → slot → filament.

// Compute the ordered slot id list for a given hardware config.
//
// Ordering rule: when an AMS is attached (Bambu Lab style), the AMS slots are
// the primary loadout the user picks from and the direct ext slot is the
// overflow / fallback — so we list AMS first, ext at the end. Without an AMS,
// ext slot(s) lead.
function computeSlotIds({ extruders = 1, amsUnits = 0 } = {}) {
  const extIds = [];
  if (extruders <= 1) {
    extIds.push("ext");
  } else {
    for (let i = 1; i <= extruders; i++) extIds.push(`ext:${i}`);
  }
  const amsIds = [];
  for (let u = 0; u < amsUnits; u++) {
    const letter = String.fromCharCode(65 + u); // A, B, C, D
    for (let s = 1; s <= 4; s++) amsIds.push(`AMS-${letter}:${s}`);
  }
  return amsIds.length > 0 ? [...amsIds, ...extIds] : [...extIds, ...amsIds];
}

// Compact label for a slot — what shows on the slot pill.
// Hides the AMS-unit letter when there's only one AMS unit (no ambiguity).
// Multi-extruder printers label their direct slots by toolhead (T1, T2…) —
// in this build one toolhead = one extruder; richer topologies (a toolhead
// with multiple extruders) are out of scope for now.
function slotShortLabel(slotId, slotIds = []) {
  if (slotId === "ext") return "ext";
  const extMatch = slotId.match(/^ext:(\d+)$/);
  if (extMatch) return `T${extMatch[1]}`;
  const amsMatch = slotId.match(/^AMS-([A-Z]):(\d+)$/);
  if (amsMatch) {
    const [, letter, idx] = amsMatch;
    // If multiple AMS units present, show "A:1", "B:3", etc.
    // If only one, just show the slot number.
    const distinctUnits = new Set();
    slotIds.forEach(id => {
      const m = id.match(/^AMS-([A-Z]):/);
      if (m) distinctUnits.add(m[1]);
    });
    return distinctUnits.size > 1 ? `${letter}:${idx}` : idx;
  }
  return slotId;
}

// Verbose label for dropdowns / tooltips.
function slotLongLabel(slotId) {
  if (slotId === "ext") return "External spool";
  const extMatch = slotId.match(/^ext:(\d+)$/);
  if (extMatch) return `Toolhead ${extMatch[1]}`;
  const amsMatch = slotId.match(/^AMS-([A-Z]):(\d+)$/);
  if (amsMatch) return `AMS ${amsMatch[1]} · slot ${amsMatch[2]}`;
  return slotId;
}

// Resolve an object's filament through the two-step indirection.
// Returns { material, slot, filament } — any leg may be null/undefined if
// the mapping is incomplete; callers should fall back gracefully.
function resolveObjectFilament(obj, materialMap, slotMap, filaments) {
  const materialId = obj && obj.materialId;
  const slotId = materialId ? materialMap[materialId] : null;
  const filamentId = slotId ? slotMap[slotId] : null;
  const filament = filamentId ? filaments.find(f => f.id === filamentId) : null;
  return { materialId, slotId, filamentId, filament };
}

window.SLICER_DATA = {
  CASCADE_LAYERS, LAYER_BY_ID, LAYER_INDEX,
  CATEGORIES, ALL_SETTINGS,
  resolveValue, getOverriddenLayers, countOverridesAtLayer,
  computeSlotIds, slotShortLabel, slotLongLabel,
  resolveObjectFilament,
};
