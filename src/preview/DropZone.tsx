// Webview drag-drop overlay + the preview-mode file routing.
//
// The overlay subscribes to Tauri's webview-level drag-drop events.
// The browser's HTML5 dragenter/drop API doesn't expose the OS file
// path (only a `File` object), but Tauri's wrapper does — required
// so callers can hand paths straight to backend commands that read
// off disk inside the worker thread.
//
// Visual: invisible until a drag enters; on enter renders a
// dashed-border overlay across the parent positioning context
// with the caller's prompt. Reset on drop OR leave.
//
// The drag-drop event is webview-global, not per-element, so mount
// at most one DropZone per layout mode: PreviewWorkspace routes
// gcode files via `handleDrop`; the prepare canvas mounts
// ModelDropZone (mesh import) on the same shell.

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
  prompt: string;
  onDrop: (paths: string[]) => void;
}

export function DropZone({ prompt, onDrop }: DropZoneProps) {
  const [dragOver, setDragOver] = useState(false);
  // Ref so the long-lived event listener doesn't capture a stale
  // callback on re-render.
  const onDropRef = useRef(onDrop);
  onDropRef.current = onDrop;

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
          const paths = payload.paths ?? [];
          if (paths.length === 0) return;
          onDropRef.current(paths);
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
      <div className="preview-dropzone-prompt">{prompt}</div>
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

export function formatError(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  return String(e);
}
