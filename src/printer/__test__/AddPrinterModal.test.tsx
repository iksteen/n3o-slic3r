// AddPrinterModal pure-helper tests.
//
// Component rendering / submit lifecycle needs a jsdom + RTL setup
// we don't have (same pattern as `PrinterCredentialsDialog.test.ts`).
// We pin `makeUniqueName` so name-collision logic regressions trip
// loudly. The AMS labelling convention lives in
// `printerInstance.ts::deriveSlotLabel` and is covered by
// `printerInstance.test.ts::flattenSlots`.

import { describe, expect, it } from "vitest";
import { makeUniqueName } from "../AddPrinterModal";

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

