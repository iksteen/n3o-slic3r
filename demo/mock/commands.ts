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

// Group id for the two-part OrangeCon case (Rust GroupId(Uuid)).
const GROUP_ID = "b1a7c0de-0000-4000-8000-000000000001";

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
const DRIVER_ID = 1;

// AMS Lite spool colors (OrangeCon orange first; A4 = black, bound to M2).
const AMS_COLORS = ["#f26722", "#2f7de1", "#35b36b", "#111111"];

// A connected-but-idle Bambu status report. The header dot reads this (via the
// driver:status_update event → ConnectionSummary.runtime) and the Devices
// monitor seeds off driver_status; both show "Idle". The camera is faked
// offline separately (camera_start rejects).
const idleStatus = () => ({
  connection: { state: "Connected" },
  job: null,
  temps: {
    nozzles: [{ current: 27, target: 0 }],
    bed: { current: 23, target: 0 },
    chamber: null,
  },
  extra: {
    kind: "Bambu",
    data: {
      mounted_plate: null,
      current_stage: null,
      print_error_code: null,
      command_error_code: null,
      fan_speed: 0,
      ams: {
        units: [
          {
            id: 0,
            trays: AMS_COLORS.map((c, i) => ({
              id: i,
              // AmsFilament color is RRGGBBAA hex without the leading '#'.
              identity: { tray_type: "PLA", color: c.slice(1) + "ff", sub_brand: null, multi_colors: [], filament_id: "GFA00" },
            })),
          },
        ],
        active_slot: null,
      },
      external_spool: null,
    },
  },
  last_updated: Date.now(),
});

// Push the idle status so the picker dot flips green. One-shot at connect (the
// dot needs a runtime event, not just a live registry entry) + a slow repush so
// it stays fresh regardless of listener-install ordering.
let statusFeed = false;
function startStatusFeed(): void {
  if (statusFeed) return;
  statusFeed = true;
  const push = () => emit("driver:status_update", { driver_id: DRIVER_ID, status: idleStatus() });
  setTimeout(push, 300);
  setInterval(push, 5000);
}

const A1_INSTANCE = {
  id: INSTANCE_ID,
  display_name: "A1 mini",
  vendor_profile_ref: "bambu-lab-a1-mini",
  printer_fragment_slug: "bambu-lab-a1-mini",
  default_filament_fragment_slug: "bambu-pla-basic",
  quality_profile: "0.20mm-standard",
  // A usable (fake) LAN connection so the reconciler auto-registers on boot and
  // the printer reads as connected. Nothing is actually reached — the driver
  // lifecycle commands below are canned.
  connection: { kind: "bambu", host: "192.168.1.42", access_code: "a1b2c3d4" },
  extruders: [
    {
      installed_nozzle: { diameter: "0.4", material: "stainless" },
      // 4 AMS Lite slots + the always-present external ("Ext") spool slot,
      // matching the real A1 mini topology (flatten_slots) and the Devices
      // monitor loadout, which always shows the Ext row.
      slots: [
        ...AMS_COLORS.map((color) => ({
          feed: "ams" as const,
          filament_identity: "bambu-pla-basic",
          color,
          tag_uid: null,
        })),
        { feed: "direct" as const, filament_identity: null, color: null, tag_uid: null },
      ],
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
  // AMS Lite: one unit, four slots, plus the trailing external ("Ext") slot.
  slots: [
    ...AMS_COLORS.map((color, i) => ({
      ref: { extruder: 0, slot: i },
      label: `AMS slot ${i + 1}`,
      short_label: `A${i + 1}`,
      feed: "ams" as const,
      filament_identity: "bambu-pla-basic",
      color,
      tag_uid: null,
    })),
    {
      ref: { extruder: 0, slot: 4 },
      label: "Ext",
      short_label: "Ext",
      feed: "direct" as const,
      filament_identity: null,
      color: null,
      tag_uid: null,
    },
  ],
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
      bounding_box: { min: [141.9, 10.0, 0.0], max: [169.2, 71.22, 16.63] },
      provenance: { kind: "Imported", data: "ORANGECON_body" },
    },
    {
      id: 2,
      vertex_count: 0,
      index_count: 0,
      bounding_box: { min: [167.8, 25.4, 6.5], max: [170.0, 56.65, 10.14] },
      provenance: { kind: "Imported", data: "ORANGECON_logo" },
    },
  ],
  plates: [
    {
      plate_id: PLATE_ID,
      name: "Plate 1",
      printer_identity: "bambu-lab-a1-mini",
      printer_instance_id: INSTANCE_ID,
      // Two materials: body → slot 0 / A1 (orange), logo insert → slot 3 / A4 (black).
      material_to_slot: { "1": { extruder: 0, slot: 0 }, "2": { extruder: 0, slot: 3 } },
      project_overrides: {},
      quality_profile: null,
      // Two grouped parts of the OrangeCon case, each on its own material —
      // demonstrates grouping + multi-material assignment.
      objects: [
        {
          id: 1,
          mesh: 1,
          transform: [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1],
          name: "Case body",
          visible: true,
          extruder_id: 1,
          group: GROUP_ID,
        },
        {
          id: 2,
          mesh: 2,
          transform: [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1],
          name: "ORANGECON logo insert",
          visible: true,
          extruder_id: 2,
          group: GROUP_ID,
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
      groups: { [GROUP_ID]: { name: "M5 StickS3 case" } },
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

  // --- fake connected-but-idle driver (offline camera) ---
  driver_register: () => DRIVER_ID,
  driver_test_connection: () => null,
  driver_connect: () => {
    startStatusFeed();
    return null;
  },
  driver_disconnect: () => null,
  driver_unregister: () => null,
  driver_status: () => idleStatus(),
  // Reject so the camera panel shows its offline placeholder (String(e) is the
  // detail line) instead of trying to open a stream.
  camera_start: () => {
    throw "No camera signal — printer webcam is off";
  },
  camera_stop: () => null,
  // Status-driven sync no-ops (armed only by user action; safe to return the
  // instance unchanged if they ever fire).
  printer_instance_sync_from_driver: () => A1_INSTANCE,
  printer_instance_set_ams_units: () => A1_INSTANCE,

  // --- slice -> preview flow ---
  slice_active_plate: () => runMockSlice(),
  slice_status: () => ({ kind: "Finished" }),
  slice_cancel: () => null,
  preview_load: () => PREVIEW,
  preview_layer_stats: () => [],
  preview_drop: () => null,
  preview_segment_detail: () => null,
};
