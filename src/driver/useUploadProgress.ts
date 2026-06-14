// Live per-driver send-upload state, backed by one shared module-scoped store.
//
// SendControls brackets a send with `beginUpload`/`endUpload` (the latter in a
// `finally`, so it clears on success *and* error — a failed upload doesn't leave
// the window stuck). While active, the backend emits throttled
// `driver:upload_progress` events that fill in the percent. SendProgressWindow
// reads `{ active, progress }` for the active plate's driver and shows the
// shared ProgressWindow while `active`.
//
// Module-scoped + useSyncExternalStore so the value survives the SendControls /
// SendProgressWindow living in different parts of the tree, mirroring
// useDriverStatus.

import { useCallback, useSyncExternalStore } from "react";
import { onEvents } from "../state/eventRouter";
import type { DriverId, UploadProgress } from "./types";

export interface UploadState {
  /** A send invoke is in flight for this driver. */
  active: boolean;
  /** Latest progress event, or null before the first one arrives. */
  progress: UploadProgress | null;
}

const entries = new Map<DriverId, UploadState>();
const subscribers = new Set<() => void>();
const IDLE: UploadState = { active: false, progress: null };

let off: (() => void) | null = null;

function notify(): void {
  for (const cb of subscribers) cb();
}

// Immutable snapshots — replace the object so getSnapshot returns a stable
// reference between updates (required by useSyncExternalStore).
function patch(id: DriverId, next: Partial<UploadState>): void {
  const prev = entries.get(id) ?? IDLE;
  entries.set(id, { ...prev, ...next });
  notify();
}

/** Mark a send in flight for `driverId` (call right before the send invoke).
 *  Resets any stale progress from a previous send. */
export function beginUpload(driverId: DriverId): void {
  entries.set(driverId, { active: true, progress: null });
  notify();
}

/** Clear the in-flight flag (call in the send's `finally` — success or error). */
export function endUpload(driverId: DriverId): void {
  patch(driverId, { active: false });
}

// One shared subscription for the app's lifetime (the event name is constant).
function ensureListening(): void {
  if (off != null) return;
  off = onEvents<UploadProgress>(["driver:upload_progress"], (e) => {
    patch(e.payload.driver_id, { progress: e.payload });
  });
}

/** The send-upload state for `driverId` (idle when none / no driver). */
export function useUploadProgress(driverId: DriverId | null): UploadState {
  const subscribe = useCallback((cb: () => void) => {
    subscribers.add(cb);
    ensureListening();
    return () => {
      subscribers.delete(cb);
    };
  }, []);
  const getSnapshot = useCallback(
    (): UploadState =>
      driverId != null ? (entries.get(driverId) ?? IDLE) : IDLE,
    [driverId],
  );
  return useSyncExternalStore(subscribe, getSnapshot);
}

// Vite HMR teardown — mirror useDriverStatus: drop this module's handler so a
// dev re-eval doesn't double every event.
if (import.meta.hot) {
  import.meta.hot.dispose(() => {
    off?.();
    off = null;
    entries.clear();
    subscribers.clear();
  });
}
