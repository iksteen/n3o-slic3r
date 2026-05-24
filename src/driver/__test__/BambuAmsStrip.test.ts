// PR-7a-7 BambuAmsStrip projection tests.
//
// Component-render lifecycle (jsdom + React) isn't set up in this
// repo's vitest config; we test the pure projection helper that
// turns the AMS wire shape into chip descriptors. Visual sanity
// of the rendered DOM is covered by the eventual Playwright smoke.

import { describe, expect, it } from "vitest";
import { chipsFromAms, cssColorFromHex } from "../BambuAmsStrip";
import type { AmsState } from "../types";

describe("cssColorFromHex", () => {
  it("drops alpha when it is fully opaque (FF)", () => {
    expect(cssColorFromHex("ABCDEFFF")).toBe("#ABCDEF");
  });

  it("preserves alpha when it is not fully opaque", () => {
    expect(cssColorFromHex("ABCDEF80")).toBe("#ABCDEF80");
  });

  it("handles 6-digit wire values verbatim (no alpha to strip)", () => {
    // BBL sometimes sends 6-digit values during partial reports.
    expect(cssColorFromHex("123456")).toBe("#123456");
  });
});

describe("chipsFromAms", () => {
  it("produces one chip per tray, in unit-then-tray order", () => {
    const ams: AmsState = {
      active_slot: null,
      units: [
        {
          id: 0,
          trays: [
            { id: 0, identity: { tray_type: "PLA", color: "FF0000FF", sub_brand: null, multi_colors: [] } },
            { id: 1, identity: null },
            { id: 2, identity: { tray_type: "PETG", color: "00FF00FF", sub_brand: null, multi_colors: [] } },
            { id: 3, identity: null },
          ],
        },
      ],
    };
    const chips = chipsFromAms(ams);
    expect(chips).toHaveLength(4);
    expect(chips.map((c) => c.cssColor)).toEqual([
      "#FF0000",
      null,
      "#00FF00",
      null,
    ]);
    expect(chips.map((c) => c.trayType)).toEqual(["PLA", null, "PETG", null]);
  });

  it("marks the active slot via isActive", () => {
    const ams: AmsState = {
      active_slot: 1,
      units: [
        {
          id: 0,
          trays: [
            { id: 0, identity: { tray_type: "PLA", color: "FF0000FF", sub_brand: null, multi_colors: [] } },
            { id: 1, identity: { tray_type: "PLA", color: "00FF00FF", sub_brand: null, multi_colors: [] } },
          ],
        },
      ],
    };
    const chips = chipsFromAms(ams);
    expect(chips[0].isActive).toBe(false);
    expect(chips[1].isActive).toBe(true);
  });

  it("walks multi-unit topologies (X1C-style)", () => {
    const ams: AmsState = {
      active_slot: null,
      units: [
        {
          id: 0,
          trays: [
            { id: 0, identity: { tray_type: "PLA", color: "FF0000FF", sub_brand: null, multi_colors: [] } },
          ],
        },
        {
          id: 1,
          trays: [
            { id: 0, identity: { tray_type: "ABS", color: "0000FFFF", sub_brand: null, multi_colors: [] } },
          ],
        },
      ],
    };
    const chips = chipsFromAms(ams);
    expect(chips).toHaveLength(2);
    expect(chips[0].unitId).toBe(0);
    expect(chips[1].unitId).toBe(1);
  });

  it("produces the hover-label including tray_type + hex color", () => {
    const ams: AmsState = {
      active_slot: null,
      units: [
        {
          id: 0,
          trays: [
            { id: 0, identity: { tray_type: "PLA", color: "FF0000FF", sub_brand: null, multi_colors: [] } },
          ],
        },
      ],
    };
    const chips = chipsFromAms(ams);
    expect(chips[0].cssLabel).toBe("PLA · #FF0000FF");
  });
});
