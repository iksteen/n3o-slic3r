// TS mirrors of the Rust preview types (PR-6-7).
//
// Match the serde wire shapes from `core/preview/{ir,colors,stats,
// registry,commands}.rs`. Keep narrow — only what the frontend
// actually consumes; full types are inferred from invokes.ts.

/** `serde(transparent)` u64 on the Rust side — bare integer on the
 * wire. */
export type PreviewHandle = number;

/** `core::preview::colors::ColorMode`. Serde tag-less enum →
 * camelCase string variants on the wire? No — it's a normal enum
 * without `#[serde(tag = …)]`, which serializes to the variant
 * name as a string. */
export type ColorMode =
  | "Feature"
  | "Speed"
  | "Flow"
  | "LayerTime"
  | "Tool";

export type Palette = "Default" | "Classic";

export interface BoundingBox {
  min: [number, number, number];
  max: [number, number, number];
}

export interface PreviewLoadResponse {
  handle: PreviewHandle;
  header: HeaderMetadata;
  layer_count: number;
  extrusion_count: number;
  travel_count: number;
  retraction_count: number;
  bounding_box: BoundingBox;
  job_stats: FullJobStats;
}

export interface HeaderMetadata {
  slicer: SlicerOrigin | null;
  slicer_version: string | null;
  estimated_time: string | null;
  filament_used: FilamentUsage[];
  layer_count: number | null;
  object_count: number | null;
  printer_model: string | null;
  bbox_min: [number, number, number] | null;
  bbox_max: [number, number, number] | null;
  raw_settings: Record<string, string>;
}

export type SlicerOrigin = "Orca" | "PrusaSlicer" | "Cura" | "Unknown";

export interface FilamentUsage {
  unit: string;
  value: string;
}

export interface FullJobStats {
  total_duration_seconds: number;
  layer_count: number;
  feature_breakdown: Record<string, number>;
  filament_used_mm: Record<string, number>;
  bounding_box: BoundingBox;
  layer_heights: HeightStats;
}

export interface HeightStats {
  min: number;
  max: number;
  variable: boolean;
}

export interface PerLayerStats {
  layer_index: number;
  z: number;
  layer_height: number;
  duration_seconds: number;
  max_speed: number;
  filament_used_mm: Record<string, number>;
  feature_breakdown: Record<string, number>;
}

export interface SegmentDetail {
  source_line_text: string;
  start: [number, number, number];
  end: [number, number, number];
  speed: number;
  feature: FeatureType;
  layer_index: number;
  tool: number;
  extrusion_mm: number;
}

/** Mirror of Rust's `FeatureType` — internally-tagged enum where
 * unit variants serialize as bare strings, but `Other(String)`
 * serializes as `{ "Other": "..." }`. */
export type FeatureType =
  | "Perimeter"
  | "ExternalPerimeter"
  | "Infill"
  | "SolidInfill"
  | "TopSolidInfill"
  | "Bridge"
  | "Support"
  | "Skirt"
  | "Brim"
  | "Travel"
  | { Other: string };

/** Layer-window state PR-6-9 owns + threads down to the renderer. */
export type LayerWindow =
  | { mode: "single"; layer: number }
  | { mode: "up-to"; max: number }
  | { mode: "range"; min: number; max: number };
