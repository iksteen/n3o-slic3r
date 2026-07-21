// Canned responses for the demo's mock `invoke`, keyed by Tauri command name.
// Anything not listed resolves to `undefined` (logged once). The ids are kept
// consistent across snapshot + instance (the app correlates plate ->
// printer_instance_id -> instance.id, and active_plate_id -> plate.plate_id).

import { emit } from "./tauri-event";
import processOptions from "../assets/settings-process.json";
import machineOptions from "../assets/settings-machine.json";
import extruderOptions from "../assets/settings-extruder.json";
import filamentOptions from "../assets/settings-filament.json";
import resolvedEntries from "../assets/settings-resolved.json";

type Handler = (args: Record<string, unknown>) => unknown | Promise<unknown>;

// Real n3o option summaries (dumped via `dump_settings`). The panel passes a
// search `filter`; match it client-side against key/label so the search box
// works in the demo.
type Opt = { key: string; label: string | null };
function filtered(list: Opt[], args: Record<string, unknown>): Opt[] {
  const f = typeof args.filter === "string" ? args.filter.trim().toLowerCase() : "";
  if (!f) return list;
  return list.filter(
    (o) => o.key.toLowerCase().includes(f) || (o.label ?? "").toLowerCase().includes(f),
  );
}

const BBOX = { min: [142.11, 10.21, 0.2], max: [169.81, 71.01, 16.6] };

// Canned preview for the sample slice (values from the real n3o slice of the
// M5 case). The demo GcodePreview draws the toolpaths itself; this feeds the
// surrounding chrome (layer count, timeline, stats summary).
const PREVIEW = {
  handle: 1,
  header: {
    slicer: "Orca",
    slicer_version: "n3o demo",
    estimated_time: "1h 12m",
    filament_used: [{ unit: "g", value: "8.4" }, { unit: "m", value: "2.81" }],
    layer_count: 83,
    object_count: 1,
    printer_model: "Bambu Lab A1 mini",
    bbox_min: BBOX.min,
    bbox_max: BBOX.max,
    raw_settings: {},
  },
  layer_count: 83,
  extrusion_count: 2746,
  travel_count: 900,
  retraction_count: 120,
  bounding_box: BBOX,
  job_stats: {
    total_duration_seconds: 4320,
    layer_count: 83,
    feature_breakdown: {
      "Outer wall": 1180, "Inner wall": 900, "Sparse infill": 620,
      "Top surface": 210, "Bottom surface": 180, "Overhang wall": 96,
    },
    filament_used_mm: { "1": 2810 },
    bounding_box: BBOX,
    layer_heights: { min: 0.2, max: 0.2, variable: false },
  },
};

// Simulate a slice: emit the same event stream the real backend does, so the
// progress bar runs and the preview auto-loads + the Preview tab unlocks.
function runMockSlice(): string {
  const job_id = "demo-job-1";
  const ev = (name: string, kind: string, data: Record<string, unknown>) =>
    emit(name, { kind, data: { job_id, ...data } });
  setTimeout(() => ev("slice:plate_started", "PlateStarted", { plate_id: 1 }), 120);
  ([["Slicing model", 25], ["Generating infill", 55], ["Wipe tower", 78], ["Exporting G-code", 95]] as [string, number][])
    .forEach(([stage, percent], i) =>
      setTimeout(() => ev("slice:plate_progress", "PlateProgress", { plate_id: 1, percent, stage }), 350 + i * 320));
  setTimeout(() => ev("slice:plate_finished", "PlateFinished", { plate_id: 1, output_path: "demo.gcode" }), 1700);
  setTimeout(() => emit("slice:job_finished", { kind: "JobFinished", data: { job_id } }), 1800);
  return job_id;
}

const INSTANCE_ID = "inst-a1mini-1";
const PLATE_ID = 1;

// AMS Lite spool colors (OrangeCon orange first).
const AMS_COLORS = ["#f26722", "#2f7de1", "#35b36b", "#e8c341"];

const A1_INSTANCE = {
  id: INSTANCE_ID,
  display_name: "A1 mini",
  vendor_profile_ref: "bambu-lab-a1-mini",
  printer_fragment_slug: "bambu-lab-a1-mini",
  default_filament_fragment_slug: "bambu-pla-basic",
  quality_profile: "0.20mm-standard",
  connection: null,
  extruders: [
    {
      installed_nozzle: { diameter: "0.4", material: "stainless" },
      slots: AMS_COLORS.map((color) => ({
        feed: "ams" as const,
        filament_identity: "bambu-pla-basic",
        color,
        tag_uid: null,
      })),
    },
  ],
  bed: { identity: "bambu-textured-pei" },
  config_overrides: {},
  send_options: {
    bed_leveling: true,
    flow_calibration: true,
    vibration_calibration: false,
    timelapse: false,
  },
  // AMS Lite: one unit, four slots.
  slots: AMS_COLORS.map((color, i) => ({
    ref: { extruder: 0, slot: i },
    label: `AMS slot ${i + 1}`,
    short_label: `A${i + 1}`,
    feed: "ams" as const,
    filament_identity: "bambu-pla-basic",
    color,
    tag_uid: null,
  })),
  ams_units: 1,
};

