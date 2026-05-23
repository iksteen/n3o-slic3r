// PR-5-3 plate command wrappers — wire-shape contract.

import { beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

import {
  addPlate,
  removePlate,
  renamePlate,
  setActivePlate,
} from "../plateCommands";

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockResolvedValue(undefined);
});

describe("addPlate", () => {
  it("invokes scene_add_plate with printer=null when called with no args", async () => {
    invokeMock.mockResolvedValueOnce(7);
    const id = await addPlate();
    expect(invokeMock).toHaveBeenCalledWith("scene_add_plate", { printer: null });
    expect(id).toBe(7);
  });

  it("passes through a PrinterBinding when supplied", async () => {
    invokeMock.mockResolvedValueOnce(8);
    await addPlate({
      printer_identity: "bambu_a1_mini",
      build_plate_identity: "textured_pei",
    });
    expect(invokeMock).toHaveBeenCalledWith("scene_add_plate", {
      printer: {
        printer_identity: "bambu_a1_mini",
        build_plate_identity: "textured_pei",
      },
    });
  });
});

describe("removePlate", () => {
  it("invokes scene_remove_plate with the plate id", async () => {
    await removePlate(3);
    expect(invokeMock).toHaveBeenCalledWith("scene_remove_plate", { plateId: 3 });
  });
});

describe("setActivePlate", () => {
  it("invokes scene_set_active_plate with the plate id", async () => {
    await setActivePlate(5);
    expect(invokeMock).toHaveBeenCalledWith("scene_set_active_plate", {
      plateId: 5,
    });
  });
});

describe("renamePlate", () => {
  it("invokes scene_rename_plate with plate id + name", async () => {
    await renamePlate(2, "Calibration");
    expect(invokeMock).toHaveBeenCalledWith("scene_rename_plate", {
      plateId: 2,
      name: "Calibration",
    });
  });

  it("propagates backend rejection (caller is expected to catch + log)", async () => {
    invokeMock.mockRejectedValueOnce("plate 2: plate name must not be empty");
    await expect(renamePlate(2, "")).rejects.toBe(
      "plate 2: plate name must not be empty",
    );
  });
});
