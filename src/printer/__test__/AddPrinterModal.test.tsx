// AddPrinterModal pure-helper tests.
//
// Component rendering / submit lifecycle needs a jsdom + RTL setup
// we don't have (same pattern as `PrinterCredentialsDialog.test.ts`).
// We pin `makeUniqueName` so name-collision logic regressions trip
// loudly. The AMS labelling convention lives in
// `printerInstance.ts::deriveSlotLabel` and is covered by
// `printerInstance.test.ts::flattenSlots`.

import { describe, expect, it } from "vitest";
import { makeUniqueName, matchesPickerFilter } from "../AddPrinterModal";
import type { PrinterCatalogEntry } from "../printerCommands";

const entry = (
  model: string,
  opts: { brand?: string; experimental?: boolean } = {},
): PrinterCatalogEntry =>
  ({
    identity: model.toLowerCase().replace(/\s+/g, "-"),
    experimental: opts.experimental,
    profile: {
      model,
      brand: opts.brand ?? "Bambu Lab",
    },
  }) as PrinterCatalogEntry;

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

describe("matchesPickerFilter", () => {
  const a1 = entry("A1", { experimental: true });
  const mini = entry("A1 mini");

  it("hides experimental profiles unless the toggle is on", () => {
    expect(matchesPickerFilter(a1, "", false)).toBe(false);
    expect(matchesPickerFilter(a1, "", true)).toBe(true);
  });

  it("always shows non-experimental profiles", () => {
    expect(matchesPickerFilter(mini, "", false)).toBe(true);
  });

  it("applies the case-insensitive brand+model query", () => {
    expect(matchesPickerFilter(mini, "bambu", false)).toBe(true);
    expect(matchesPickerFilter(mini, "MINI", false)).toBe(true);
    expect(matchesPickerFilter(mini, "snapmaker", false)).toBe(false);
  });

  it("gates experimental first, then the query", () => {
    // Matches the query, but still hidden while the toggle is off.
    expect(matchesPickerFilter(a1, "a1", false)).toBe(false);
    expect(matchesPickerFilter(a1, "a1", true)).toBe(true);
    expect(matchesPickerFilter(a1, "zzz", true)).toBe(false);
  });
});