// The sample model's real bounding box (the OrangeCon M5 StickS3 case, from the
// n3o slice: x 142..170, y 10..71, z 0..16.6). Kept truthful so the objects
// panel shows real dimensions.
const SNAPSHOT = {
  project_uuid: "demo-orangecon-m5-sticks3-case",
  source_path: "m5sticks3_click_case_color_logo_embossed.3mf",
  recovery_origin: null,
  user_overrides: {},
  file_metadata: {},
  meshes: [
    {
      id: 1,
      vertex_count: 0,
      index_count: 0,
      bounding_box: { min: [142.11, 10.21, 0.2], max: [169.81, 71.01, 16.6] },
      provenance: { kind: "Imported", data: "m5sticks3_click_case" },
    },
  ],
  plates: [
    {
      plate_id: PLATE_ID,
      name: "Plate 1",
      printer_identity: "bambu-lab-a1-mini",
      printer_instance_id: INSTANCE_ID,
      material_to_slot: { "1": { extruder: 0, slot: 0 } },
      project_overrides: {},
      quality_profile: null,
      objects: [
        {
          id: 1,
          mesh: 1,
          transform: [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1],
          name: "M5 StickS3 case",
          visible: true,
          extruder_id: 1,
          group: null,
        },
      ],
      selection: [],
      exclusion_zones: [],
      bed: {
        extents: { min: [0, 0, 0], max: [180, 180, 180] },
        grid_spacing: 10,
        origin_marker: [0, 0, 0],
        exclusion_zones: [],
      },
      object_overrides: {},
      groups: {},
    },
  ],
  active_plate_id: PLATE_ID,
};

const FILAMENTS = [
  {
    identity: "bambu-pla-basic",
    display_name: "PLA Basic",
    base_type: "PLA",
    vendor: "Bambu Lab",
    nozzle_temp: 220,
    bed_temp: 60,
    filament_id: "GFA00",
  },
];

const PROCESSES = [
  { slug: "0.20mm-standard", display_name: "0.20mm Standard", layer_height_mm: 0.2, available_for: [] },
];

const CATALOG = [
  {
    identity: "bambu-lab-a1-mini",
    profile: {
      model: "Bambu Lab A1 mini",
      brand: "Bambu Lab",
      brand_short: "B",
      ams_max: 1,
      ams_type: "AMS Lite",
      ams_slots_per_unit: 4,
      supported_build_plates: ["bambu-textured-pei"],
      available_nozzle_diameters: ["0.2", "0.4", "0.6", "0.8"],
      default_bed: "bambu-textured-pei",
      toolheads: [{ default_nozzle_diameter: "0.4", hotend_type: "stainless", max_temp: 300 }],
      build_volume: { min: [0, 0, 0], max: [180, 180, 180] },
      exclusion_zones: [],
      driver_kind: "bambu",
    },
  },
];

export const COMMANDS: Record<string, Handler> = {
  // --- boot: must be exact + consistent ---
  scene_snapshot: () => SNAPSHOT,
  printer_instance_list: () => [A1_INSTANCE],

  printer_instance_get: (a) => (a.id === INSTANCE_ID ? A1_INSTANCE : null),
  // Resolved machine config (key -> value) for the printer settings modal.
  printer_instance_resolved_config: () =>
    Object.fromEntries(
      Object.entries(resolvedEntries as Record<string, { value: string }>).map(
        ([k, v]) => [k, v.value],
      ),
    ),

  // --- boot: safe defaults ---
  printer_catalog: () => CATALOG,
  project_autosave_list: () => [],
  project_autosave_enable: () => null,
  plugin_list: () => [],
  project_history_state: () => ({ can_undo: false, can_redo: false }),
  project_is_dirty: () => false,

  // --- settings/option panels (lazy; empty renders fine) ---
  filament_profile_list: () => FILAMENTS,
  process_fragment_list: () => PROCESSES,
  slicer_options_for_printer: (a) => filtered(processOptions as Opt[], a),
  slicer_machine_options_for_printer: (a) => filtered(machineOptions as Opt[], a),
  slicer_extruder_options_for_printer: (a) => filtered(extruderOptions as Opt[], a),
  slicer_filament_options: (a) => filtered(filamentOptions as Opt[], a),
  plate_cascade_resolve: () => ({ entries: resolvedEntries }),

  // --- slice -> preview flow ---
  slice_active_plate: () => runMockSlice(),
  slice_status: () => ({ kind: "Finished" }),
  slice_cancel: () => null,
  preview_load: () => PREVIEW,
  preview_layer_stats: () => [],
  preview_drop: () => null,
  preview_segment_detail: () => null,
};
