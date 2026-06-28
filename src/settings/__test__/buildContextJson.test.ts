// Context-builder + override-TOML escaping tests.

import { describe, expect, it } from "vitest";
import {
  buildContextJson,
  DEFAULT_BUILD_PLATE,
  DEFAULT_FILAMENT,
  overridesToFileSpec,
} from "../buildContextJson";
import type { PrinterProfileJson } from "../resolve";

const PRINTER: PrinterProfileJson = {
  model: "Bambu A1 mini",
  brand: "Bambu Lab",
  brand_short: "B",
  ams_max: 1,
  ams_type: "AMS Lite",
  ams_slots_per_unit: 4,
  default_bed: "Textured PEI",
  supported_build_plates: ["Textured PEI", "Cool"],
  available_nozzle_diameters: ["0.4"],
  toolheads: [
    {
      default_nozzle_diameter: "0.4",
      hotend_type: "stainless_steel",
      max_temp: 300,
    },
  ],
  build_volume: { min: [0, 0, 0], max: [180, 180, 180] },
  exclusion_zones: [],
  driver_kind: "bambu",
};

describe("overridesToFileSpec", () => {
  it("returns null for an empty override map (caller skips the spec)", () => {
    expect(overridesToFileSpec("user-overrides", {})).toBeNull();
  });

  it("serializes a single override as TOML key = \"value\"", () => {
    const spec = overridesToFileSpec("project-overrides", {
      layer_height: "0.12",
    });
    expect(spec).toEqual({
      label: "project-overrides",
      content: 'layer_height = "0.12"\n',
    });
  });

  it("sorts keys alphabetically so cache keys stay stable across map iteration order", () => {
    const spec = overridesToFileSpec("user-overrides", {
      z: "1",
      a: "2",
      m: "3",
    });
    expect(spec?.content).toBe('a = "2"\nm = "3"\nz = "1"\n');
  });

  it("escapes embedded quotes + backslashes in values", () => {
    const spec = overridesToFileSpec("project-overrides", {
      raw: 'has "quote" and \\backslash',
    });
    expect(spec?.content).toBe(
      'raw = "has \\"quote\\" and \\\\backslash"\n',
    );
  });
});

describe("buildContextJson", () => {
  it("packs the printer + defaults + tiered overrides into the resolver shape", () => {
    const ctx = buildContextJson({
      printer: PRINTER,
      projectOverrides: { layer_height: "0.12" },
      userOverrides: {},
      objectOverrides: { wall_loops: "3" },
      activeSlot: 1,
    });
    expect(ctx.printer).toBe(PRINTER);
    expect(ctx.plate).toEqual(DEFAULT_BUILD_PLATE);
    expect(ctx.filaments).toEqual([DEFAULT_FILAMENT]);
    expect(ctx.active_slot).toBe(1);
    expect(ctx.user_overrides).toEqual([]);
    expect(ctx.project_overrides).toEqual([
      { label: "project-overrides", content: 'layer_height = "0.12"\n' },
    ]);
    expect(ctx.object_overrides).toEqual({ wall_loops: "3" });
  });

  it("emits both tiers when both have authored values", () => {
    const ctx = buildContextJson({
      printer: PRINTER,
      projectOverrides: { layer_height: "0.12" },
      userOverrides: { fan_speed: "100" },
      objectOverrides: {},
      activeSlot: 0,
    });
    expect(ctx.user_overrides).toHaveLength(1);
    expect(ctx.project_overrides).toHaveLength(1);
    expect(ctx.user_overrides[0].label).toBe("user-overrides");
    expect(ctx.project_overrides[0].label).toBe("project-overrides");
  });

  it("emits empty arrays when neither tier has values", () => {
    const ctx = buildContextJson({
      printer: PRINTER,
      projectOverrides: {},
      userOverrides: {},
      objectOverrides: {},
      activeSlot: 0,
    });
    expect(ctx.user_overrides).toEqual([]);
    expect(ctx.project_overrides).toEqual([]);
    expect(ctx.object_overrides).toEqual({});
  });
});
