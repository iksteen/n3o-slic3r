// Pure tests for the per-instance query family — the key is per-id and the
// invalidation is payload-filtered so one printer's `printer:instance_changed`
// doesn't refetch every other printer's cached instance. (The hook lifecycle
// needs a DOM we don't set up in vitest; the testable logic is the factory.)

import { describe, expect, it } from "vitest";
import type { Event as TauriEvent } from "@tauri-apps/api/event";
import { printerInstanceQuery } from "../usePrinterInstance";

const evt = (payload: unknown): TauriEvent<unknown> =>
  ({ event: "printer:instance_changed", id: 0, payload }) as TauriEvent<unknown>;

describe("printerInstanceQuery", () => {
  it("keys the cache entry per instance id", () => {
    expect(printerInstanceQuery("abc").key).toBe("printer_instance:abc");
    expect(printerInstanceQuery("def").key).toBe("printer_instance:def");
  });

  it("refetches only when the event payload names this id", () => {
    const q = printerInstanceQuery("abc");
    expect(q.shouldInvalidate?.(evt("abc"))).toBe(true);
    expect(q.shouldInvalidate?.(evt("other"))).toBe(false);
    expect(q.shouldInvalidate?.(evt(null))).toBe(false);
  });

  it("invalidates on printer:instance_changed", () => {
    expect(printerInstanceQuery("abc").invalidateOn).toContain(
      "printer:instance_changed",
    );
  });
});
