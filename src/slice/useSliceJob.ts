// React hook for the slice loop (PR-3-4, PR-6-3).
//
// Owns the Tauri `slice:*` subscription, a reducer over the event
// stream (see `reducer.ts`), and `start()` / `cancel()` actions that
// wrap the backend commands. Post-PR-6-3 the slice is driven by
// live project state via `slice_active_plate` — no model path, no
// output dir. The backend builds the SliceJobInput from the scene
// and picks a temp output dir per job.

import { useCallback, useEffect, useReducer, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { onEvents } from "../state/eventRouter";

import { initialState, reduce } from "./reducer";
import { SLICE_EVENT_NAMES, type JobId, type SliceEvent } from "./types";
import { refreshThumbnailCache } from "../viewport/thumbnailCapture";

const LAST_JOB_KEY = "n3o.slice.last_job_id";

export function useSliceJob() {
  const [state, dispatch] = useReducer(reduce, initialState);
  // Track jobId in a ref too — handlers that fire during async cancel
  // shouldn't capture the stale reducer state.
  const jobIdRef = useRef<JobId | null>(null);
  jobIdRef.current = state.job_id;

  useEffect(() => {
    // Subscribe via the router (synchronous registration — no subscribe-race
    // to guard).
    const off = onEvents<SliceEvent>(SLICE_EVENT_NAMES, (e) => {
      dispatch({ type: "event", event: e.payload });
    });

    // Reconnect path: if a prior renderer session was running a job
    // when the window reloaded, resync from the cached status. The
    // events that already fired are gone but the backend snapshot
    // gets us back to a reasonable starting point.
    void (async () => {
      const saved = readStoredJobId();
      if (saved != null) {
        try {
          // We don't get individual events back, but the Running
          // snapshot lets the panel resume with the most recent
          // percent + stage so the user isn't stuck at "idle".
          type JobStatus =
            | { kind: "Queued" }
            | { kind: "Running"; data: { plate_id: number; percent: number; stage: string } }
            | { kind: "Cancelling" }
            | { kind: "Finished" }
            | { kind: "Failed"; data: { plate_id: number; error: string } }
            | { kind: "Cancelled"; data: { plate_id_in_progress: number | null } };
          const status = await invoke<JobStatus>("slice_status", { jobId: saved });
          if (status.kind === "Running") {
            dispatch({ type: "start", job_id: saved });
            dispatch({
              type: "event",
              event: {
                kind: "PlateProgress",
                data: {
                  job_id: saved,
                  plate_id: status.data.plate_id,
                  percent: status.data.percent,
                  stage: status.data.stage,
                },
              },
            });
          } else {
            // Job terminated while we were away. Clear the cached id
            // so future reloads stay clean.
            clearStoredJobId();
          }
        } catch {
          // `slice_status` errors when the registry has dropped the
          // job (terminal events arrived); treat as "no job to resume."
          clearStoredJobId();
        }
      }
    })();

    return off;
  }, []);

  const start = useCallback(async (): Promise<JobId> => {
    // Capture the plate preview now, while the edit viewport is still
    // mounted — slicing flips the app into preview mode (which unmounts
    // it), and send/export then read this cached thumbnail. Awaited so an
    // async (wgpu, offscreen+IPC) capture lands before we move on.
    await refreshThumbnailCache();
    // Backend uses the project's active plate when `plateId` is
    // null. Future "slice plate N" affordances can pass a specific
    // PlateId here.
    const jobId = await invoke<JobId>("slice_active_plate", {
      plateId: null,
    });
    writeStoredJobId(jobId);
    dispatch({ type: "start", job_id: jobId });
    return jobId;
  }, []);

  const cancel = useCallback(async () => {
    const id = jobIdRef.current;
    if (id == null) return;
    dispatch({ type: "cancel_requested" });
    try {
      await invoke("slice_cancel", { jobId: id });
    } catch (err) {
      // If the worker already terminated, the cancel call errors —
      // not an issue, the terminal event has already advanced our
      // status. Swallow.
      console.debug("[slice] cancel after terminal:", err);
    }
  }, []);

  const reset = useCallback(() => {
    clearStoredJobId();
    dispatch({ type: "reset" });
  }, []);

  return { state, start, cancel, reset };
}

function readStoredJobId(): JobId | null {
  try {
    const raw = window.localStorage.getItem(LAST_JOB_KEY);
    if (raw == null) return null;
    const id = Number(raw);
    return Number.isFinite(id) ? id : null;
  } catch {
    return null;
  }
}

function writeStoredJobId(id: JobId): void {
  try {
    window.localStorage.setItem(LAST_JOB_KEY, String(id));
  } catch {
    // ignore: storage disabled / quota
  }
}

function clearStoredJobId(): void {
  try {
    window.localStorage.removeItem(LAST_JOB_KEY);
  } catch {
    // ignore
  }
}
