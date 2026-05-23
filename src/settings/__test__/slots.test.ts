// Slot-adaptive layout helpers (PR-4-6) — exercises the vector
// commit path the SlotTabStrip drives.
//
// The component itself is render-side; the testable contract is
// "vector commits at the active slot honor sync mode" which lives
// in commitVectorEdit + padVector (helpers.ts) and is already
// covered. This file adds the slot-state hook's clamp behavior
// and a few panel-level integration cases that span multiple
// helpers.

import { describe, expect, it } from "vitest";
import {
  commitVectorEdit,
  formatVector,
  padVector,
  parseVector,
} from "../inputs/helpers";

describe("vector commit through the SlotTabStrip's syncAll modes", () => {
  it("sync OFF + 4-slot vector + slot-2 edit lands at index 1 only", () => {
    const raw = "0.4,0.4,0.4,0.4";
    const parsed = parseVector(raw);
    const padded = padVector(parsed, 4);
    const committed = commitVectorEdit(padded, 1, "0.6", false);
    expect(formatVector(committed)).toBe("0.4,0.6,0.4,0.4");
  });

  it("sync ON + 4-slot vector + slot-2 edit broadcasts to every index", () => {
    const raw = "0.4,0.4,0.4,0.4";
    const parsed = parseVector(raw);
    const padded = padVector(parsed, 4);
    const committed = commitVectorEdit(padded, 1, "0.6", true);
    expect(formatVector(committed)).toBe("0.6,0.6,0.6,0.6");
  });

  it("wrap-extend on under-sized vector: '0.4' across 4 slots", () => {
    const padded = padVector(parseVector("0.4"), 4);
    expect(padded).toEqual(["0.4", "0.4", "0.4", "0.4"]);
  });

  it("clip on over-sized vector: 5-entry vector on a 4-slot printer", () => {
    const padded = padVector(parseVector("1,2,3,4,5"), 4);
    expect(padded).toEqual(["1", "2", "3", "4"]);
  });

  it("sync OFF + slot-3 edit on under-sized vector keeps other slots at the wrapped value", () => {
    // PLA across 4 slots starts as ["PLA"], padded to
    // ["PLA","PLA","PLA","PLA"]. Editing slot 3 to PETG should
    // produce ["PLA","PLA","PETG","PLA"].
    const padded = padVector(parseVector("PLA"), 4);
    const committed = commitVectorEdit(padded, 2, "PETG", false);
    expect(committed).toEqual(["PLA", "PLA", "PETG", "PLA"]);
  });
});
