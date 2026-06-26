// DropZone routing logic.
//
// The React subscription bookkeeping (Tauri webview listener,
// drag-over visual) needs jsdom + a fake Tauri runtime — covered
// by the Playwright smoke. Here we pin the pure routing:
// extension → command, error path, error formatting.

import { afterEach, describe, expect, it, vi } from "vitest";

import { handleDrop } from "../DropZone";

// Hoist-aware mock: vitest evaluates `vi.mock` factory before the
// test module imports run, so the mocks are live by the time
// DropZone.tsx's `import { previewLoad, ... }` resolves.
vi.mock("../invokes", () => ({
  previewLoad: vi.fn(),
  previewLoadGcode3mf: vi.fn(),
}));

import { previewLoad, previewLoadGcode3mf } from "../invokes";

afterEach(() => {
  vi.resetAllMocks();
});

describe("handleDrop", () => {
  it("routes .gcode to preview_load and forwards the response", async () => {
    const fake = { handle: 1, layer_count: 5 } as never;
    (previewLoad as ReturnType<typeof vi.fn>).mockResolvedValue(fake);
    const onLoaded = vi.fn();
    const onError = vi.fn();
    handleDrop("/tmp/foo.gcode", onLoaded, onError);
    await flushPromises();
    expect(previewLoad).toHaveBeenCalledWith("/tmp/foo.gcode");
    expect(onLoaded).toHaveBeenCalledWith({ preview: fake, sliced: null });
    expect(onError).not.toHaveBeenCalled();
  });

  it("routes .gcode.3mf to preview_load_gcode_3mf and forwards both preview + sliced", async () => {
    const preview = { handle: 2, layer_count: 9 } as never;
    const fake = {
      preview,
      plate_count: 2,
      plate_metadata: null,
      thumbnail_png: null,
    };
    (previewLoadGcode3mf as ReturnType<typeof vi.fn>).mockResolvedValue(fake);
    const onLoaded = vi.fn();
    const onError = vi.fn();
    handleDrop("/tmp/foo.gcode.3mf", onLoaded, onError);
    await flushPromises();
    expect(previewLoadGcode3mf).toHaveBeenCalledWith("/tmp/foo.gcode.3mf");
    expect(onLoaded).toHaveBeenCalledWith({ preview, sliced: fake });
  });

  it("rejects unsupported extensions before invoking anything", () => {
    const onLoaded = vi.fn();
    const onError = vi.fn();
    handleDrop("/tmp/photo.png", onLoaded, onError);
    expect(previewLoad).not.toHaveBeenCalled();
    expect(previewLoadGcode3mf).not.toHaveBeenCalled();
    expect(onError).toHaveBeenCalledWith(
      "only .gcode and .gcode.3mf files supported",
    );
  });

  it("matches .gcode.3mf before .gcode (compound extension takes precedence)", async () => {
    const fake = {
      preview: { handle: 3 },
      plate_count: 1,
      plate_metadata: null,
      thumbnail_png: null,
    } as never;
    (previewLoadGcode3mf as ReturnType<typeof vi.fn>).mockResolvedValue(fake);
    handleDrop("/tmp/bundle.gcode.3mf", vi.fn(), vi.fn());
    await flushPromises();
    expect(previewLoadGcode3mf).toHaveBeenCalled();
    expect(previewLoad).not.toHaveBeenCalled();
  });

  it("is case-insensitive for the extension match", async () => {
    const fake = { handle: 4 } as never;
    (previewLoad as ReturnType<typeof vi.fn>).mockResolvedValue(fake);
    handleDrop("/tmp/UPPERCASE.GCODE", vi.fn(), vi.fn());
    await flushPromises();
    expect(previewLoad).toHaveBeenCalled();
  });

  it("surfaces backend errors via onError", async () => {
    (previewLoad as ReturnType<typeof vi.fn>).mockRejectedValue(
      "preview_load: file not found",
    );
    const onError = vi.fn();
    handleDrop("/missing.gcode", vi.fn(), onError);
    await flushPromises();
    expect(onError).toHaveBeenCalledWith("preview_load: file not found");
  });
});

function flushPromises(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 0));
}
