// PR-5-6 UI material binding wrappers — wire-shape contract.

import { beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

import {
  autoBindMaterials,
  clearMaterialBinding,
  setMaterialBinding,
} from "../materialCommands";

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockResolvedValue(undefined);
});

describe("setMaterialBinding", () => {
  it("invokes project_set_material_binding with the full quartet of args", async () => {
    await setMaterialBinding(7, 3, 2, "Generic PETG");
    expect(invokeMock).toHaveBeenCalledWith("project_set_material_binding", {
      plateId: 7,
      modelMaterial: 3,
      physicalSlot: 2,
      filamentIdentity: "Generic PETG",
    });
  });

  it("propagates the backend's range rejection", async () => {
    invokeMock.mockRejectedValueOnce(
      "plate 7: physical_slot 9 out of range (slot_count = 4)",
    );
    await expect(
      setMaterialBinding(7, 1, 9, "Generic PLA"),
    ).rejects.toMatch(/out of range/);
  });
});

describe("clearMaterialBinding", () => {
  it("invokes project_clear_material_binding with plate + material id", async () => {
    await clearMaterialBinding(3, 2);
    expect(invokeMock).toHaveBeenCalledWith("project_clear_material_binding", {
      plateId: 3,
      modelMaterial: 2,
    });
  });
});

describe("autoBindMaterials", () => {
  it("invokes project_auto_bind_materials with plate + slotCount", async () => {
    invokeMock.mockResolvedValueOnce([
      { model_material: 1, physical_slot: 1, filament_identity: "Generic PLA" },
      { model_material: 2, physical_slot: 2, filament_identity: "Generic PLA" },
    ]);
    const bindings = await autoBindMaterials(5, 4);
    expect(invokeMock).toHaveBeenCalledWith("project_auto_bind_materials", {
      plateId: 5,
      slotCount: 4,
    });
    expect(bindings).toHaveLength(2);
    expect(bindings[0].physical_slot).toBe(1);
  });
});
