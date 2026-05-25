import { describe, expect, it } from "vitest";
import { chunkExtruders, nozzlesInline } from "../nozzleLayout";

describe("nozzlesInline", () => {
  it("inline for 1 or 2 extruders, separate rows for 0 or 3+", () => {
    expect(nozzlesInline(0)).toBe(false);
    expect(nozzlesInline(1)).toBe(true);
    expect(nozzlesInline(2)).toBe(true);
    expect(nozzlesInline(3)).toBe(false);
    expect(nozzlesInline(8)).toBe(false);
  });
});

describe("chunkExtruders", () => {
  it("returns no rows when extruders ride inline with Printer + Bed", () => {
    expect(chunkExtruders(0)).toEqual([]);
    expect(chunkExtruders(1)).toEqual([]);
    expect(chunkExtruders(2)).toEqual([]);
  });

  it("packs up to 4 per row, indices in order", () => {
    expect(chunkExtruders(3)).toEqual([[0, 1, 2]]);
    expect(chunkExtruders(4)).toEqual([[0, 1, 2, 3]]);
  });

  it("wraps onto extra rows past 4", () => {
    expect(chunkExtruders(5)).toEqual([[0, 1, 2, 3], [4]]);
    expect(chunkExtruders(8)).toEqual([
      [0, 1, 2, 3],
      [4, 5, 6, 7],
    ]);
    expect(chunkExtruders(9)).toEqual([
      [0, 1, 2, 3],
      [4, 5, 6, 7],
      [8],
    ]);
  });
});
