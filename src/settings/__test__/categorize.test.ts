// Category grouping + mode-filter helper tests.

import { describe, expect, it } from "vitest";
import {
  CATEGORY_ORDER,
  categorize,
  categoryCounts,
  passesMode,
} from "../nav/categories";
import type { OptionSummary } from "../types";

function stub(
  key: string,
  category: string | null,
  mode: OptionSummary["mode"] = "simple",
): OptionSummary {
  return {
    key,
    ty: "Float",
    label: key,
    category,
    group: null,
    default_value: { kind: "scalar", value: "0" },
    multiline: false,
    is_color: false,
    enum_values: [],
    tooltip: null,
    sidetext: null,
    mode,
    scope: { project: false, object: false, region: true },
    capability: null,
  };
}

describe("categorize", () => {
  it("groups by category and preserves declaration order within each", () => {
    const opts = [
      stub("layer_height", "Quality"),
      stub("wall_count", "Strength"),
      stub("initial_layer_h", "Quality"),
      stub("infill_density", "Strength"),
    ];
    const groups = categorize(opts);

    expect(groups.map((g) => g.id)).toEqual(["Quality", "Strength"]);
    expect(groups[0].settings.map((s) => s.key)).toEqual([
      "layer_height",
      "initial_layer_h",
    ]);
    expect(groups[1].settings.map((s) => s.key)).toEqual([
      "wall_count",
      "infill_density",
    ]);
  });

  it("orders categories per CATEGORY_ORDER", () => {
    const opts = [
      stub("a", "Speed"),
      stub("b", "Quality"),
      stub("c", "Support"),
      stub("d", "Strength"),
    ];
    const ids = categorize(opts).map((g) => g.id);
    // Verify that the result matches the relative declaration order
    // from CATEGORY_ORDER for the categories that appear.
    const expected = CATEGORY_ORDER.filter((c) =>
      ["Quality", "Strength", "Speed", "Support"].includes(c),
    );
    expect(ids).toEqual(expected);
  });

  it("elides empty categories", () => {
    const opts = [stub("a", "Quality")];
    const ids = categorize(opts).map((g) => g.id);
    expect(ids).toEqual(["Quality"]);
  });

  it("orders unknown categories by first appearance (Orca display order), after the canonical ones", () => {
    // Options arrive pre-sorted into Orca's display order, so a trailing
    // category's first-appearance is its Orca position — Zoo before Future
    // because its option comes first, not alphabetically.
    const opts = [
      stub("a", "Quality"),
      stub("b", "Zoo"),
      stub("c", "Future"),
    ];
    const ids = categorize(opts).map((g) => g.id);
    expect(ids).toEqual(["Quality", "Zoo", "Future"]);
  });

  it("with an empty pinned order, orders every category by first appearance", () => {
    // The printer/filament panels pass []: their pages are already in Orca Tab
    // order via display-order sort, so "Machine limits" (in CATEGORY_ORDER) must
    // stay in place, not jump to the front.
    const opts = [
      stub("a", "Basic information"),
      stub("b", "Machine limits"),
      stub("c", "Retraction"),
    ];
    expect(categorize(opts, []).map((g) => g.id)).toEqual([
      "Basic information",
      "Machine limits",
      "Retraction",
    ]);
  });

  it("buckets null category as 'Other'", () => {
    const opts = [stub("orphan", null)];
    const ids = categorize(opts).map((g) => g.id);
    expect(ids).toEqual(["Other"]);
  });
});

describe("categoryCounts", () => {
  it("counts total and overrides per group", () => {
    const opts = [
      stub("a", "Quality"),
      stub("b", "Quality"),
      stub("c", "Strength"),
    ];
    const groups = categorize(opts);
    const counts = categoryCounts(groups, new Set(["a"]));
    expect(counts.get("Quality")).toEqual({ total: 2, overrides: 1 });
    expect(counts.get("Strength")).toEqual({ total: 1, overrides: 0 });
  });
});

describe("passesMode", () => {
  it("Simple filter shows Simple only", () => {
    expect(passesMode(stub("a", "Q", "simple"), "simple")).toBe(true);
    expect(passesMode(stub("a", "Q", "advanced"), "simple")).toBe(false);
    expect(passesMode(stub("a", "Q", "expert"), "simple")).toBe(false);
  });

  it("Advanced shows Simple + Advanced", () => {
    expect(passesMode(stub("a", "Q", "simple"), "advanced")).toBe(true);
    expect(passesMode(stub("a", "Q", "advanced"), "advanced")).toBe(true);
    expect(passesMode(stub("a", "Q", "expert"), "advanced")).toBe(false);
  });

  it("Expert shows everything except Develop", () => {
    expect(passesMode(stub("a", "Q", "simple"), "expert")).toBe(true);
    expect(passesMode(stub("a", "Q", "advanced"), "expert")).toBe(true);
    expect(passesMode(stub("a", "Q", "expert"), "expert")).toBe(true);
    expect(passesMode(stub("a", "Q", "develop"), "expert")).toBe(false);
  });

  it("Develop shows everything", () => {
    expect(passesMode(stub("a", "Q", "develop"), "develop")).toBe(true);
    expect(passesMode(stub("a", "Q", "expert"), "develop")).toBe(true);
  });
});
