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

/** A router handler. Receives the raw Tauri event (name + payload) so a
 *  handler that needs `data.plate_id` can read it. */
export type EventHandler = (event: TauriEvent<unknown>) => void;

const handlers = new Map<string, Set<EventHandler>>();
/** Names we've already opened a (single) Tauri `listen` for. */
const listening = new Set<string>();

function ensureListening(name: string): void {
  if (listening.has(name)) return;
  listening.add(name);
  // Fire-and-forget: the subscription lives for the app's lifetime. An event
  // arriving in the tiny window before this promise resolves is lost — the
  // same race the per-hook subscribe-before-fetch pattern already tolerates,
  // and queries do an initial fetch regardless.
  void listen(name, (event) => {
    const set = handlers.get(name);
    if (!set) return;
    // Snapshot before iterating: a handler may unsubscribe mid-dispatch.
    for (const h of [...set]) h(event);
  });
}

/** Register `handler` for every name in `names`. Returns an unsubscribe that
 *  detaches it from all of them. Multiple calls for the same name share one
 *  underlying Tauri subscription. */
export function onEvents(
  names: readonly string[],
  handler: EventHandler,
): () => void {
  for (const name of names) {
    let set = handlers.get(name);
    if (!set) handlers.set(name, (set = new Set()));
    set.add(handler);
    ensureListening(name);
  }
  return () => {
    for (const name of names) handlers.get(name)?.delete(handler);
  };
}
