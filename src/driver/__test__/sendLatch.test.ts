// The module-scoped send latch: armed after a send, it must release on
// the FIRST job-state transition away from the send-time token — even a
// transition that lands back on the same token later (idle → printing →
// cancelled → idle) must release at the first step, because the release
// listener runs at module scope on the app-wide status stream, not
// inside the (possibly unmounted) SendControls component. This is the
// regression test for the "cancelled from Devices, Send stuck on
// 'Waiting for the printer…'" bug.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { EventHandler } from "../../state/eventRouter";

// Capture the router registration instead of touching Tauri.
const routerHandlers = new Set<EventHandler<unknown>>();
vi.mock("../../state/eventRouter", () => ({
  onEvents: vi.fn((_names: readonly string[], handler: EventHandler<unknown>) => {
    routerHandlers.add(handler);
    return () => routerHandlers.delete(handler);
  }),
}));
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn().mockResolvedValue(undefined),
}));

import {
  armPendingSend,
  releasePendingSend,
  pendingSendDriverForTests,
} from "../SendControls";

function emitStatus(driverId: number, jobState: string | null, connected = true): void {
  const status = {
    connection: { state: connected ? "Connected" : "Disconnected" },
    job: jobState == null ? null : { state: { state: jobState } },
  };
  for (const h of [...routerHandlers]) {
    h({
      event: "driver:status_update",
      id: 1,
      payload: { driver_id: driverId, status },
    } as never);
  }
}

describe("send latch", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    releasePendingSend();
    routerHandlers.clear();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("releases on the first job-state change, even mid-cycle back to idle", () => {
    armPendingSend(7, "idle");
    expect(pendingSendDriverForTests()).toBe(7);

    // Same token → still latched (printer hasn't acted yet).
    emitStatus(7, null);
    expect(pendingSendDriverForTests()).toBe(7);

    // Printer picks the job up → released immediately. A later return
    // to idle (cancel) can't wedge us: we already let go.
    emitStatus(7, "Printing");
    expect(pendingSendDriverForTests()).toBeNull();
    emitStatus(7, null);
    expect(pendingSendDriverForTests()).toBeNull();
  });

  it("ignores other drivers' updates", () => {
    armPendingSend(7, "idle");
    emitStatus(8, "Printing");
    expect(pendingSendDriverForTests()).toBe(7);
  });

  it("releases when the link drops", () => {
    armPendingSend(7, "idle");
    emitStatus(7, null, false);
    expect(pendingSendDriverForTests()).toBeNull();
  });

  it("releases via the 60s backstop if nothing ever changes", () => {
    armPendingSend(7, "idle");
    vi.advanceTimersByTime(59000);
    expect(pendingSendDriverForTests()).toBe(7);
    vi.advanceTimersByTime(2000);
    expect(pendingSendDriverForTests()).toBeNull();
  });

  it("re-arming replaces the previous latch and its listener", () => {
    armPendingSend(7, "idle");
    armPendingSend(9, "Finished");
    expect(pendingSendDriverForTests()).toBe(9);
    // The stale driver-7 listener must be gone: only driver 9 releases.
    emitStatus(7, "Printing");
    expect(pendingSendDriverForTests()).toBe(9);
    emitStatus(9, "Printing");
    expect(pendingSendDriverForTests()).toBeNull();
  });
});
