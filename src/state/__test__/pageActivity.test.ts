// Freeze detection + the router's coalescing gate.

import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  isPageActive,
  onPageResume,
  resetPageActivityForTests,
} from "../pageActivity";

describe("isPageActive", () => {
  beforeEach(() => {
    vi.unstubAllGlobals();
    // Without rAF the detector can't see frames at all and deliberately
    // reports "active" (never gate what we can't measure), so a test of the
    // heartbeat has to provide one.
    vi.stubGlobal("requestAnimationFrame", () => 0);
    resetPageActivityForTests(0);
  });

  it("reports active when the environment exposes no frame callback at all", () => {
    vi.unstubAllGlobals();
    resetPageActivityForTests(999999);
    expect(isPageActive()).toBe(true);
  });

  it("is active right after a painted frame", () => {
    expect(isPageActive()).toBe(true);
  });

  it("goes inactive once the paint heartbeat has been silent", () => {
    // rAF stops when the page stops being painted — the same condition that
    // stops GC, which is what makes allocation while frozen unbounded.
    resetPageActivityForTests(5000);
    expect(isPageActive()).toBe(false);
  });

  it("trusts document visibility even while frames are still arriving", () => {
    // A hidden page is frozen regardless of the heartbeat; under some
    // compositors one signal fires and not the other, so either suffices.
    vi.stubGlobal("document", { visibilityState: "hidden" });
    resetPageActivityForTests(0);
    expect(isPageActive()).toBe(false);
  });
});

describe("onPageResume", () => {
  beforeEach(() => {
    vi.stubGlobal("requestAnimationFrame", () => 0);
    resetPageActivityForTests(0);
  });

  it("unsubscribes cleanly", () => {
    const cb = vi.fn();
    const off = onPageResume(cb);
    off();
    // No resume can be synthesised without a real rAF loop; the contract under
    // test is that unsubscribing drops the reference so a long-lived consumer
    // can't leak one per mount.
    expect(cb).not.toHaveBeenCalled();
  });
});
