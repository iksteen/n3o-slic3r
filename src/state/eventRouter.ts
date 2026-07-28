// Central Tauri-event router (state-layer spike).
//
// Today every mirror hook (useProjectSession, usePlateTabs, usePlugins,
// useDriverStatus, …) opens its OWN `listen()` for each event name it cares
// about. The sets overlap heavily — a single `scene:object_added` fans out
// into a dozen independent subscriptions, each triggering its own refetch.
//
// This router collapses that to ONE Tauri `listen()` per distinct event name,
// shared across all subscribers. Hooks register a handler for a set of names
// via `onEvents`; the router multiplexes. The per-name Tauri subscription is
// created lazily on first interest and kept for the app's lifetime (the event
// names are a small fixed vocabulary and the streams are app-global — the same
// rationale as `setupLogSinks`). Only the per-handler registration is
// ref-counted and torn down, so components don't leak handlers on unmount.
//
// Spike scope: this is deliberately app-lifetime simple. A production version
// would ref-count the Tauri subscription itself and tear it down when the last
// handler for a name unsubscribes.

import { listen, type Event as TauriEvent } from "@tauri-apps/api/event";
import { isPageActive, onPageResume } from "./pageActivity";

/** A router handler. Receives the raw Tauri event (name + payload). `T` is the
 *  payload type when a name-group is homogeneous (e.g. all `SliceEvent`);
 *  defaults to `unknown`. */
export type EventHandler<T = unknown> = (event: TauriEvent<T>) => void;

const handlers = new Map<string, Set<EventHandler>>();

// ── Freeze gate ──────────────────────────────────────────────────
// While the page isn't painted, WebKit stops collecting garbage but the backend
// keeps emitting. Dispatching a stream of "here's the current temperature"
// events into React in that state allocates render garbage that can never be
// reclaimed until the page comes back — the mechanism behind the 16 GB kills.
//
// So while frozen, events whose *only* value is their latest payload are held
// instead of dispatched, one per key, and replayed on resume. Everything else
// dispatches normally: an event that carries a transition (a slice finished, an
// object was added) is not something we may drop, and those don't arrive at
// telemetry rates anyway.
//
// The key extractor keeps one pending event per logical subject — per driver
// here, so a quiet printer's last status isn't clobbered by a busy one's.
const COALESCE_WHILE_FROZEN: Record<string, (payload: unknown) => string> = {
  "driver:status_update": (p) => String((p as { driver_id?: number })?.driver_id ?? ""),
  "driver:upload_progress": (p) => String((p as { driver_id?: number })?.driver_id ?? ""),
};

/** Held events, keyed `name|subject`. At most one per subject, so memory is
 *  bounded by the number of drivers no matter how long the freeze lasts. */
const frozenPending = new Map<string, { name: string; event: TauriEvent<unknown> }>();

function dispatch(name: string, event: TauriEvent<unknown>): void {
  const set = handlers.get(name);
  if (!set) return;
  // Snapshot before iterating: a handler may unsubscribe mid-dispatch.
  for (const h of [...set]) h(event);
}

onPageResume(() => {
  if (frozenPending.size === 0) return;
  const held = [...frozenPending.values()];
  frozenPending.clear();
  for (const { name, event } of held) dispatch(name, event);
});
/** Per-name `listen()` promise — one shared subscription per name, created on
 *  first interest. Awaited by `onEventsReady` so order-sensitive consumers
 *  (the scene-mirror bridge: subscribe before the initial snapshot) don't miss
 *  an event in the listen-resolution window. */
const listenPromises = new Map<string, Promise<void>>();

function ensureListening(name: string): Promise<void> {
  const existing = listenPromises.get(name);
  if (existing) return existing;
  // The subscription lives for the app's lifetime (event names are a small
  // fixed vocabulary; the streams are app-global), so we discard the
  // UnlistenFn and never tear the Tauri listener down — only per-handler
  // registration is ref-counted.
  const p = listen(name, (event) => {
    const coalesceKey = COALESCE_WHILE_FROZEN[name];
    if (coalesceKey && !isPageActive()) {
      frozenPending.set(`${name}|${coalesceKey(event.payload)}`, { name, event });
      return;
    }
    dispatch(name, event);
  }).then(() => {});
  listenPromises.set(name, p);
  return p;
}

/** Register `handler` for every name in `names`. Returns an unsubscribe that
 *  detaches it from all of them. Multiple calls for the same name share one
 *  underlying Tauri subscription. */
export function onEvents<T = unknown>(
  names: readonly string[],
  handler: EventHandler<T>,
): () => void {
  for (const name of names) {
    let set = handlers.get(name);
    if (!set) handlers.set(name, (set = new Set()));
    set.add(handler as EventHandler);
    void ensureListening(name);
  }
  return () => {
    for (const name of names) handlers.get(name)?.delete(handler as EventHandler);
  };
}
