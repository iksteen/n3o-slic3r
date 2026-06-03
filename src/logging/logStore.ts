// App-wide log store backing the error console.
//
// Logs originate in many subsystems (the slice loop, the scene viewport's
// transient toasts, drivers), and the console that shows them lives in App
// while the producers mount/unmount independently. So the store sits at
// module scope — the same pattern as `towerMeshCache` / `useDriverStatus` —
// and `useLogs()` surfaces it to React via `useSyncExternalStore`. Non-React
// code (Tauri event listeners) pushes through the plain `pushLog` function.

import { useSyncExternalStore } from "react";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { onEvents } from "../state/eventRouter";
import type { PlateWarningEvent, SliceEvent } from "../slice/types";
import { sliceErrorMessage } from "../slice/reducer";

export type LogLevel = "info" | "warn" | "error";

export interface LogEntry {
  /** Monotonic id — a stable React key (the buffer is a sliding window, so
   *  array indices shift; `ts` can collide within a millisecond). */
  id: number;
  /** Epoch millis (the console renders HH:MM:SS). */
  ts: number;
  level: LogLevel;
  msg: string;
}

// Cap the buffer so a long session can't grow it unbounded; the console only
// ever shows the tail anyway.
const MAX_LOGS = 200;

let nextId = 0;
let logs: LogEntry[] = [];
const subscribers = new Set<() => void>();

function emit(): void {
  for (const cb of subscribers) cb();
}

// Console open/closed lives in the store, not in the ErrorConsole component,
// so it survives the console being re-mounted in a different canvas frame
// when the user switches between prepare and preview.
let consoleOpen = false;
const openSubscribers = new Set<() => void>();

export function setConsoleOpen(open: boolean): void {
  if (consoleOpen === open) return;
  consoleOpen = open;
  for (const cb of openSubscribers) cb();
}

/** React hook: whether the error console is open. */
export function useConsoleOpen(): boolean {
  return useSyncExternalStore(
    (cb) => {
      openSubscribers.add(cb);
      return () => {
        openSubscribers.delete(cb);
      };
    },
    () => consoleOpen,
  );
}

/** Append a log entry. Safe from anywhere — React render-effects, event
 *  listeners, or imperative callbacks. Replaces the array (new reference) so
 *  `useSyncExternalStore` re-renders. */
export function pushLog(level: LogLevel, msg: string): void {
  logs = [...logs, { id: nextId++, ts: Date.now(), level, msg }].slice(-MAX_LOGS);
  emit();
  // An error pops the console open automatically (warnings only badge it).
  if (level === "error") setConsoleOpen(true);
}

/** Drop every entry (the console's Clear button). No-op when already empty,
 *  so it can't spuriously re-render. */
export function clearLogs(): void {
  if (logs.length === 0) return;
  logs = [];
  emit();
}

/** The current log list (same reference `useLogs` reads). For non-React
 *  readers and tests; treat as immutable. */
export function getLogs(): readonly LogEntry[] {
  return logs;
}

function subscribe(cb: () => void): () => void {
  subscribers.add(cb);
  return () => {
    subscribers.delete(cb);
  };
}

// Stable reference between mutations — required by useSyncExternalStore to
// avoid an infinite render loop (we only ever replace `logs` on a real change).
function getSnapshot(): LogEntry[] {
  return logs;
}

/** React hook: the current log list, re-rendering on every push/clear. */
export function useLogs(): LogEntry[] {
  return useSyncExternalStore(subscribe, getSnapshot);
}

/** Wire app-lifetime slice-event → console routing. Call once from an
 *  always-mounted component (App). Sinks slice failures as errors and
 *  libslic3r's non-fatal validation warnings as warnings. Returns an
 *  unlisten fn that tears down both listeners. */
export async function setupLogSinks(): Promise<UnlistenFn> {
  const offFail = onEvents<SliceEvent>(["slice:job_failed"], (e) => {
    if (e.payload.kind === "JobFailed") {
      pushLog("error", `Slice failed: ${sliceErrorMessage(e.payload.data.error)}`);
    }
  });
  const offWarn = onEvents<PlateWarningEvent>(["slice:plate_warning"], (e) => {
    pushLog("warn", e.payload.data.message);
  });
  return () => {
    offFail();
    offWarn();
  };
}
