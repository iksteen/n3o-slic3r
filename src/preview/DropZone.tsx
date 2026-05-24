// Drag-drop loader for `.gcode` and `.gcode.3mf` files (PR-6-14).
//
// Subscribes to Tauri's webview-level drag-drop events. The
// browser's HTML5 dragenter/drop API doesn't expose the OS file
// path (only a `File` object), but Tauri's wrapper does — required
// so we can hand the path straight to preview_load{,_gcode_3mf}
// which read off disk inside the worker thread.
//
// Visual: invisible until a drag enters; on enter renders a
// dashed-border overlay across the parent positioning context
// with a "Drop here" prompt. Reset on drop OR leave.
//
// Scope: PreviewWorkspace mounts this only in preview mode. The
// 3D viewport's drag-drop flow (mesh import) is unrelated and
// still uses the file-open dialog from PR-2-3.

import { useEffect, useRef, useState } from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import type { UnlistenFn } from "@tauri-apps/api/event";

import { previewLoad, previewLoadGcode3mf } from "./invokes";
import type {
  PreviewLoadGcode3mfResponse,
  PreviewLoadResponse,
} from "./types";

/** What the DropZone hands back when a file successfully loads.
 * `sliced` is `null` for raw `.gcode` drops and populated with
 * the container metadata (plate count, plate JSON, thumbnail) for
 * `.gcode.3mf` drops. */
export interface DroppedPreview {
  preview: PreviewLoadResponse;
  sliced: PreviewLoadGcode3mfResponse | null;
}

export interface DropZoneProps {
  onLoaded: (result: DroppedPreview) => void;
  onError: (message: string) => void;
}

export function DropZone({ onLoaded, onError }: DropZoneProps) {
  const [dragOver, setDragOver] = useState(false);
  // Refs so the long-lived event listener doesn't capture stale
  // callbacks on re-render.
  const onLoadedRef = useRef(onLoaded);
  const onErrorRef = useRef(onError);
  onLoadedRef.current = onLoaded;
  onErrorRef.current = onError;

  useEffect(() => {
    let cancelled = false;
    let unlisten: UnlistenFn | null = null;
    void (async () => {
      const webview = getCurrentWebview();
      const handle = await webview.onDragDropEvent((event) => {
        if (cancelled) return;
        const payload = event.payload;
        if (payload.type === "enter" || payload.type === "over") {
          setDragOver(true);
        } else if (payload.type === "leave") {
          setDragOver(false);
        } else if (payload.type === "drop") {
          setDragOver(false);
          const path = payload.paths?.[0];
          if (!path) return;
          handleDrop(path, onLoadedRef.current, onErrorRef.current);
        }
      });
      if (cancelled) {
        handle();
      } else {
        unlisten = handle;
      }
    })();
    return () => {
      cancelled = true;
      if (unlisten) unlisten();
    };
  }, []);

  if (!dragOver) return null;
  return (
    <div className="preview-dropzone-overlay" role="presentation">
      <div className="preview-dropzone-prompt">
        Drop .gcode or .gcode.3mf here
      </div>
    </div>
  );
}

/** Route a dropped file to the right Tauri command by extension.
 * Exported for unit-test access — the React component above is
 * mostly subscription bookkeeping. */
export function handleDrop(
  path: string,
  onLoaded: (result: DroppedPreview) => void,
  onError: (message: string) => void,
): void {
  const lower = path.toLowerCase();
  if (lower.endsWith(".gcode.3mf")) {
    void previewLoadGcode3mf(path)
      .then((sliced) => onLoaded({ preview: sliced.preview, sliced }))
      .catch((e: unknown) => onError(formatError(e)));
  } else if (lower.endsWith(".gcode")) {
    void previewLoad(path)
      .then((preview) => onLoaded({ preview, sliced: null }))
      .catch((e: unknown) => onError(formatError(e)));
  } else {
    onError("only .gcode and .gcode.3mf files supported");
  }
}

function formatError(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  return String(e);
}
