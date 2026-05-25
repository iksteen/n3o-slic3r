// AddPrinterModal pure-helper tests.
//
// Component rendering / submit lifecycle needs a jsdom + RTL setup
// we don't have (same pattern as `PrinterCredentialsDialog.test.ts`).
// We pin the pure helpers `makeUniqueName` + `amsSlotLabels` so
// regressions in the name-collision logic or the AMS topology
// generator trip loudly.

import { describe, expect, it } from "vitest";
import { amsSlotLabels, makeUniqueName } from "../AddPrinterModal";

describe("makeUniqueName", () => {
  it("returns the base when nothing collides", () => {
    expect(makeUniqueName("Bambu A1 mini", [])).toBe("Bambu A1 mini");
    expect(makeUniqueName("Bambu A1 mini", ["Other"])).toBe("Bambu A1 mini");
  });

  it("appends ` (2)` on a single collision", () => {
    expect(
      makeUniqueName("Bambu A1 mini", ["Bambu A1 mini"]),
    ).toBe("Bambu A1 mini (2)");
  });

  it("walks the counter past taken slots", () => {
    expect(
      makeUniqueName("A1", ["A1", "A1 (2)", "A1 (3)"]),
    ).toBe("A1 (4)");
  });

  it("returns empty for empty input regardless of collisions", () => {
    expect(makeUniqueName("", [])).toBe("");
    expect(makeUniqueName("", ["anything"])).toBe("");
  });
});

describe("amsSlotLabels", () => {
  it("returns just `Ext` for 0 AMS units", () => {
    expect(amsSlotLabels(0)).toEqual(["Ext"]);
  });

  it("returns `Ext` + 4 unlettered AMS slots for 1 AMS unit", () => {
    expect(amsSlotLabels(1)).toEqual([
      "Ext",
      "AMS:1",
      "AMS:2",
      "AMS:3",
      "AMS:4",
    ]);
  });

  it("disambiguates with letters when multiple AMS units are installed", () => {
    expect(amsSlotLabels(3)).toEqual([
      "Ext",
      "AMS A:1",
      "AMS A:2",
      "AMS A:3",
      "AMS A:4",
      "AMS B:1",
      "AMS B:2",
      "AMS B:3",
      "AMS B:4",
      "AMS C:1",
      "AMS C:2",
      "AMS C:3",
      "AMS C:4",
    ]);
  });

  it("produces N*4 + 1 slots regardless of N", () => {
    for (let n = 0; n <= 4; n++) {
      expect(amsSlotLabels(n).length).toBe(n * 4 + 1);
    }
  });
});
