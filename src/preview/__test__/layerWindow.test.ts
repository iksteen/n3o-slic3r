// PR-6-9 layerWindow transitions.
//
// The LayerSlider component is presentation; the interesting
// behavior is the mode-switch + step + jump math. Tests pin
// those transitions so a future refactor doesn't drift the
// preserve-visible-layer invariant the UX depends on.

import { describe, expect, it } from "vitest";

import {
  defaultWindow,
  jumpTo,
  stepLayer,
  switchMode,
  windowBounds,
} from "../layerWindow";

describe("windowBounds", () => {
  it("single collapses to one layer", () => {
    expect(windowBounds({ mode: "single", layer: 12 })).toEqual([12, 12]);
  });
  it("up-to starts at zero", () => {
    expect(windowBounds({ mode: "up-to", max: 40 })).toEqual([0, 40]);
  });
  it("range passes its own bounds", () => {
    expect(windowBounds({ mode: "range", min: 10, max: 30 })).toEqual([10, 30]);
  });
});

describe("switchMode", () => {
  it("single → up-to preserves the current layer as max", () => {
    expect(switchMode({ mode: "single", layer: 42 }, "up-to")).toEqual({
      mode: "up-to",
      max: 42,
    });
  });

  it("up-to → single keeps the same visible top", () => {
    expect(switchMode({ mode: "up-to", max: 30 }, "single")).toEqual({
      mode: "single",
      layer: 30,
    });
  });

  it("single → range expands to (0..layer)", () => {
    expect(switchMode({ mode: "single", layer: 50 }, "range")).toEqual({
      mode: "range",
      min: 0,
      max: 50,
    });
  });

  it("range → single collapses to max", () => {
    expect(
      switchMode({ mode: "range", min: 20, max: 80 }, "single"),
    ).toEqual({
      mode: "single",
      layer: 80,
    });
  });

  it("range → up-to preserves the upper bound", () => {
    expect(
      switchMode({ mode: "range", min: 20, max: 80 }, "up-to"),
    ).toEqual({
      mode: "up-to",
      max: 80,
    });
  });
});

describe("stepLayer", () => {
  it("steps single mode layer", () => {
    expect(
      stepLayer({ mode: "single", layer: 10 }, 1, 100),
    ).toEqual({ mode: "single", layer: 11 });
  });

  it("clamps to last layer", () => {
    expect(
      stepLayer({ mode: "single", layer: 99 }, 5, 100),
    ).toEqual({ mode: "single", layer: 99 });
  });

  it("clamps to 0", () => {
    expect(stepLayer({ mode: "single", layer: 0 }, -5, 100)).toEqual({
      mode: "single",
      layer: 0,
    });
  });

  it("range steps the top thumb without crossing min", () => {
    expect(
      stepLayer({ mode: "range", min: 20, max: 30 }, -15, 100),
    ).toEqual({ mode: "range", min: 20, max: 20 });
  });

  it("no-op when layerCount is 0", () => {
    const before = { mode: "single" as const, layer: 0 };
    expect(stepLayer(before, 5, 0)).toBe(before);
  });
});

describe("jumpTo", () => {
  it("first → 0 in single mode", () => {
    expect(jumpTo({ mode: "single", layer: 50 }, "first", 100)).toEqual({
      mode: "single",
      layer: 0,
    });
  });

  it("last → layerCount-1 in up-to mode", () => {
    expect(jumpTo({ mode: "up-to", max: 0 }, "last", 100)).toEqual({
      mode: "up-to",
      max: 99,
    });
  });

  it("range first resets both bounds", () => {
    expect(
      jumpTo({ mode: "range", min: 30, max: 70 }, "first", 100),
    ).toEqual({ mode: "range", min: 0, max: 0 });
  });
});

describe("defaultWindow", () => {
  it("opens at up-to with max = top layer (show the whole print)", () => {
    // The shader's depth-fade keeps up-to mode legible on tall
    // prints — the top ~25 layers stay opaque, older ones fade
    // toward the background — so we can safely default to it.
    expect(defaultWindow(235)).toEqual({ mode: "up-to", max: 234 });
  });

  it("doesn't underflow for empty preview", () => {
    expect(defaultWindow(0)).toEqual({ mode: "up-to", max: 0 });
  });
});
