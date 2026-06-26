// Winning-layer derivation tests.

import { describe, expect, it } from "vitest";
import {
  LAYER_HUE,
  isAuthoredTier,
  winningLayerFor,
} from "../layers";

describe("winningLayerFor", () => {
  it("returns 'object' when both project and object override the same key (object beats project)", () => {
    expect(
      winningLayerFor("layer_height", { layer_height: "0.1" }, { layer_height: "0.2" }),
    ).toBe("object");
  });

  it("returns 'project' when only the project tier overrides", () => {
    expect(
      winningLayerFor("layer_height", { layer_height: "0.1" }, {}),
    ).toBe("project");
  });

  it("returns 'cascade' when neither tier overrides", () => {
    expect(winningLayerFor("layer_height", {}, {})).toBe("cascade");
  });
});

describe("isAuthoredTier", () => {
  it("project / object / user are authored tiers", () => {
    expect(isAuthoredTier("project")).toBe(true);
    expect(isAuthoredTier("object")).toBe(true);
    expect(isAuthoredTier("user")).toBe(true);
  });

  it("cascade-side layers are not authored tiers", () => {
    expect(isAuthoredTier("printer")).toBe(false);
    expect(isAuthoredTier("filament")).toBe(false);
    expect(isAuthoredTier("build_plate")).toBe(false);
    expect(isAuthoredTier("default")).toBe(false);
    expect(isAuthoredTier("cascade")).toBe(false);
  });
});

describe("LAYER_HUE palette matches docs/dev/design/data.jsx", () => {
  it("default hue = 220 (neutral blue-grey)", () => {
    expect(LAYER_HUE.default).toBe(220);
  });
  it("printer hue = 18 (warm orange)", () => {
    expect(LAYER_HUE.printer).toBe(18);
  });
  it("build_plate hue = 95 (green)", () => {
    expect(LAYER_HUE.build_plate).toBe(95);
  });
  it("filament hue = 175 (teal)", () => {
    expect(LAYER_HUE.filament).toBe(175);
  });
  it("user hue = 235 (cool blue)", () => {
    expect(LAYER_HUE.user).toBe(235);
  });
  it("project hue = 285 (purple)", () => {
    expect(LAYER_HUE.project).toBe(285);
  });
  it("object hue = 340 (rose)", () => {
    expect(LAYER_HUE.object).toBe(340);
  });
});
