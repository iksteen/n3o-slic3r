// Mock of @tauri-apps/api/event for the browser demo. An in-page event bus:
// `listen` registers, `emit` (demo-only) can synthesize backend events.

export type UnlistenFn = () => void;
export interface Event<T> {
  event: string;
  id: number;
  payload: T;
}

type Handler = (e: Event<unknown>) => void;
const listeners = new Map<string, Set<Handler>>();

export async function listen<T>(
  event: string,
  handler: (e: Event<T>) => void,
): Promise<UnlistenFn> {
  let set = listeners.get(event);
  if (!set) {
    set = new Set();
    listeners.set(event, set);
  }
  set.add(handler as Handler);
  return () => set!.delete(handler as Handler);
}

/** Demo-only: fire a synthesized backend event to any registered listeners. */
export function emit(event: string, payload: unknown): void {
  const set = listeners.get(event);
  if (set) for (const h of set) h({ event, id: 0, payload });
}
