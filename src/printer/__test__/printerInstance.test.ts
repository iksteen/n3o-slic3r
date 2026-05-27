// Pure helpers for the slot-binding panel (PR-S-7).

import { describe, expect, it } from "vitest";
import {
  flattenSlots,
  type PrinterInstance,
} from "../printerInstance";

function bambi(): PrinterInstance {
  return {
    id: "bambi",
    display_name: "Bambi",
    vendor_profile_ref: "bambu-lab-a1-mini",
    printer_fragment_slug: "bambu-lab-a1-mini",
    default_filament_fragment_slug: "bambu-pla-basic-bbl-a1m",
    default_process_fragment_slug: "0.20mm-standard-bbl-a1m",
    connection: null,
    extruders: [
      {
        installed_nozzle: { diameter_mm: 0.4, material: "stainless" },
        slots: [
          { feed: "direct", filament_identity: null, color: null },
        ],
      },
    ],
    bed: { identity: "Bambu Cool Plate SuperTack" },
    config_overrides: {},
  };
}

function snappy(): PrinterInstance {
  return {
    id: "snappy",
    display_name: "Snappy",
    vendor_profile_ref: "snapmaker-u1",
    printer_fragment_slug: "snapmaker-u1",
    default_filament_fragment_slug: "snapmaker-pla-u1",
    default_process_fragment_slug: "0.20-standard-snapmaker-u1-0.4-nozzle",
    connection: null,
    extruders: [0, 1, 2, 3].map(() => ({
      installed_nozzle: { diameter_mm: 0.4, material: "stainless" },
      slots: [
        { feed: "direct" as const, filament_identity: null, color: null },
      ],
    })),
    bed: { identity: "Snapmaker Textured PEI" },
    config_overrides: {},
  };
}

function ams_a1_mini(): PrinterInstance {
  // Hypothetical Bambi + AMS Lite shape: 1 extruder × 5 slots
  // (AMS:1..4 + Ext). AMS-first ordering matches BBS's
  // ams_mapping convention; see the Rust-side `bambi()` fixture.
  const ext: PrinterInstance["extruders"][number] = {
    installed_nozzle: { diameter_mm: 0.4, material: "stainless" },
    slots: [
      { feed: "ams", filament_identity: null, color: null },
      { feed: "ams", filament_identity: null, color: null },
      { feed: "ams", filament_identity: null, color: null },
      { feed: "ams", filament_identity: null, color: null },
      { feed: "direct", filament_identity: null, color: null },
    ],
  };
  return { ...bambi(), extruders: [ext] };
}

describe("flattenSlots", () => {
  it("Bambi: 1 entry labeled 'Direct'", () => {
    const slots = flattenSlots(bambi());
    expect(slots).toHaveLength(1);
    expect(slots[0].label).toBe("Direct");
    expect(slots[0].feed).toBe("direct");
    expect(slots[0].ref).toEqual({ extruder: 0, slot: 0 });
  });

  it("Snappy: 4 entries labeled by extruder (T1..T4) — solo slot label is empty", () => {
    const slots = flattenSlots(snappy());
    expect(slots.map((s) => s.label)).toEqual(["T1", "T2", "T3", "T4"]);
    expect(slots.every((s) => s.feed === "direct")).toBe(true);
    expect(slots.map((s) => s.ref.extruder)).toEqual([0, 1, 2, 3]);
  });

  it("A1+AMS shape: extruder label empty, slot labels carry the identity", () => {
    const slots = flattenSlots(ams_a1_mini());
    expect(slots.map((s) => s.label)).toEqual([
      "AMS:1",
      "AMS:2",
      "AMS:3",
      "AMS:4",
      "Ext",
    ]);
    expect(slots.slice(0, 4).every((s) => s.feed === "ams")).toBe(true);
    expect(slots[4].feed).toBe("direct");
  });

  it("multi-AMS shape (>4 AMS slots): letter-prefix disambiguation", () => {
    // 3 AMS units = 12 AMS slots + 1 Ext. Labels group into A:1..4,
    // B:1..4, C:1..4. Ext trails.
    const multi: PrinterInstance = {
      ...bambi(),
      extruders: [
        {
          installed_nozzle: { diameter_mm: 0.4, material: "stainless" },
          slots: [
            ...Array.from({ length: 12 }, () => ({
              feed: "ams" as const,
              filament_identity: null,
              color: null,
            })),
            { feed: "direct" as const, filament_identity: null, color: null },
          ],
        },
      ],
    };
    expect(flattenSlots(multi).map((s) => s.label)).toEqual([
      "AMS A:1", "AMS A:2", "AMS A:3", "AMS A:4",
      "AMS B:1", "AMS B:2", "AMS B:3", "AMS B:4",
      "AMS C:1", "AMS C:2", "AMS C:3", "AMS C:4",
      "Ext",
    ]);
  });
});

