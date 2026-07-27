// Session-scoped UI state: useState that survives component unmount/remount.
// Top-level tab switches conditionally render whole views away (App.tsx's
// mode ternary), losing any component-local state; this parks the value in a
// module-level map instead. In-memory by design — it's "survive a tab
// switch" state, not a persisted preference; a restart starts fresh.

import { useCallback, useState, type SetStateAction } from "react";

const store = new Map<string, unknown>();

export function readSession<T>(key: string, initial: T): T {
  return store.has(key) ? (store.get(key) as T) : initial;
}

export function writeSession<T>(key: string, value: T): void {
  store.set(key, value);
}

export function useSessionState<T>(
  key: string,
  initial: T,
): [T, (next: SetStateAction<T>) => void] {
  // The key is part of the state so a key switch (e.g. per-printer keys on
  // a component that stays mounted across printer selection) re-reads the
  // store instead of carrying the previous key's value.
  const [state, setState] = useState(() => ({
    key,
    value: readSession(key, initial),
  }));
  if (state.key !== key) {
    setState({ key, value: readSession(key, initial) });
  }
  const set = useCallback((next: SetStateAction<T>) => {
    setState((prev) => {
      const value =
        typeof next === "function" ? (next as (p: T) => T)(prev.value) : next;
      writeSession(prev.key, value);
      return { key: prev.key, value };
    });
  }, []);
  return [state.value, set];
}
