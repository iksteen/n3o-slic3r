// PR-5-4 printer command wrappers — wire-shape contract tests.

import { beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

import {
  printerCatalog,
  rebindPlatePrinter,
  setActivePrinter,
} from "../printerCommands";

beforeEach(() => {
  invokeMock.mockReset();
});

describe("printerCatalog", () => {
  it("invokes printer_catalog and returns the parsed list", async () => {
    const minimalProfile = (model: string) => ({
      model,
      slot_count: 4,
      supported_build_plates: ["Textured PEI"],
      toolheads: [
        {
          nozzle_diameter: 0.4,
          hotend_type: "stainless_steel",
          max_temp: 300,
          slot_indices: [0, 1, 2, 3],
        },
      ],
      build_volume: { min: [0, 0, 0] as [number, number, number], max: [180, 180, 180] as [number, number, number] },
      exclusion_zones: [],
    });
    invokeMock.mockResolvedValueOnce([
      { identity: "bambu-a1-mini", profile: minimalProfile("Bambu A1 mini") },
      { identity: "snapmaker-u1", profile: minimalProfile("Snapmaker U1") },
    ]);
    const entries = await printerCatalog();
    expect(invokeMock).toHaveBeenCalledWith("printer_catalog");
    expect(entries).toHaveLength(2);
    expect(entries[0].identity).toBe("bambu-a1-mini");
    expect(entries[0].profile.model).toBe("Bambu A1 mini");
    expect(entries[1].identity).toBe("snapmaker-u1");
  });
});

describe("rebindPlatePrinter", () => {
  it("invokes scene_rebind_plate_printer with plateId/printerIdentity/buildPlateIdentity", async () => {
    invokeMock.mockResolvedValueOnce({
      plate_id: 1,
      previous_printer: null,
      new_printer: "bambu-a1-mini",
      new_build_plate: "Textured PEI",
      incompatible: [],
      clamped: [],
    });
    const report = await rebindPlatePrinter(1, "bambu-a1-mini", "Textured PEI");
    expect(invokeMock).toHaveBeenCalledWith("scene_rebind_plate_printer", {
      plateId: 1,
      printerIdentity: "bambu-a1-mini",
      buildPlateIdentity: "Textured PEI",
    });
    expect(report.new_printer).toBe("bambu-a1-mini");
    expect(report.incompatible).toEqual([]);
  });

  it("propagates the backend's UnsupportedBuildPlate rejection", async () => {
    invokeMock.mockRejectedValueOnce(
      "plate 1: printer `bambu-a1-mini` does not support build plate `Magnetic`",
    );
    await expect(
      rebindPlatePrinter(1, "bambu-a1-mini", "Magnetic"),
    ).rejects.toMatch(/does not support build plate/);
  });
});

describe("setActivePrinter", () => {
  it("passes through to scene_set_active_printer with the printer payload", async () => {
    invokeMock.mockResolvedValueOnce(undefined);
    const printer = {
      model: "X",
      slot_count: 1,
      supported_build_plates: [],
      toolheads: [],
      build_volume: { min: [0, 0, 0], max: [1, 1, 1] },
      exclusion_zones: [],
    } as never;
    await setActivePrinter(printer);
    expect(invokeMock).toHaveBeenCalledWith("scene_set_active_printer", {
      printer,
    });
  });

  it("passes null to clear the active printer", async () => {
    invokeMock.mockResolvedValueOnce(undefined);
    await setActivePrinter(null);
    expect(invokeMock).toHaveBeenCalledWith("scene_set_active_printer", {
      printer: null,
    });
  });
});
