// Project-tier override invoke wrappers — wire-shape +
// bound-callback contract.

import { beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

import {
  clearAllProjectOverrides,
  clearProjectOverride,
  makeProjectOverrideCallbacks,
  setProjectOverride,
} from "../projectOverrideCommands";

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockResolvedValue(undefined);
});

describe("setProjectOverride", () => {
  it("invokes scene_project_override_set with plateId/key/value", async () => {
    await setProjectOverride(5, "layer_height", "0.12");
    expect(invokeMock).toHaveBeenCalledWith("scene_project_override_set", {
      plateId: 5,
      key: "layer_height",
      value: "0.12",
    });
  });
});

describe("clearProjectOverride", () => {
  it("invokes scene_project_override_clear with plateId/key", async () => {
    await clearProjectOverride(3, "wall_loops");
    expect(invokeMock).toHaveBeenCalledWith("scene_project_override_clear", {
      plateId: 3,
      key: "wall_loops",
    });
  });
});

describe("clearAllProjectOverrides", () => {
  it("invokes scene_project_override_clear_all with plateId", async () => {
    await clearAllProjectOverrides(7);
    expect(invokeMock).toHaveBeenCalledWith(
      "scene_project_override_clear_all",
      { plateId: 7 },
    );
  });
});

describe("makeProjectOverrideCallbacks", () => {
  it("binds plate so (key, value) calls reach the backend", () => {
    const cbs = makeProjectOverrideCallbacks(2);
    cbs.onSetProjectOverride("infill_density", "30");
    expect(invokeMock).toHaveBeenCalledWith("scene_project_override_set", {
      plateId: 2,
      key: "infill_density",
      value: "30",
    });
  });

  it("clear binds the plate too", () => {
    const cbs = makeProjectOverrideCallbacks(2);
    cbs.onClearProjectOverride("infill_density");
    expect(invokeMock).toHaveBeenCalledWith("scene_project_override_clear", {
      plateId: 2,
      key: "infill_density",
    });
  });

  it("returns silent no-ops when plateId is null", () => {
    const cbs = makeProjectOverrideCallbacks(null);
    cbs.onSetProjectOverride("k", "v");
    cbs.onClearProjectOverride("k");
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("catches + logs backend failure on set so the panel UI doesn't break", async () => {
    const consoleSpy = vi
      .spyOn(console, "error")
      .mockImplementation(() => undefined);
    invokeMock.mockRejectedValueOnce(new Error("boom"));
    const cbs = makeProjectOverrideCallbacks(2);
    cbs.onSetProjectOverride("k", "v");
    await new Promise((r) => setTimeout(r, 0));
    expect(consoleSpy).toHaveBeenCalledWith(
      "[settings] setProjectOverride failed",
      expect.any(Error),
    );
    consoleSpy.mockRestore();
  });
});
