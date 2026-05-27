// PR-7b-8 U1ToolheadStrip projection tests.
//
// Mirrors BambuAmsStrip.test's approach: component DOM-render
// isn't set up in this repo's vitest config; we exercise the pure
// projection function (cellsFromU1) that turns U1Extra + Temps
// into render-ready cell descriptors.

import { describe, expect, it } from "vitest";
import { cellsFromU1 } from "../U1ToolheadStrip";
import type { Temps, U1Extra, U1Filament } from "../types";

function filament(material: string, color: string): U1Filament {
  return { material_type: material, color };
}

function temps(currents: number[], targets: number[], bed = 60): Temps {
  return {
    nozzles: currents.map((c, i) => ({
      current: c,
      target: targets[i] ?? 0,
    })),
    bed: { current: bed, target: bed },
    chamber: null,
  };
}

function extra(
  filaments: (U1Filament | null)[],
  mounted: number | null = null,
): U1Extra {
  return {
    mounted_toolhead: mounted,
    toolhead_filaments: filaments,
    current_stage: null,
    fan_speed: null,
  };
}

describe("cellsFromU1", () => {
  it("always emits 4 cells regardless of input length", () => {
    // Printer reports only 2 toolheads loaded — the strip still
    // renders 4 fixed cells (T1..T4) so the U1's hardware shape is
    // visible at a glance. Empty cells fill in for the unreported
    // slots.
    const cells = cellsFromU1(
      extra([filament("PLA", "FF0000FF"), filament("PETG", "00FF00FF")]),
      temps([215, 220], [220, 220]),
    );
    expect(cells).toHaveLength(4);
    expect(cells.map((c) => c.index)).toEqual([0, 1, 2, 3]);
  });

  it("renders empty cells with dashed visual + dash material label", () => {
    const cells = cellsFromU1(extra([null, null, null, null]), temps([24, 24, 24, 24], [0, 0, 0, 0]));
    for (const c of cells) {
      expect(c.cssColor).toBeNull();
      expect(c.materialLabel).toBeNull();
    }
  });

  it("normalizes RGBA wire-hex per toolhead", () => {
    const cells = cellsFromU1(
      extra([
        filament("PLA", "ABCDEFFF"),
        filament("PETG", "12345680"),
        null,
        null,
      ]),
      temps([200, 200], [200, 200]),
    );
    // Same logic as BambuAmsStrip — alpha dropped when FF, kept
    // when non-opaque. Cross-validation that cssColorFromHex is
    // wired in.
    expect(cells[0].cssColor).toBe("#ABCDEF");
    expect(cells[1].cssColor).toBe("#12345680");
    expect(cells[2].cssColor).toBeNull();
  });

  it("truncates material types longer than 6 chars", () => {
    const cells = cellsFromU1(
      extra([
        filament("PLA", "FF0000FF"),
        filament("Carbon Fiber PLA", "00FF00FF"),
        filament("PETG-CF", "0000FFFF"),
        null,
      ]),
      temps([200, 200, 200], [200, 200, 200]),
    );
    expect(cells[0].materialLabel).toBe("PLA");
    expect(cells[1].materialLabel).toBe("Carbon");
    expect(cells[2].materialLabel).toBe("PETG-C");
  });

  it("marks only the mounted toolhead via isMounted", () => {
    const cells = cellsFromU1(
      extra(
        [
          filament("PLA", "FF0000FF"),
          filament("PLA", "00FF00FF"),
          filament("PLA", "0000FFFF"),
          filament("PLA", "FFFF00FF"),
        ],
        1,
      ),
      temps([200, 215, 24, 24], [200, 220, 0, 0]),
    );
    expect(cells.map((c) => c.isMounted)).toEqual([false, true, false, false]);
  });

  it("emits no isMounted flag when no toolhead is currently mounted", () => {
    // Idle printer: `toolhead.extruder` may be null between prints.
    const cells = cellsFromU1(
      extra([filament("PLA", "FF0000FF")], null),
      temps([24], [0]),
    );
    expect(cells.every((c) => !c.isMounted)).toBe(true);
  });

  it("formats per-toolhead temp readout from temps.nozzles[i]", () => {
    const cells = cellsFromU1(
      extra([
        filament("PLA", "FF0000FF"),
        filament("PLA", "00FF00FF"),
        null,
        null,
      ]),
      temps([215.4, 24.6], [220, 0]),
    );
    expect(cells[0].tempReadout).toBe("215/220°");
    expect(cells[1].tempReadout).toBe("25/0°");
    // Empty toolheads with no nozzle reading fall back to em dash.
    expect(cells[2].tempReadout).toBe("—");
  });

  it("aria label calls out mounted state + full untruncated material", () => {
    const cells = cellsFromU1(
      extra([null, filament("Carbon Fiber PLA", "ABCDEFFF")], 1),
      temps([24, 215], [0, 220]),
    );
    expect(cells[0].ariaLabel).toContain("T1");
    expect(cells[0].ariaLabel).toContain("empty");
    expect(cells[1].ariaLabel).toContain("(mounted)");
    expect(cells[1].ariaLabel).toContain("Carbon Fiber PLA");
    expect(cells[1].ariaLabel).toContain("215/220°");
  });
});
