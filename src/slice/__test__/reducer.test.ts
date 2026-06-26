// Reducer unit tests.
//
// The hook layers Tauri subscriptions + invoke()s on top, but the
// state machine that drives the panel is the pure `reduce` function
// in `reducer.ts`. Driving it from a literal event sequence keeps
// the test independent of React + the @tauri-apps/api mocks.

import { describe, expect, it } from "vitest";
import { initialState, reduce, sliceErrorMessage } from "../reducer";
import type { PlateSummary, SliceEvent } from "../types";

function blankSummary(plate_id: number): PlateSummary {
  return {
    estimated_time_seconds: 123,
    estimated_time_text: "2m 3s",
    filament_used_grams: { "0": 4.2 },
    filament_used_mm: { "0": 1400 },
    filament_used_mm3: { "0": 3500 },
    layer_count: 47,
    object_count: 1,
    bbox_min: [0, 0, 0],
    bbox_max: [20, 20, 20],
    output_path: `/tmp/plate_${plate_id}.gcode`,
  };
}

function play(events: SliceEvent[]) {
  let state = reduce(initialState, { type: "start", job_id: 7 });
  for (const e of events) {
    state = reduce(state, { type: "event", event: e });
  }
  return state;
}

describe("slice reducer", () => {
  it("walks happy path: started → progress×4 → finished → job_finished", () => {
    const state = play([
      { kind: "PlateStarted", data: { job_id: 7, plate_id: 1 } },
      {
        kind: "PlateProgress",
        data: { job_id: 7, plate_id: 1, percent: 10, stage: "perimeter" },
      },
      {
        kind: "PlateProgress",
        data: { job_id: 7, plate_id: 1, percent: 30, stage: "infill" },
      },
      {
        kind: "PlateProgress",
        data: { job_id: 7, plate_id: 1, percent: 60, stage: "infill" },
      },
      {
        kind: "PlateProgress",
        data: { job_id: 7, plate_id: 1, percent: 90, stage: "skirt" },
      },
      {
        kind: "PlateFinished",
        data: {
          job_id: 7,
          plate_id: 1,
          output_path: "/tmp/plate_1.gcode",
          summary: blankSummary(1),
        },
      },
      { kind: "JobFinished", data: { job_id: 7 } },
    ]);

    expect(state.status).toBe("complete");
    expect(state.percent).toBe(100);
    expect(state.summaries).toHaveLength(1);
    expect(state.summaries[0].layer_count).toBe(47);
    expect(state.error).toBeNull();
  });

  it("ignores events for a stale job_id", () => {
    let state = reduce(initialState, { type: "start", job_id: 7 });
    state = reduce(state, {
      type: "event",
      event: { kind: "PlateStarted", data: { job_id: 99, plate_id: 1 } },
    });
    expect(state.status).toBe("starting");
    expect(state.plate_id).toBeNull();
  });

  it("transitions to failed with the typed error attached", () => {
    const state = play([
      { kind: "PlateStarted", data: { job_id: 7, plate_id: 1 } },
      {
        kind: "JobFailed",
        data: {
          job_id: 7,
          plate_id: 1,
          error: {
            kind: "InvalidConfig",
            data: {
              setting_key: "layer_height",
              reason: "must be > 0",
              raw_message: "layer_height invalid",
            },
          },
        },
      },
    ]);

    expect(state.status).toBe("failed");
    expect(state.error).toEqual({
      kind: "InvalidConfig",
      data: {
        setting_key: "layer_height",
        reason: "must be > 0",
        raw_message: "layer_height invalid",
      },
    });
    expect(sliceErrorMessage(state.error!)).toBe(
      "invalid config (layer_height): must be > 0",
    );
  });

  it("holds 'cancelling' through progress ticks until Cancelled arrives", () => {
    let state = play([
      { kind: "PlateStarted", data: { job_id: 7, plate_id: 1 } },
      {
        kind: "PlateProgress",
        data: { job_id: 7, plate_id: 1, percent: 20, stage: "perimeter" },
      },
    ]);
    state = reduce(state, { type: "cancel_requested" });
    expect(state.status).toBe("cancelling");

    // A late progress tick arrives — must NOT flip back to running.
    state = reduce(state, {
      type: "event",
      event: {
        kind: "PlateProgress",
        data: { job_id: 7, plate_id: 1, percent: 25, stage: "perimeter" },
      },
    });
    expect(state.status).toBe("cancelling");

    state = reduce(state, {
      type: "event",
      event: { kind: "Cancelled", data: { job_id: 7, plate_id_in_progress: 1 } },
    });
    expect(state.status).toBe("cancelled");
    expect(state.plate_id_at_cancel).toBe(1);
  });

  it("`reset` returns to initial state", () => {
    const state = play([
      { kind: "PlateStarted", data: { job_id: 7, plate_id: 1 } },
      { kind: "JobFinished", data: { job_id: 7 } },
    ]);
    expect(state.status).toBe("complete");
    const cleared = reduce(state, { type: "reset" });
    expect(cleared).toEqual(initialState);
  });

  it("`start` clears prior summaries (back-to-back slices)", () => {
    const after = play([
      { kind: "PlateStarted", data: { job_id: 7, plate_id: 1 } },
      {
        kind: "PlateFinished",
        data: {
          job_id: 7,
          plate_id: 1,
          output_path: "/tmp/plate_1.gcode",
          summary: blankSummary(1),
        },
      },
      { kind: "JobFinished", data: { job_id: 7 } },
    ]);
    const restarted = reduce(after, { type: "start", job_id: 8 });
    expect(restarted.summaries).toEqual([]);
    expect(restarted.status).toBe("starting");
    expect(restarted.job_id).toBe(8);
  });
});

describe("sliceErrorMessage", () => {
  it("formats each variant", () => {
    expect(
      sliceErrorMessage({
        kind: "InvalidConfig",
        data: { setting_key: "", reason: "bad", raw_message: "raw" },
      }),
    ).toBe("invalid config: bad");
    expect(
      sliceErrorMessage({
        kind: "OutOfBounds",
        data: { plate_id: 2, raw_message: "" },
      }),
    ).toBe("object out of bounds on plate 2");
    expect(sliceErrorMessage({ kind: "Cancelled" })).toBe("slice cancelled");
    expect(
      sliceErrorMessage({ kind: "Unknown", data: { raw_message: "boom" } }),
    ).toBe("boom");
  });
});
