// PR-6-8 layer-window math.
//
// The full mount/dispose lifecycle requires WebGL, which vitest's
// jsdom doesn't provide. Cover the pure-TS pieces we can — the
// layer-window → shader-uniform conversion — and leave the
// GPU smoke for PR-6-16's Playwright suite.

import { describe, expect, it } from "vitest";

import { layerWindowBounds } from "../previewScene";

describe("layerWindowBounds", () => {
  it("single mode collapses to identical min + max", () => {
    expect(layerWindowBounds({ mode: "single", layer: 42 })).toEqual({
      min: 42,
      max: 42,
    });
  });

  it("up-to mode starts at layer 0", () => {
    expect(layerWindowBounds({ mode: "up-to", max: 100 })).toEqual({
      min: 0,
      max: 100,
    });
  });

  it("range mode passes both bounds through", () => {
    expect(layerWindowBounds({ mode: "range", min: 10, max: 50 })).toEqual({
      min: 10,
      max: 50,
    });
  });
});
