// Pure helpers for the slot-binding panel (PR-S-7).

import { describe, expect, it } from "vitest";
import {
  flattenSlots,
  isFeedMixConflict,
  type FlatSlotOption,
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
        label: "",
        installed_nozzle: { diameter_mm: 0.4, material: "stainless" },
        slots: [
          { label: "Direct", feed: "direct", filament_identity: null, color: null },
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
    extruders: ["T0", "T1", "T2", "T3"].map((label) => ({
      label,
      installed_nozzle: { diameter_mm: 0.4, material: "stainless" },
      slots: [
        { label: "", feed: "direct" as const, filament_identity: null, color: null },
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
    label: "",
    installed_nozzle: { diameter_mm: 0.4, material: "stainless" },
    slots: [
      { label: "AMS:1", feed: "ams", filament_identity: null, color: null },
      { label: "AMS:2", feed: "ams", filament_identity: null, color: null },
      { label: "AMS:3", feed: "ams", filament_identity: null, color: null },
      { label: "AMS:4", feed: "ams", filament_identity: null, color: null },
      { label: "Ext", feed: "direct", filament_identity: null, color: null },
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

  it("Snappy: 4 entries labeled by extruder (T0..T3) — solo slot label is empty", () => {
    const slots = flattenSlots(snappy());
    expect(slots.map((s) => s.label)).toEqual(["T0", "T1", "T2", "T3"]);
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
});

describe("isFeedMixConflict", () => {
  const slots = flattenSlots(ams_a1_mini());
  const ams1 = slots[0]!; // Ams
  const ams2 = slots[1]!; // Ams
  const ext = slots[4]!; // Direct

  it("flags Direct → already-Ams-used on same extruder", () => {
    expect(isFeedMixConflict(ext, [ams1])).toBe(true);
  });

  it("flags Ams → already-Direct-used on same extruder", () => {
    expect(isFeedMixConflict(ams1, [ext])).toBe(true);
  });

  it("allows Ams + Ams on same extruder (AMS handles the swap)", () => {
    expect(isFeedMixConflict(ams1, [ams2])).toBe(false);
  });

  it("allows mixed kinds across different extruders", () => {
    // Synthesize a 2-extruder fixture, conflicts only matter
    // per-extruder.
    const otherExtAms: FlatSlotOption = {
      ref: { extruder: 1, slot: 0 },
      label: "T1 — AMS:1",
      feed: "ams",
      filament_identity: null,
      color: null,
    };
    expect(isFeedMixConflict(ext, [otherExtAms])).toBe(false);
  });

  it("empty already-used → never a conflict", () => {
    expect(isFeedMixConflict(ext, [])).toBe(false);
    expect(isFeedMixConflict(ams1, [])).toBe(false);
  });
});
