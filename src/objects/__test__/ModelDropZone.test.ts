// ModelDropZone routing logic — extension → loader, error paths.
// The React subscription/overlay shell is shared with the preview
// DropZone and covered there.

import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("../objectCommands", () => ({
  loadModelFromPath: vi.fn(),
}));

import { handleModelDrop } from "../ModelDropZone";
import { loadModelFromPath } from "../objectCommands";

const loadMock = loadModelFromPath as ReturnType<typeof vi.fn>;

afterEach(() => {
  vi.resetAllMocks();
});

describe("handleModelDrop", () => {
  it("routes .stl/.obj/.3mf to loadModelFromPath, in drop order", async () => {
    loadMock.mockResolvedValue(undefined);
    const onError = vi.fn();
    await handleModelDrop(["/a.stl", "/b.OBJ", "/c.3mf"], onError);
    expect(loadMock.mock.calls).toEqual([["/a.stl"], ["/b.OBJ"], ["/c.3mf"]]);
    expect(onError).not.toHaveBeenCalled();
  });

  it("redirects sliced gcode files to preview mode without loading", async () => {
    const onError = vi.fn();
    await handleModelDrop(["/x.gcode", "/y.gcode.3mf"], onError);
    expect(loadMock).not.toHaveBeenCalled();
    expect(onError).toHaveBeenCalledWith(
      "sliced G-code — drop it on the Preview canvas instead",
    );
  });

  it("rejects unsupported extensions but still loads supported siblings", async () => {
    loadMock.mockResolvedValue(undefined);
    const onError = vi.fn();
    await handleModelDrop(["/photo.png", "/a.stl"], onError);
    expect(onError).toHaveBeenCalledWith(
      "only .stl, .obj and .3mf files supported",
    );
    expect(loadMock).toHaveBeenCalledWith("/a.stl");
  });

  it("surfaces backend errors via onError and continues", async () => {
    loadMock
      .mockRejectedValueOnce("scene_load_mesh_from_path: bad mesh")
      .mockResolvedValueOnce(undefined);
    const onError = vi.fn();
    await handleModelDrop(["/bad.stl", "/good.stl"], onError);
    expect(onError).toHaveBeenCalledWith(
      "scene_load_mesh_from_path: bad mesh",
    );
    expect(loadMock).toHaveBeenCalledTimes(2);
  });
});
