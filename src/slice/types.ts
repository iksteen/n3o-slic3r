// Wire-format types for the slice loop (PR-3-4).
//
// Mirrors the Rust shapes in `src-tauri/src/core/slice/`:
//   - `events::SliceEvent` (`#[serde(tag="kind", content="data")]`)
//   - `summary::PlateSummary`
//   - `errors::SliceError` (same tagged shape)
//   - `job::{JobId, JobStatus}`
//
// Wire-shape drift between this file and the Rust enums is the most
// common source of "silently works on one side" bugs — the gizmo
// transform / mesh-buffer fixtures already taught us to mirror serde
// output literally rather than "what we think it should look like."

/** `serde(transparent)` u64 on the Rust side — bare integer on the wire. */
export type JobId = number;

export type PlateSummary = {
  estimated_time_seconds: number;
  estimated_time_text: string;
  // `BTreeMap<u8, f64>` — JSON serializes integer keys as strings.
  filament_used_grams: Record<string, number>;
  filament_used_mm: Record<string, number>;
  filament_used_mm3: Record<string, number>;
  layer_count: number;
  object_count: number;
  bbox_min: [number, number, number] | null;
  bbox_max: [number, number, number] | null;
  output_path: string;
};

export type SliceError =
  | {
      kind: "InvalidConfig";
      data: { setting_key: string; reason: string; raw_message: string };
    }
  | {
      kind: "InvalidGeometry";
      data: { reason: string; raw_message: string };
    }
  | {
      kind: "OutOfBounds";
      data: { plate_id: number | null; raw_message: string };
    }
  | { kind: "Cancelled" }
  | { kind: "Unknown"; data: { raw_message: string } };

export type SliceEvent =
  | { kind: "PlateStarted"; data: { job_id: JobId; plate_id: number } }
  | {
      kind: "PlateProgress";
      data: { job_id: JobId; plate_id: number; percent: number; stage: string };
    }
  | {
      kind: "PlateFinished";
      data: {
        job_id: JobId;
        plate_id: number;
        output_path: string;
        summary: PlateSummary;
      };
    }
  | { kind: "JobFinished"; data: { job_id: JobId } }
  | {
      kind: "JobFailed";
      data: { job_id: JobId; plate_id: number; error: SliceError };
    }
  | {
      kind: "Cancelled";
      data: { job_id: JobId; plate_id_in_progress: number | null };
    };

export const SLICE_EVENT_NAMES = [
  "slice:plate_started",
  "slice:plate_progress",
  "slice:plate_finished",
  "slice:job_finished",
  "slice:job_failed",
  "slice:cancelled",
] as const;

/** Non-fatal libslic3r validation warning, emitted on `slice:plate_warning`
 *  just before the plate finishes. Mirrors Rust `SliceEvent::PlateWarning`,
 *  but kept out of the `SliceEvent` union / `SLICE_EVENT_NAMES` above on
 *  purpose: it's a console/notification concern, not slice-reducer state, so
 *  it isn't routed through `useSliceJob` → `reducer`. */
export type PlateWarningEvent = {
  kind: "PlateWarning";
  data: { job_id: JobId; plate_id: number; message: string };
};

export type SliceStatus =
  | "idle"
  | "starting"
  | "running"
  | "cancelling"
  | "complete"
  | "failed"
  | "cancelled";

/** Flat reducer state — what the SlicePanel renders from directly.
 *  Carries everything observed for the current job plus any post-
 *  terminal residue (summaries, last error). Reset by `{type:"reset"}`
 *  before kicking off a new job. */
export type SliceState = {
  status: SliceStatus;
  job_id: JobId | null;
  /** Plate the worker is currently on, or last reported on. */
  plate_id: number | null;
  percent: number;
  stage: string;
  summaries: PlateSummary[];
  /** Set on `JobFailed`; otherwise null. */
  error: SliceError | null;
  /** Set on `Cancelled`; otherwise null. */
  plate_id_at_cancel: number | null;
};
