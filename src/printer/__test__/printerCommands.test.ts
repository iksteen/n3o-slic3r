// Printer command wrappers — wire-shape contract tests.

import { beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

import { printerCatalog, rebindPlatePrinter } from "../printerCommands";

beforeEach(() => {
  invokeMock.mockReset();
});

describe("printerCatalog", () => {
  it("invokes printer_catalog and returns the parsed list", async () => {
    const minimalProfile = (model: string) => ({
      model,
      supported_build_plates: ["Textured PEI"],
      toolheads: [
        {
          default_nozzle_diameter: "0.4",
          hotend_type: "stainless_steel",
          max_temp: 300,
        },
      ],
      build_volume: { min: [0, 0, 0] as [number, number, number], max: [180, 180, 180] as [number, number, number] },
      exclusion_zones: [],
    });
    invokeMock.mockResolvedValueOnce([
      { identity: "bambu-lab-a1-mini", profile: minimalProfile("Bambu A1 mini") },
      { identity: "snapmaker-u1", profile: minimalProfile("Snapmaker U1") },
    ]);
    const entries = await printerCatalog();
    expect(invokeMock).toHaveBeenCalledWith("printer_catalog");
    expect(entries).toHaveLength(2);
    expect(entries[0].identity).toBe("bambu-lab-a1-mini");
    expect(entries[0].profile.model).toBe("Bambu A1 mini");
    expect(entries[1].identity).toBe("snapmaker-u1");
  });
});

describe("rebindPlatePrinter", () => {
  it("invokes scene_rebind_plate_printer with plateId/instanceId", async () => {
    invokeMock.mockResolvedValueOnce({
      plate_id: 1,
      previous_printer: null,
      new_printer: "bambu-lab-a1-mini",
      new_build_plate: "Supertack Plate",
    });
    const report = await rebindPlatePrinter(1, "bambi");
    expect(invokeMock).toHaveBeenCalledWith("scene_rebind_plate_printer", {
      plateId: 1,
      instanceId: "bambi",
    });
    expect(report.new_printer).toBe("bambu-lab-a1-mini");
  });
});
