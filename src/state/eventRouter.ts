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

/** A router handler. Receives the raw Tauri event (name + payload). `T` is the
 *  payload type when a name-group is homogeneous (e.g. all `SliceEvent`);
 *  defaults to `unknown`. */
export type EventHandler<T = unknown> = (event: TauriEvent<T>) => void;

const handlers = new Map<string, Set<EventHandler>>();
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
    const set = handlers.get(name);
    if (!set) return;
    // Snapshot before iterating: a handler may unsubscribe mid-dispatch.
    for (const h of [...set]) h(event);
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

/** Like `onEvents`, but resolves only once every name's underlying Tauri
 *  `listen` is established — so a consumer that must not miss events between
 *  subscribing and an initial fetch can `await` this before fetching. */
export async function onEventsReady<T = unknown>(
  names: readonly string[],
  handler: EventHandler<T>,
): Promise<() => void> {
  const off = onEvents(names, handler);
  await Promise.all(names.map((name) => ensureListening(name)));
  return off;
}
