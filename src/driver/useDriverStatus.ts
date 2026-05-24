// React hook bridging the `driver:status_update` Tauri event to
// a state value the printer panel can render directly (PR-7a-7).
//
// Subscribes to the event channel filtered by `driverId`; fetches
// the initial status on mount via `driver_status` so the panel
// has something to render before the first event fires.
// Unsubscribes on unmount + when the `driverId` changes.

import { useEffect, useState } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { driverStatus } from "./invokes";
import type { DriverId, PrinterStatus, StatusUpdateEvent } from "./types";

export interface UseDriverStatusResult {
  /** `null` until either the initial fetch resolves or the first
   * `driver:status_update` event fires. */
  status: PrinterStatus | null;
  /** The last error from either the initial fetch or any failed
   * invoke. Cleared when the next status update arrives. `null`
   * during healthy operation. */
  error: string | null;
}

export function useDriverStatus(
  driverId: DriverId | null,
): UseDriverStatusResult {
  const [status, setStatus] = useState<PrinterStatus | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (driverId == null) {
      setStatus(null);
      setError(null);
      return;
    }

    let cancelled = false;
    let unlisten: UnlistenFn | null = null;

    // 1. Subscribe BEFORE the initial fetch so we don't miss an
    //    event fired between the two. The subscribe path filters
    //    on driver_id so multiple panels-per-app stay isolated.
    const subscribe = async (): Promise<void> => {
      try {
        unlisten = await listen<StatusUpdateEvent>(
          "driver:status_update",
          (e) => {
            if (cancelled) return;
            if (e.payload.driver_id !== driverId) return;
            setStatus(e.payload.status);
            setError(null);
          },
        );
      } catch (e) {
        if (!cancelled) {
          setError(`subscribe failed: ${String(e)}`);
        }
      }
    };

    const initial = async (): Promise<void> => {
      try {
        const s = await driverStatus(driverId);
        if (!cancelled) setStatus(s);
      } catch (e) {
        if (!cancelled) {
          setError(`status fetch failed: ${String(e)}`);
        }
      }
    };

    void subscribe().then(initial);

    return () => {
      cancelled = true;
      if (unlisten) unlisten();
    };
  }, [driverId]);

  return { status, error };
}
