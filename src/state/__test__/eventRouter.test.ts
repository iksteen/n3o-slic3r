// The router's freeze gate: what may be held while the page isn't painted, and
// what must never be.

import { beforeEach, describe, expect, it, vi } from "vitest";

// Capture the per-name callback `listen` is given so a test can push events in
// as the Tauri side would.
const listeners = new Map<string, (event: { payload: unknown }) => void>();
vi.mock("@tauri-apps/api/event", () => ({
  listen: (name: string, cb: (event: { payload: unknown }) => void) => {
    listeners.set(name, cb);
    return Promise.resolve(() => {});
  },
}));

import { onEvents } from "../eventRouter";
import {
  resetPageActivityForTests,
  triggerPageResumeForTests,
} from "../pageActivity";

/** Push an event as the backend would, after `onEvents` wired the name up. */
async function emit(name: string, payload: unknown): Promise<void> {
  await Promise.resolve(); // let ensureListening's promise settle
  listeners.get(name)?.({ payload });
}

describe("event router freeze gate", () => {
  beforeEach(() => {
    // Deliberately NOT clearing `listeners`: the router subscribes once per
    // event name for the app's lifetime, so a later `onEvents` for a name a
    // previous test already wired up won't call `listen` again. Clearing here
    // made emits go nowhere and every assertion pass vacuously.
    vi.stubGlobal("requestAnimationFrame", () => 0);
    resetPageActivityForTests(0);
  });

  it("delivers telemetry normally while the page is painted", async () => {
    const seen: unknown[] = [];
    onEvents(["driver:status_update"], (e) => seen.push(e.payload));
    await emit("driver:status_update", { driver_id: 1, n: 1 });
    await emit("driver:status_update", { driver_id: 1, n: 2 });
    expect(seen).toHaveLength(2);
  });

  it("holds telemetry while frozen and replays only the latest per driver", async () => {
    const seen: Array<{ driver_id: number; n: number }> = [];
    onEvents<{ driver_id: number; n: number }>(["driver:status_update"], (e) =>
      seen.push(e.payload),
    );
    await emit("driver:status_update", { driver_id: 1, n: 0 }); // wire up while painted
    seen.length = 0;

    resetPageActivityForTests(5000); // frozen: no frames, so no GC either
    for (let n = 1; n <= 50; n++) {
      await emit("driver:status_update", { driver_id: 1, n });
      await emit("driver:status_update", { driver_id: 2, n });
    }
    expect(seen).toEqual([]);

    // On the first painted frame, each driver's newest status lands — one
    // event each, not the 50 that arrived. The UI is current without having
    // paid for the backlog.
    resetPageActivityForTests(0);
    triggerPageResumeForTests();
    expect(seen).toEqual([
      { driver_id: 1, n: 50 },
      { driver_id: 2, n: 50 },
    ]);
  });

  it("never holds a transition — a finished slice must reach its handler", async () => {
    // Coalescing these would lose state outright: the preview loads off
    // plate_finished, so a slice completing while the screen is locked has to
    // dispatch even though nothing is painted.
    const seen: unknown[] = [];
    onEvents(["slice:plate_finished"], (e) => seen.push(e.payload));
    resetPageActivityForTests(5000);
    await emit("slice:plate_finished", { data: { plate_id: 1 } });
    expect(seen).toHaveLength(1);
  });
});
