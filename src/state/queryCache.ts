// Tiny event-invalidated query cache (state-layer spike).
//
// The shape of every mirror hook in this app is: invoke a backend command for
// a value, then refetch it when one of a set of Tauri events fires. That's a
// `fetch + cache + invalidate` cache — the same job React Query does, adapted
// to our event-driven invalidation instead of time-based staleness.
//
// A query is `{ key, fetch, invalidateOn }`. The FIRST component to use a key
// creates one shared cache entry: it registers a single router subscription
// for `invalidateOn` and kicks the initial fetch. Every other component using
// the same key reads the same entry — so two hooks backed by `scene_snapshot`
// share one fetch and one invalidation, instead of each running their own.
//
// Selector subscriptions are implemented (`useQuerySelector`): a consumer that
// only needs a projected slice re-renders only when that slice changes, not on
// every invalidation of the underlying query.
//
// What this still leaves out: ref-counted teardown / GC of idle cache entries.
// Here entries live for the app's lifetime — fine for our handful of
// app-global queries.

import { useCallback, useRef, useSyncExternalStore } from "react";
import type { Event as TauriEvent } from "@tauri-apps/api/event";
import { onEvents } from "./eventRouter";

export interface QueryState<T> {
  /** Last successful value, or `null` before the first load. Retained across
   *  a failed refetch (stale-but-valid beats blank). */
  data: T | null;
  /** True until the first fetch settles (success or failure). */
  loading: boolean;
  /** Set only when the INITIAL load fails (no data yet) — a fatal,
   *  surface-to-the-user condition. A later refetch failure keeps the stale
   *  data and only logs, mirroring the hand-written hooks' policy. */
  error: string | null;
}

export interface QueryDef<T> {
  key: string;
  fetch: () => Promise<T>;
  invalidateOn: readonly string[];
  /** Optional payload filter for parameterized ("family") queries: when set,
   *  an `invalidateOn` event only triggers a refetch if this returns true.
   *  E.g. a per-instance query refetches on `printer:instance_changed` only
   *  when `event.payload === id`, so one printer's change doesn't refetch
   *  every other printer's cached entry. */
  shouldInvalidate?: (event: TauriEvent<unknown>) => boolean;
}

interface QueryEntry<T> {
  def: QueryDef<T>;
  state: QueryState<T>;
  subscribers: Set<() => void>;
  inFlight: Promise<void> | null;
  /** An invalidation arrived while a fetch was in flight — coalesce into one
   *  trailing refetch so the final state reflects the latest event. */
  requeued: boolean;
}

const registry = new Map<string, QueryEntry<unknown>>();

/** Identity helper — gives a query def its type and reads as a declaration. */
export function defineQuery<T>(def: QueryDef<T>): QueryDef<T> {
  return def;
}

function getEntry<T>(def: QueryDef<T>): QueryEntry<T> {
  const existing = registry.get(def.key);
  if (existing) return existing as QueryEntry<T>;

  const entry: QueryEntry<T> = {
    def,
    state: { data: null, loading: true, error: null },
    subscribers: new Set(),
    inFlight: null,
    requeued: false,
  };
  registry.set(def.key, entry as QueryEntry<unknown>);
  // One shared invalidation subscription for the lifetime of the entry, and
  // the initial fetch. Both happen exactly once per key.
  onEvents(def.invalidateOn, (event) => {
    if (!def.shouldInvalidate || def.shouldInvalidate(event)) {
      invalidateQuery(def.key);
    }
  });
  void runFetch(entry);
  return entry;
}

function patch<T>(entry: QueryEntry<T>, next: Partial<QueryState<T>>): void {
  entry.state = { ...entry.state, ...next };
  for (const cb of entry.subscribers) cb();
}

async function runFetch<T>(entry: QueryEntry<T>): Promise<void> {
  // Dedup concurrent fetches; remember that another was requested so we run
  // one more pass after this settles (don't drop the latest invalidation).
  if (entry.inFlight) {
    entry.requeued = true;
    return entry.inFlight;
  }

  const run = (async () => {
    try {
      const data = await entry.def.fetch();
      patch(entry, { data, loading: false, error: null });
    } catch (err) {
      if (entry.state.data == null) {
        // Initial load failed — fatal.
        patch(entry, { loading: false, error: String(err) });
        console.error(`[query:${entry.def.key}] initial fetch failed`, err);
      } else {
        // Refetch failed — keep the stale value, just log.
        patch(entry, { loading: false });
        console.error(`[query:${entry.def.key}] refetch failed (kept stale)`, err);
      }
    } finally {
      entry.inFlight = null;
    }
  })();
  entry.inFlight = run;
  await run;
  if (entry.requeued) {
    entry.requeued = false;
    void runFetch(entry);
  }
}

