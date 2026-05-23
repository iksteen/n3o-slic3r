// Pure reducer over the Tauri `slice:*` event stream (PR-3-4).
//
// Lives separately from `useSliceJob.ts` so the vitest can drive it
// without spinning up React + the Tauri mocks. The hook layers the
// `invoke()` calls + `listen()` subscriptions on top of this; every
// state transition the panel renders flows through `reduce`.

import type {
  JobId,
  PlateSummary,
  SliceError,
  SliceEvent,
  SliceState,
} from "./types";

export const initialState: SliceState = {
  status: "idle",
  job_id: null,
  plate_id: null,
  percent: 0,
  stage: "",
  summaries: [],
  error: null,
  plate_id_at_cancel: null,
};

export type SliceAction =
  | { type: "start"; job_id: JobId }
  | { type: "event"; event: SliceEvent }
  | { type: "cancel_requested" }
  | { type: "reset" }
  | { type: "hydrate_summaries"; summaries: PlateSummary[] };

export function reduce(state: SliceState, action: SliceAction): SliceState {
  switch (action.type) {
    case "reset":
      return initialState;

    case "start":
      // Always clears the residue of any prior run; the panel relies
      // on this so a second slice doesn't show the previous
      // job's summaries before the first PlateStarted lands.
      return { ...initialState, status: "starting", job_id: action.job_id };

    case "cancel_requested":
      if (state.status !== "running" && state.status !== "starting") {
        return state;
      }
      return { ...state, status: "cancelling" };

    case "hydrate_summaries":
      return { ...state, summaries: action.summaries };

    case "event":
      return applyEvent(state, action.event);
  }
}

function applyEvent(state: SliceState, event: SliceEvent): SliceState {
  // Late events from a previous (terminated) job arrive on the same
  // channel because Tauri events are global. Filter by job_id —
  // the reducer's authoritative job_id wins.
  const eventJobId = sliceEventJobId(event);
  if (state.job_id != null && eventJobId != null && eventJobId !== state.job_id) {
    return state;
  }

  switch (event.kind) {
    case "PlateStarted":
      return {
        ...state,
        status: "running",
        job_id: event.data.job_id,
        plate_id: event.data.plate_id,
        percent: 0,
        stage: "",
      };

    case "PlateProgress":
      // Once the user has hit Cancel we hold the "cancelling" status
      // until the worker acknowledges with Cancelled — incoming
      // progress ticks don't flip us back to running.
      if (state.status === "cancelling") {
        return state;
      }
      return {
        ...state,
        status: "running",
        plate_id: event.data.plate_id,
        percent: event.data.percent,
        stage: event.data.stage,
      };

    case "PlateFinished":
      return {
        ...state,
        summaries: [...state.summaries, event.data.summary],
        percent: 100,
      };

    case "JobFinished":
      return {
        ...state,
        status: "complete",
        percent: 100,
        plate_id: null,
        stage: "",
      };

    case "JobFailed":
      return {
        ...state,
        status: "failed",
        error: event.data.error,
        plate_id: event.data.plate_id,
      };

    case "Cancelled":
      return {
        ...state,
        status: "cancelled",
        plate_id_at_cancel: event.data.plate_id_in_progress,
      };
  }
}

function sliceEventJobId(event: SliceEvent): JobId | null {
  switch (event.kind) {
    case "PlateStarted":
    case "PlateProgress":
    case "PlateFinished":
    case "JobFinished":
    case "JobFailed":
    case "Cancelled":
      return event.data.job_id;
  }
}

/** Human-readable summary of a typed SliceError. The panel renders
 *  this in a toast on JobFailed; tests assert against it too. */
export function sliceErrorMessage(error: SliceError): string {
  switch (error.kind) {
    case "InvalidConfig":
      return error.data.setting_key
        ? `invalid config (${error.data.setting_key}): ${error.data.reason}`
        : `invalid config: ${error.data.reason}`;
    case "InvalidGeometry":
      return `invalid geometry: ${error.data.reason}`;
    case "OutOfBounds":
      return error.data.plate_id != null
        ? `object out of bounds on plate ${error.data.plate_id}`
        : "object out of bounds";
    case "Cancelled":
      return "slice cancelled";
    case "Unknown":
      return error.data.raw_message;
  }
}
