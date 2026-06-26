// hover-tooltip helpers.
//
// featureLabel is the only pure piece worth pinning — the DOM
// render path (positioning, edge flip) needs jsdom + a fake
// viewport which is more setup than the math is worth here.
// Edge-flip is exercised via Playwright.

import { describe, expect, it } from "vitest";

import { featureLabel } from "../HoverTooltip";

describe("featureLabel", () => {
  it("renders canonical names for the common feature types", () => {
    expect(featureLabel("ExternalPerimeter")).toBe("External perimeter");
    expect(featureLabel("Infill")).toBe("Internal infill");
    expect(featureLabel("SolidInfill")).toBe("Solid infill");
    expect(featureLabel("Travel")).toBe("Travel");
  });

  it("surfaces Other(string) verbatim for forward compat", () => {
    expect(featureLabel({ Other: "Wipe tower" })).toBe("Wipe tower");
  });
});
