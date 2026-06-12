// Object command wrappers — wire-shape contract.

import { beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

import { moveObjectsToPlate } from "../objectCommands";

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockResolvedValue(undefined);
});

describe("moveObjectsToPlate", () => {
  it("invokes scene_move_objects_to_plate with the camelCase arg shape", async () => {
    await moveObjectsToPlate(1, 2, [10, 11, 12]);
    expect(invokeMock).toHaveBeenCalledWith("scene_move_objects_to_plate", {
      fromPlate: 1,
      toPlate: 2,
      objectIds: [10, 11, 12],
    });
  });

  it("passes an empty set through unchanged (backend treats it as a no-op)", async () => {
    await moveObjectsToPlate(3, 4, []);
    expect(invokeMock).toHaveBeenCalledWith("scene_move_objects_to_plate", {
      fromPlate: 3,
      toPlate: 4,
      objectIds: [],
    });
  });
});
