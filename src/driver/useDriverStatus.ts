// Live per-driver status, backed by one shared module-scoped store.
//
// A single app-wide `driver:status_update` subscription fans every
// event into a `Map<DriverId, Entry>`; consumers read their driver's
// slice via `useSyncExternalStore`. This replaces the previous
// one-listener-and-one-copy-per-consumer model, which meant N+1
// listeners for a fleet and — more importantly — re-fetched from
// `null` every time a consumer mounted (entering the Devices view,
// switching the selected printer), causing a first-paint gap. The
// store outlives the React tree, so a remount reads the last-known
// status immediately. Mirrors the module-scoped pattern in
// `useDriverConnections`.

import { useCallback, useSyncExternalStore } from "react";
import { onEvents } from "../state/eventRouter";
import { pushLog } from "../logging/logStore";
import { driverStatus } from "./invokes";
import type { DriverId, PrinterStatus, StatusUpdateEvent } from "./types";

/** Bambu `err_code` from a rejected command, or null. */
function rejectionCode(status: PrinterStatus | null): number | null {
  return status?.extra.kind === "Bambu"
    ? status.extra.data.command_error_code
    : null;
}

export interface UseDriverStatusResult {
  /** `null` until either the initial fetch resolves or the first
   * `driver:status_update` event fires for this driver. */
  status: PrinterStatus | null;
  /** The last error from the initial fetch, or `null`. Cleared when a
   * status update arrives. */
  error: string | null;
}

interface Entry {
  status: PrinterStatus | null;
  error: string | null;
}

// The shared store. Entries are immutable snapshots — `setEntry`
// replaces the object so `useSyncExternalStore`'s getSnapshot returns a
// stable reference between updates (required to avoid render loops).
const entries = new Map<DriverId, Entry>();
const subscribers = new Set<() => void>();
const seeded = new Set<DriverId>();
const EMPTY: UseDriverStatusResult = { status: null, error: null };

let statusOff: (() => void) | null = null;

function notify(): void {
  for (const cb of subscribers) cb();
}

function setEntry(id: DriverId, patch: Partial<Entry>): void {
  const prev = entries.get(id) ?? { status: null, error: null };
  const next = { ...prev, ...patch };
  entries.set(id, next);
  // Log a freshly-arrived command rejection once (84033543 = Developer
  // Mode off). Fired from this app-lifetime store so it reaches the user
  // in any view; an error log auto-opens the console.
  const code = rejectionCode(next.status);
  if (code != null && code !== rejectionCode(prev.status)) {
    const hex = `0x${(code >>> 0).toString(16).toUpperCase().padStart(8, "0")}`;
    pushLog(
      "error",
      code === 84033543
        ? `Print rejected (err ${hex}): this printer needs Developer Mode for third-party software. On the printer, enable LAN Only Mode → “LAN Only” + “Developer Mode”, then reconnect.`
        : `Printer rejected a command (err ${hex}).`,
    );
  }
  notify();
}

// One shared subscription for the app's lifetime, via the router. The channel
// name is constant (drivers come and go, the event isn't), so once started it's
// never torn down — there's nothing per-consumer to clean up. The router
// shares this `driver:status_update` subscription with useDriverConnections'
// reconciler, which reads the same stream.
function ensureListening(): void {
  if (statusOff != null) return;
  statusOff = onEvents<StatusUpdateEvent>(["driver:status_update"], (e) => {
    setEntry(e.payload.driver_id, { status: e.payload.status, error: null });
  });
}

/** Drop a driver's cached status + seed marker. Call when a driver is
 *  unregistered/replaced so `entries`/`seeded` don't accumulate dead
 *  ids for the app's lifetime (driver ids are monotonic and never
 *  reused, so a stale entry would otherwise never be read again). */
export function forgetDriver(id: DriverId): void {
  const had = entries.delete(id);
  seeded.delete(id);
  if (had) notify();
}

// One-shot initial fetch per driver, so a consumer has a value before
// the first event arrives. A live event may beat the fetch; don't
// clobber it.
function seed(id: DriverId): void {
  if (seeded.has(id)) return;
  seeded.add(id);
  void driverStatus(id)
    .then((s) => {
      if (entries.get(id)?.status == null) setEntry(id, { status: s });
    })
    .catch((e) => {
      seeded.delete(id); // allow a retry on a later mount
      setEntry(id, { error: `status fetch failed: ${String(e)}` });
    });
}

export function useDriverStatus(
  driverId: DriverId | null,
): UseDriverStatusResult {
  const subscribe = useCallback(
    (cb: () => void) => {
      subscribers.add(cb);
      ensureListening();
      if (driverId != null) seed(driverId);
      return () => {
        subscribers.delete(cb);
      };
    },
    [driverId],
  );
  const getSnapshot = useCallback(
    (): UseDriverStatusResult =>
      driverId != null ? (entries.get(driverId) ?? EMPTY) : EMPTY,
    [driverId],
  );
  return useSyncExternalStore(subscribe, getSnapshot);
}

// Vite HMR teardown: when this module re-evaluates in dev, the old
// listener's closure still references the prior module's `entries` /
// `subscribers` maps; without this it would stay subscribed forever and
// double every status event. Production builds skip this (no
// import.meta.hot). Mirrors the dispose hook in `useDriverConnections`.
if (import.meta.hot) {
  import.meta.hot.dispose(() => {
    // Detach only this module's handler — the router's shared
    // `driver:status_update` subscription stays up for other consumers.
    statusOff?.();
    statusOff = null;
    entries.clear();
    subscribers.clear();
    seeded.clear();
  });
}