/** Force a refetch of a query if it's been instantiated. The router calls this
 *  on every `invalidateOn` event; also exported for imperative invalidation
 *  after a mutation whose event isn't modeled yet, and for tests. No-op for a
 *  key nobody has mounted (nothing to keep fresh). */
export function invalidateQuery(key: string): void {
  const entry = registry.get(key);
  if (entry) void runFetch(entry);
}

/** Stable "no query" state for a disabled (`null` def) `useQuery` — lets a
 *  parameterized consumer call the hook unconditionally when its argument
 *  isn't available yet (e.g. no instance bound). */
const DISABLED: QueryState<never> = { data: null, loading: false, error: null };

/** React hook: subscribe to a query's state. The first caller for a key
 *  triggers its fetch + invalidation wiring; all callers share one entry.
 *  Pass `null`/`undefined` to disable (returns a constant empty state without
 *  touching the cache) — for parameterized queries whose argument may be
 *  absent. */
export function useQuery<T>(def: QueryDef<T> | null | undefined): QueryState<T> {
  const entry = def ? getEntry(def) : null;
  const subscribe = useCallback(
    (cb: () => void) => {
      if (!entry) return () => {};
      entry.subscribers.add(cb);
      return () => {
        entry.subscribers.delete(cb);
      };
    },
    [entry],
  );
  return useSyncExternalStore(subscribe, () =>
    entry ? entry.state : (DISABLED as QueryState<T>),
  );
}

/** React hook: subscribe to a DERIVED slice of a query, re-rendering only when
 *  that slice changes per `isEqual` (default `Object.is`). This is what makes a
 *  shared query safe for a projection-only consumer: the query refetches on its
 *  full (superset) invalidation set, but a consumer reading just a projection
 *  doesn't re-render on events that don't touch its slice.
 *
 *  The memo is per-hook-instance: it caches the last (raw state → selected)
 *  pair, returns the prior selection's reference when the new one is `isEqual`
 *  (so `useSyncExternalStore` bails out of the render), and is `O(1)` on
 *  renders that don't change the raw state. */
export function useQuerySelector<T, S>(
  def: QueryDef<T>,
  selector: (state: QueryState<T>) => S,
  isEqual: (a: S, b: S) => boolean = Object.is,
): S {
  const entry = getEntry(def);
  const memo = useRef<SelectMemo<T, S> | null>(null);

  const subscribe = useCallback(
    (cb: () => void) => {
      entry.subscribers.add(cb);
      return () => {
        entry.subscribers.delete(cb);
      };
    },
    [entry],
  );

  const getSelection = (): S => {
    memo.current = selectMemo(memo.current, entry.state, selector, isEqual);
    return memo.current.output;
  };

  return useSyncExternalStore(subscribe, getSelection);
}

export interface SelectMemo<T, S> {
  input: QueryState<T>;
  output: S;
}

/** Pure memo step for `useQuerySelector` (exported for tests). Given the prior
 *  `(input → output)` memo, the new raw query state, a selector, and an
 *  equality: return the memo to keep. Reuses the prior reference whenever it
 *  can so `useSyncExternalStore` can bail out of a re-render —
 *   - raw state reference unchanged → return `prev` verbatim;
 *   - new raw state but `isEqual` selection → keep `prev.output`'s reference,
 *     advance `input` (don't recompute next render);
 *   - otherwise → a fresh `(input, output)`. */
export function selectMemo<T, S>(
  prev: SelectMemo<T, S> | null,
  input: QueryState<T>,
  selector: (state: QueryState<T>) => S,
  isEqual: (a: S, b: S) => boolean,
): SelectMemo<T, S> {
  if (prev && prev.input === input) return prev;
  const output = selector(input);
  if (prev && isEqual(prev.output, output)) return { input, output: prev.output };
  return { input, output };
}

/** Instantiate a query (register + initial fetch) without React. For tests. */
export function primeQueryForTests<T>(def: QueryDef<T>): void {
  getEntry(def);
}

/** Read a query's current cached state without subscribing. For tests. */
export function peekQueryForTests<T>(key: string): QueryState<T> | null {
  return (registry.get(key)?.state as QueryState<T>) ?? null;
}

/** Clear the whole registry. For test isolation. */
export function resetQueryCacheForTests(): void {
  registry.clear();
}
