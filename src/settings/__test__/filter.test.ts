// Per-row filter logic for SettingsPanel (PR-4-4).

import { describe, expect, it } from "vitest";
import { filterRow } from "../SettingsPanel";
import type { PrinterAwareOptionSummary } from "../types";

function stub(over: Partial<PrinterAwareOptionSummary> = {}): PrinterAwareOptionSummary {
  return {
    key: "layer_height",
    ty: "Float",
    label: "Layer height",
    category: "Quality",
    default_value: { kind: "scalar", value: "0.2" },
    multiline: false,
    enum_values: [],
    tooltip: null,
    mode: "simple",
    scope: { project: false, object: true, region: true },
    capability: null,
    hidden: false,
    ...over,
  };
}

describe("filterRow", () => {
  it("passes a Simple-mode option in any visible filter", () => {
    expect(filterRow(stub(), "simple", "")).toBe(true);
    expect(filterRow(stub(), "advanced", "")).toBe(true);
    expect(filterRow(stub(), "expert", "")).toBe(true);
  });

  it("hides advanced options when Simple is active", () => {
    expect(filterRow(stub({ mode: "advanced" }), "simple", "")).toBe(false);
    expect(filterRow(stub({ mode: "advanced" }), "advanced", "")).toBe(true);
  });

  it("matches search across key, label, and category", () => {
    expect(filterRow(stub(), "expert", "layer")).toBe(true); // key
    expect(filterRow(stub(), "expert", "Quality")).toBe(true); // category
    expect(filterRow(stub(), "expert", "height")).toBe(true); // label
    expect(filterRow(stub(), "expert", "speed")).toBe(false); // no match
  });

  it("hides capability-hidden options in the default view", () => {
    expect(filterRow(stub({ hidden: true }), "expert", "")).toBe(false);
  });

  it("surfaces capability-hidden options when search is active", () => {
    // Per PR-4-5 spec: hidden options are findable via search with
    // a 'not applicable' badge. The filter says yes; the badge
    // rendering is PR-4-5's responsibility.
    expect(filterRow(stub({ hidden: true }), "expert", "layer")).toBe(true);
  });

  it("search is case-insensitive", () => {
    expect(filterRow(stub(), "expert", "LAYER")).toBe(true);
    expect(filterRow(stub(), "expert", "QuAlItY")).toBe(true);
  });
});
