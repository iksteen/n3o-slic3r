// Module-level store for in-flight pressure-advance calibration, keyed by
// printer instance id. It lives outside React so the calibration loop — and
// its per-row status — survives the FlowDynamics component unmounting when the
// user switches tabs or printers. Keying by instance id isolates each
// printer's run; the loop keeps writing to its own printer's entry regardless
// of what's currently on screen.
//
// This is orchestration state, not fetched data, so it's a plain external
// store (subscribe + snapshot for `useSyncExternalStore`) rather than a
// queryCache query. Mirrors the store pattern in `src/state/queryCache.ts`.

import {
  driverCalibratePa,
  driverErrorMessage,
  driverParkExtruder,
} from "./invokes";
import type { DriverId } from "./types";

export type CalPhase = "queued" | "running" | "done" | "error";

export interface CalState {
  phase: CalPhase;
  message?: string;
  k?: number;
}

/** A printer's calibration state: whether a run is active, and the per-row
 *  (`extruder-slot`) phase for the current/last run. */
export interface InstanceCal {
  busy: boolean;
  rows: Record<string, CalState>;
}

/** One filament to calibrate, in run order. */
export interface CalTarget {
  key: string;
  extruderIndex: number;
  slotIndex: number;
}

/** Stable empty snapshot for printers with no run — a constant reference so
 *  `useSyncExternalStore` bails out of re-rendering when nothing changed. */
const EMPTY: InstanceCal = { busy: false, rows: {} };

const store = new Map<string, InstanceCal>();
const listeners = new Set<() => void>();

function emit(): void {
  for (const l of listeners) l();
}

/** Subscribe to any calibration-state change (all instances). */
export function subscribe(cb: () => void): () => void {
  listeners.add(cb);
  return () => {
    listeners.delete(cb);
  };
}

/** Current calibration state for a printer. Returns the shared `EMPTY`
 *  reference when idle so unchanged snapshots stay referentially stable. */
export function getInstanceCal(instanceId: string): InstanceCal {
  return store.get(instanceId) ?? EMPTY;
}

function write(instanceId: string, next: InstanceCal): void {
  store.set(instanceId, next);
  emit();
}

function patchRow(instanceId: string, key: string, cal: CalState): void {
  const cur = store.get(instanceId) ?? EMPTY;
  write(instanceId, { ...cur, rows: { ...cur.rows, [key]: cal } });
}

/** Run a calibration sequence for `targets` on `instanceId`, in order (the
 *  printer calibrates one toolhead at a time). Fire-and-forget: it drives the
 *  store, not the caller, so it continues across remounts. No-op if a run is
 *  already active for this printer. */
export async function runCalibration(
  instanceId: string,
  driverId: DriverId,
  targets: CalTarget[],
): Promise<void> {
  if (targets.length === 0) return;
  if ((store.get(instanceId) ?? EMPTY).busy) return;

  const queued: Record<string, CalState> = {};
  for (const t of targets) queued[t.key] = { phase: "queued" };
  write(instanceId, { busy: true, rows: queued });

  for (const t of targets) {
    patchRow(instanceId, t.key, { phase: "running" });
    try {
      const k = await driverCalibratePa(
        driverId,
        instanceId,
        t.extruderIndex,
        t.slotIndex,
      );
      patchRow(instanceId, t.key, { phase: "done", k });
    } catch (e) {
      patchRow(instanceId, t.key, {
        phase: "error",
        message: driverErrorMessage(e),
      });
    }
  }

  // Cycle done — park the active (last-calibrated) toolhead so the machine
  // isn't left holding a picked one. Best-effort: a park failure mustn't
  // wedge the busy state.
  try {
    await driverParkExtruder(driverId);
  } catch (e) {
    console.error("[pa-calibration] park failed", driverErrorMessage(e));
  }

  const done = store.get(instanceId) ?? EMPTY;
  write(instanceId, { ...done, busy: false });
}
