// Diff helpers (PR-4-10).

import { describe, expect, it } from "vitest";
import { computeDiff, passesDiff } from "../diff";
import type { ResolvedMap } from "../resolve";

function entry(value: string): ResolvedMap[string] {
  return { value, source_layer: null, cascade_fallback: null };
}

describe("computeDiff", () => {
  it("returns empty when resolved equals baseline", () => {
    const resolved: ResolvedMap = { a: entry("1"), b: entry("2") };
    const baseline: ResolvedMap = { a: entry("1"), b: entry("2") };
    expect(computeDiff(resolved, baseline).size).toBe(0);
  });

  it("captures changed values", () => {
    const resolved: ResolvedMap = { a: entry("1"), b: entry("3") };
    const baseline: ResolvedMap = { a: entry("1"), b: entry("2") };
    const diff = computeDiff(resolved, baseline);
    expect([...diff]).toEqual(["b"]);
  });

  it("captures additions and removals", () => {
    const resolved: ResolvedMap = { a: entry("1"), c: entry("3") };
    const baseline: ResolvedMap = { a: entry("1"), b: entry("2") };
    const diff = computeDiff(resolved, baseline);
    expect(diff.has("b")).toBe(true);
    expect(diff.has("c")).toBe(true);
    expect(diff.size).toBe(2);
  });
});

describe("passesDiff", () => {
  const diff = new Set(["a", "b"]);
  it("'all' passes everything", () => {
    expect(passesDiff("anything", "all", diff)).toBe(true);
  });
  it("non-'all' passes only diff keys", () => {
    expect(passesDiff("a", "from-default", diff)).toBe(true);
    expect(passesDiff("z", "from-default", diff)).toBe(false);
    expect(passesDiff("a", "from-save", diff)).toBe(true);
    expect(passesDiff("z", "from-save", diff)).toBe(false);
  });
});
