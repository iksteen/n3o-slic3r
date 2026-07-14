// Drag-drop loader for model files (.stl / .obj / .3mf) on the
// prepare canvas. Mounts the shared DropZone shell and routes each
// dropped path through `loadModelFromPath`, so a drop behaves
// exactly like the Objects panel's "Add model…" picker — .3mf loads
// geometry only, never the project settings.

import { useState } from "react";

import { DropZone, formatError } from "../preview/DropZone";
import { loadModelFromPath } from "./objectCommands";

export function ModelDropZone() {
  const [error, setError] = useState<string | null>(null);
  return (
    <>
      <DropZone
        prompt="Drop .stl, .obj or .3mf here"
        onDrop={(paths) => {
          setError(null);
          void handleModelDrop(paths, setError);
        }}
      />
      {error && (
        <div className="preview-drop-error" role="alert">
          {error}
          <button
            type="button"
            className="preview-drop-error-dismiss"
            onClick={() => setError(null)}
            aria-label="Dismiss"
          >
            ×
          </button>
        </div>
      )}
    </>
  );
}

/** Route dropped files to the model loader, sequentially so multi-file
 *  drops land in a stable order. Exported for unit-test access. */
export async function handleModelDrop(
  paths: string[],
  onError: (message: string) => void,
): Promise<void> {
  for (const path of paths) {
    const lower = path.toLowerCase();
    if (lower.endsWith(".gcode") || lower.endsWith(".gcode.3mf")) {
      onError("sliced G-code — drop it on the Preview canvas instead");
      continue;
    }
    if (!/\.(stl|obj|3mf)$/.test(lower)) {
      onError("only .stl, .obj and .3mf files supported");
      continue;
    }
    try {
      await loadModelFromPath(path);
    } catch (e) {
      onError(formatError(e));
    }
  }
}
