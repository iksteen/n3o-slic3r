// PR-5-6 UI material binding wrappers — wire-shape contract.

import { beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

import {
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
