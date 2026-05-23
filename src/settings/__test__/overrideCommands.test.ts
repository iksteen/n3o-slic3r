// Per-object override invoke-wrapper tests (PR-5-7 frontend).
//
// We're not validating the backend storage here (that's covered by
// the Rust scene_object_override_* tests). What this exercises is
// the wire shape — that the panel sends the right command name with
// the right serde-snake_case keys — plus `makeObjectOverrideCallbacks`'s
// null-context fallback.

import { beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

import {
  clearAllObjectOverrides,
  clearObjectOverride,
  makeObjectOverrideCallbacks,
  setObjectOverride,
} from "../overrideCommands";

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockResolvedValue(undefined);
});

describe("setObjectOverride", () => {
  it("invokes scene_object_override_set with plateId/objectId/key/value", async () => {
    await setObjectOverride(7, 42, "layer_height", "0.2");
    expect(invokeMock).toHaveBeenCalledOnce();
    expect(invokeMock).toHaveBeenCalledWith("scene_object_override_set", {
      plateId: 7,
      objectId: 42,
      key: "layer_height",
      value: "0.2",
    });
  });

  it("propagates rejection from invoke", async () => {
    invokeMock.mockRejectedValueOnce(new Error("backend boom"));
    await expect(setObjectOverride(1, 1, "k", "v")).rejects.toThrow(
      "backend boom",
    );
  });
});

describe("clearObjectOverride", () => {
  it("invokes scene_object_override_clear with plateId/objectId/key", async () => {
    await clearObjectOverride(3, 9, "wall_loops");
    expect(invokeMock).toHaveBeenCalledWith("scene_object_override_clear", {
      plateId: 3,
      objectId: 9,
      key: "wall_loops",
    });
  });
});

describe("clearAllObjectOverrides", () => {
  it("invokes scene_object_override_clear_all with plateId/objectId", async () => {
    await clearAllObjectOverrides(2, 11);
    expect(invokeMock).toHaveBeenCalledWith("scene_object_override_clear_all", {
      plateId: 2,
      objectId: 11,
    });
  });
});

describe("makeObjectOverrideCallbacks", () => {
  it("binds plate+object so the panel-supplied (key, value) reaches the backend", () => {
    const cbs = makeObjectOverrideCallbacks(5, 17);
    cbs.onSetObjectOverride("sparse_infill_density", "25");
    expect(invokeMock).toHaveBeenCalledWith("scene_object_override_set", {
      plateId: 5,
      objectId: 17,
      key: "sparse_infill_density",
      value: "25",
    });
  });

  it("clear callback routes through the bound (plate, object)", () => {
    const cbs = makeObjectOverrideCallbacks(5, 17);
    cbs.onClearObjectOverride("nozzle_temperature");
    expect(invokeMock).toHaveBeenCalledWith("scene_object_override_clear", {
      plateId: 5,
      objectId: 17,
      key: "nozzle_temperature",
    });
  });

  it("returns silent no-ops when plateId is null (Object tab with no selection)", () => {
    const cbs = makeObjectOverrideCallbacks(null, 17);
    cbs.onSetObjectOverride("k", "v");
    cbs.onClearObjectOverride("k");
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("returns silent no-ops when objectId is null", () => {
    const cbs = makeObjectOverrideCallbacks(5, null);
    cbs.onSetObjectOverride("k", "v");
    cbs.onClearObjectOverride("k");
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("swallows + logs invoke failure so the panel UI doesn't break", async () => {
    const consoleSpy = vi
      .spyOn(console, "error")
      .mockImplementation(() => undefined);
    invokeMock.mockRejectedValueOnce(new Error("set failed"));
    const cbs = makeObjectOverrideCallbacks(1, 1);
    cbs.onSetObjectOverride("k", "v");
    // Let the rejection settle.
    await new Promise((r) => setTimeout(r, 0));
    expect(consoleSpy).toHaveBeenCalledWith(
      "[settings] setObjectOverride failed",
      expect.any(Error),
    );
    consoleSpy.mockRestore();
  });
});
