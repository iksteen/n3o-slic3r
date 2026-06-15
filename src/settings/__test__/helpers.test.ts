// Form-component helper tests (PR-4-2).
//
// The project's vitest setup runs pure-logic tests only (no DOM,
// no testing-library) — see src/slice/reducer.test.ts for the
// same pattern. Each input component delegates its
// parse/format/validate logic to helpers.ts; the React shell is
// thin DOM event plumbing on top. These tests cover the helpers;
// PR-4-13's exit smoke walks the rendered UI manually.

import { describe, expect, it } from "vitest";
import {
  clamp,
  commitColor,
  commitFloatOrPercent,
  commitNumber,
  commitPercent,
  formatBool,
  formatFloatOrPercent,
  formatNumber,
  isValidHexColor,
  parseBool,
  parseFloatOrPercent,
  parseNumber,
  parsePercent,
} from "../inputs/helpers";

describe("parseNumber", () => {
  it("accepts integers and decimals", () => {
    expect(parseNumber("42")).toBe(42);
    expect(parseNumber("3.14")).toBe(3.14);
    expect(parseNumber("-1.5")).toBe(-1.5);
    expect(parseNumber("  7 ")).toBe(7);
  });

  it("rejects empty / non-numeric / NaN / infinity", () => {
    expect(parseNumber("")).toBeNull();
    expect(parseNumber("abc")).toBeNull();
    expect(parseNumber("NaN")).toBeNull();
    expect(parseNumber("Infinity")).toBeNull();
  });
});

describe("clamp", () => {
  it("respects min and max bounds", () => {
    expect(clamp(5, { min: 0, max: 10 })).toBe(5);
    expect(clamp(-1, { min: 0, max: 10 })).toBe(0);
    expect(clamp(11, { min: 0, max: 10 })).toBe(10);
  });

  it("passes through when no bounds", () => {
    expect(clamp(42, {})).toBe(42);
  });
});

describe("formatNumber", () => {
  it("drops trailing-zero noise", () => {
    expect(formatNumber(1.2)).toBe("1.2");
    expect(formatNumber(0.1 + 0.2)).toBe("0.3");
    expect(formatNumber(42)).toBe("42");
  });

  it("respects decimal precision", () => {
    expect(formatNumber(0.123456, 2)).toBe("0.12");
  });
});

describe("commitNumber", () => {
  it("rejects unparseable input", () => {
    expect(commitNumber("abc", {})).toEqual({
      ok: false,
      error: "expected a number",
    });
  });

  it("clamps to bounds", () => {
    expect(commitNumber("100", { min: 0, max: 10 })).toEqual({
      ok: true,
      value: 10,
      serialized: "10",
    });
  });

  it("round-trips the serialized form", () => {
    const r = commitNumber("0.2", {});
    expect(r.ok).toBe(true);
    if (r.ok) expect(r.serialized).toBe("0.2");
  });
});

describe("parsePercent + commitPercent", () => {
  it("strips trailing %", () => {
    expect(parsePercent("75%")).toBe(75);
    expect(parsePercent("75")).toBe(75);
  });

  it("clamps to bounds", () => {
    expect(commitPercent("150", { min: 0, max: 100 })).toEqual({
      ok: true,
      value: 100,
      serialized: "100",
    });
  });
});

describe("parseFloatOrPercent + formatFloatOrPercent", () => {
  it("preserves the percent flag through round-trip", () => {
    const p = parseFloatOrPercent("120%");
    expect(p).toEqual({ value: 120, percent: true });
    expect(formatFloatOrPercent(p!)).toBe("120%");
  });

  it("preserves absolute mode", () => {
    const a = parseFloatOrPercent("0.5");
    expect(a).toEqual({ value: 0.5, percent: false });
    expect(formatFloatOrPercent(a!)).toBe("0.5");
  });

  it("commits with bounds", () => {
    const r = commitFloatOrPercent("200%", { max: 100 });
    expect(r).toEqual({
      ok: true,
      value: { value: 100, percent: true },
      serialized: "100%",
    });
  });
});

describe("color validation", () => {
  it("accepts #RRGGBB upper and lower case", () => {
    expect(isValidHexColor("#FFAA00")).toBe(true);
    expect(isValidHexColor("#ffaa00")).toBe(true);
  });

  it("rejects shorthand + non-hex chars", () => {
    expect(isValidHexColor("#abc")).toBe(false);
    expect(isValidHexColor("red")).toBe(false);
    expect(isValidHexColor("#GGGGGG")).toBe(false);
    expect(isValidHexColor("FFAA00")).toBe(false); // missing #
  });

  it("commits as lowercase", () => {
    expect(commitColor("#FFAA00")).toEqual({
      ok: true,
      value: "#ffaa00",
      serialized: "#ffaa00",
    });
  });
});

describe("bool helpers", () => {
  it("parses 1/true and 0/false (case-insensitive)", () => {
    expect(parseBool("1")).toBe(true);
    expect(parseBool("True")).toBe(true);
    expect(parseBool("0")).toBe(false);
    expect(parseBool("FALSE")).toBe(false);
  });

  it("rejects other input", () => {
    expect(parseBool("")).toBeNull();
    expect(parseBool("yes")).toBeNull();
  });

  it("serializes as libslic3r's 1/0 convention", () => {
    expect(formatBool(true)).toBe("1");
    expect(formatBool(false)).toBe("0");
  });
});
