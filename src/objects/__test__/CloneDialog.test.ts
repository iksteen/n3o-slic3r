// CloneDialog pure-helper test. Component rendering needs a jsdom + RTL
// setup we don't have (same as AddPrinterModal.test.tsx), so we pin the
// copies-field validation: only integers >= 1 are a usable count; anything
// else (0, negative, blank, fractional, non-numeric) is rejected so the
// Clone button stays disabled.

import { describe, expect, it } from "vitest";
import { parseCopies } from "../CloneDialog";

describe("parseCopies", () => {
  it("accepts positive integers", () => {
    expect(parseCopies("1")).toBe(1);
    expect(parseCopies("12")).toBe(12);
  });

  it("rejects zero, negatives, and fractions", () => {
    expect(parseCopies("0")).toBeNull();
    expect(parseCopies("-3")).toBeNull();
    expect(parseCopies("1.5")).toBeNull();
  });

  it("rejects blank and non-numeric input", () => {
    expect(parseCopies("")).toBeNull();
    expect(parseCopies("abc")).toBeNull();
  });
});
