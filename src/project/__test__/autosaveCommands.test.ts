// Autosave invoke wrappers — wire-shape contract.

import { beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

import {
  autosaveDisable,
  autosaveDrop,
  autosaveEnable,
  autosaveList,
  projectLoad,
} from "../autosaveCommands";

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockResolvedValue(undefined);
});

describe("autosaveEnable / autosaveDisable", () => {
  it("enables with no args", async () => {
    await autosaveEnable();
    expect(invokeMock).toHaveBeenCalledWith("project_autosave_enable");
  });
  it("disables with no args", async () => {
    await autosaveDisable();
    expect(invokeMock).toHaveBeenCalledWith("project_autosave_disable");
  });
});

describe("autosaveList", () => {
  it("invokes project_autosave_list and returns the parsed list", async () => {
    invokeMock.mockResolvedValueOnce([
      {
        uuid: "abc12345-...",
        path: "/tmp/n3o/abc.3mf",
        modified_unix_secs: 1716480000,
        size_bytes: 12345,
      },
    ]);
    const list = await autosaveList();
    expect(invokeMock).toHaveBeenCalledWith("project_autosave_list");
    expect(list).toHaveLength(1);
    expect(list[0].uuid).toBe("abc12345-...");
  });

  it("returns an empty array when no recoveries exist", async () => {
    invokeMock.mockResolvedValueOnce([]);
    const list = await autosaveList();
    expect(list).toEqual([]);
  });
});

describe("autosaveDrop", () => {
  it("invokes project_autosave_drop with the uuid", async () => {
    await autosaveDrop("abc12345");
    expect(invokeMock).toHaveBeenCalledWith("project_autosave_drop", {
      uuid: "abc12345",
    });
  });
});

describe("projectLoad", () => {
  it("invokes project_load with the path arg the recovery dialog passes", async () => {
    await projectLoad("/tmp/n3o/abc.3mf");
    expect(invokeMock).toHaveBeenCalledWith("project_load", {
      path: "/tmp/n3o/abc.3mf",
    });
  });
});
