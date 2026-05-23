// React hook for the slice loop (PR-3-4).
//
// Owns the Tauri `slice:*` subscription, a reducer over the event
// stream (see `reducer.ts`), and `start()` / `cancel()` actions that
// wrap the backend commands. The hook is the minimum-viable surface
// the SlicePanel needs — Phase 4 will replace the bundled-defaults
// entrypoint with a project-state-driven call to `slice_start_job`.

import { useCallback, useEffect, useReducer, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import { initialState, reduce } from "./reducer";
import { SLICE_EVENT_NAMES, type JobId, type SliceEvent } from "./types";

const LAST_JOB_KEY = "n3o.slice.last_job_id";

export function useSliceJob() {
  const [state, dispatch] = useReducer(reduce, initialState);
  // Track jobId in a ref too — handlers that fire during async cancel
  // shouldn't capture the stale reducer state.
  const jobIdRef = useRef<JobId | null>(null);
  jobIdRef.current = state.job_id;

  useEffect(() => {
    const unlisteners: UnlistenFn[] = [];
    let mounted = true;

    void (async () => {
      for (const name of SLICE_EVENT_NAMES) {
        const un = await listen<SliceEvent>(name, (e) => {
          dispatch({ type: "event", event: e.payload });
        });
        if (!mounted) {
          un();
          continue;
        }
        unlisteners.push(un);
      }

      // Reconnect path: if a prior renderer session was running a job
      // when the window reloaded, resync from the cached status. The
      // events that already fired are gone but the backend snapshot
      // gets us back to a reasonable starting point.
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

    return () => {
      mounted = false;
      for (const un of unlisteners) un();
    };
  }, []);

  const start = useCallback(
    async (modelPath: string, outputDir: string): Promise<JobId> => {
      const jobId = await invoke<JobId>("slice_start_default_a1mini", {
        modelPath,
        outputDir,
      });
      writeStoredJobId(jobId);
      dispatch({ type: "start", job_id: jobId });
      return jobId;
    },
    [],
  );

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
