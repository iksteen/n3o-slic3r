import type { ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ViewportLegend } from "./ViewportLegend";
import type { SceneObject } from "./types";

/**
 * Chrome for the wgpu viewport: the gizmo-mode toggle, placing tools (lay-flat,
 * align X/Y, match-face), clone, plate-level actions (arrange, auto-orient), and
 * the axis/input legend.
 */
type GizmoMode = "none" | "move" | "rotate" | "scale";
type Tool = "none" | "layflat" | "alignX" | "alignY" | "facematch" | "clone";

export function ViewportChrome({
  leading,
  objects,
  selectedIds,
  gizmoMode,
  onGizmoMode,
  tool,
  onTool,
  onClone,
  faceMatchRefSet = false,
}: {
  leading: ReactNode;
  objects: SceneObject[];
  selectedIds: number[];
  gizmoMode: GizmoMode;
  onGizmoMode: (mode: GizmoMode) => void;
  tool: Tool;
  onTool: (tool: Tool) => void;
  onClone: () => void;
  /** Match-face: the reference face has been clicked (awaiting the target). */
  faceMatchRefSet?: boolean;
}) {
  const hasObjects = objects.length > 0;

  const runArrange = () => {
    void invoke("scene_auto_arrange").catch((e) => console.error("arrange failed", e));
  };
  // Align X/Y: with a selection, rotate it about Z so its dominant line is
  // parallel to the axis immediately. With no selection, arm pick-to-align — the
  // next clicked object's group is aligned (handled in WgpuViewport).
  const runAlign = (axis: "alignX" | "alignY") => {
    if (selectedIds.length > 0) {
      const a = axis === "alignX" ? "X" : "Y";
      void invoke("scene_object_align_axis", { ids: selectedIds, axis: a }).catch((e) =>
        console.error("align failed", e),
      );
      return;
    }
    if (hasObjects) onTool(axis);
  };
  const runAutoOrient = async () => {
    // Orient the selection, or everything on the plate if nothing's selected —
    // each object (or group, as a unit) individually, not the whole set as one
    // rigid mesh. The backend treats one `ids` call as a single unit, so fan out:
    // one call per group (expanded to its members) + one per solo object.
    const targets = selectedIds.length ? selectedIds : objects.map((o) => o.id);
    const seenGroups = new Set<string>();
    for (const id of targets) {
      const obj = objects.find((o) => o.id === id);
      if (!obj) continue;
      if (obj.group) {
        if (seenGroups.has(obj.group)) continue; // group already oriented
        seenGroups.add(obj.group);
      }
      try {
        await invoke("scene_object_auto_orient", { ids: [id], expandGroups: true });
      } catch (e) {
        console.error("auto-orient failed", e);
      }
    }
  };

  const btn = (enabled: boolean) =>
    `px-2 py-1.5 ${enabled ? "hover:bg-neutral-700/60" : "opacity-40 cursor-not-allowed"}`;

  return (
    <>
      <div className="absolute top-2 left-2 flex gap-2 pointer-events-auto" style={{ zIndex: 10 }}>
        {leading}
        <div className="bg-neutral-800/90 text-neutral-100 text-xs rounded shadow flex overflow-hidden">
          <button
            type="button"
            className={`px-2 py-1.5 ${gizmoMode === "move" ? "bg-neutral-700" : "hover:bg-neutral-700/60"}`}
            onClick={() => onGizmoMode("move")}
            title="Move — drag the axis/plane handles"
            aria-label="Move gizmo"
            aria-pressed={gizmoMode === "move"}
          >
            <svg width="14" height="14" viewBox="0 0 14 14" fill="none" aria-hidden="true">
              <path
                d="M7 1.5v11M1.5 7h11M7 1.5 5.3 3.4M7 1.5 8.7 3.4M7 12.5 5.3 10.6M7 12.5 8.7 10.6M1.5 7 3.4 5.3M1.5 7 3.4 8.7M12.5 7 10.6 5.3M12.5 7 10.6 8.7"
                stroke="currentColor"
                strokeWidth="1.1"
                strokeLinecap="round"
                strokeLinejoin="round"
              />
            </svg>
          </button>
          <button
            type="button"
            className={`px-2 py-1.5 ${gizmoMode === "rotate" ? "bg-neutral-700" : "hover:bg-neutral-700/60"}`}
            onClick={() => onGizmoMode("rotate")}
            title="Rotate — drag the axis rings"
            aria-label="Rotate gizmo"
            aria-pressed={gizmoMode === "rotate"}
          >
            <svg width="14" height="14" viewBox="0 0 14 14" fill="none" aria-hidden="true">
              <path
                d="M11.5 7a4.5 4.5 0 1 1-1.32-3.18"
                stroke="currentColor"
                strokeWidth="1.2"
                strokeLinecap="round"
                strokeLinejoin="round"
              />
              <path
                d="M10.4 1.2v2.7H7.7"
                stroke="currentColor"
                strokeWidth="1.2"
                strokeLinecap="round"
                strokeLinejoin="round"
              />
            </svg>
          </button>
          <button
            type="button"
            className={`px-2 py-1.5 ${gizmoMode === "scale" ? "bg-neutral-700" : "hover:bg-neutral-700/60"}`}
            onClick={() => onGizmoMode("scale")}
            title="Scale — drag the axis/plane handles"
            aria-label="Scale gizmo"
            aria-pressed={gizmoMode === "scale"}
          >
            <svg width="14" height="14" viewBox="0 0 14 14" fill="none" aria-hidden="true">
              <path
                d="M2 12 12 2M12 2H8.5M12 2v3.5"
                stroke="currentColor"
                strokeWidth="1.2"
                strokeLinecap="round"
                strokeLinejoin="round"
              />
              <rect x="1.5" y="9.5" width="3" height="3" rx="0.4" fill="currentColor" />
            </svg>
          </button>
        </div>
        <div className="bg-neutral-800/90 text-neutral-100 text-xs rounded shadow flex overflow-hidden">
          <button
            type="button"
            disabled={!hasObjects}
            className={btn(hasObjects)}
            onClick={runArrange}
            title="Arrange — auto-pack the plate"
            aria-label="Auto-arrange the plate"
          >
            <svg width="14" height="14" viewBox="0 0 14 14" fill="none" aria-hidden="true">
              <rect x="1.5" y="1.5" width="4.5" height="4.5" rx="0.6" stroke="currentColor" strokeWidth="1.2" />
              <rect x="8" y="1.5" width="4.5" height="4.5" rx="0.6" stroke="currentColor" strokeWidth="1.2" />
              <rect x="1.5" y="8" width="4.5" height="4.5" rx="0.6" stroke="currentColor" strokeWidth="1.2" />
              <rect x="8" y="8" width="4.5" height="4.5" rx="0.6" stroke="currentColor" strokeWidth="1.2" />
            </svg>
          </button>
          <button
            type="button"
            disabled={!hasObjects}
            className={btn(hasObjects)}
            onClick={runAutoOrient}
            title={selectedIds.length ? "Auto orient selection" : "Auto orient all objects"}
            aria-label="Auto orient"
          >
            <svg width="14" height="14" viewBox="0 0 14 14" fill="none" aria-hidden="true">
              <path
                d="M7 1.6v6.6M4.2 5.4 7 8.2l2.8-2.8M2.2 12h9.6"
                stroke="currentColor"
                strokeWidth="1.4"
                strokeLinecap="round"
                strokeLinejoin="round"
              />
            </svg>
          </button>
          <button
            type="button"
            disabled={!hasObjects}
            className={tool === "layflat" ? "px-2 py-1.5 bg-neutral-700" : btn(hasObjects)}
            onClick={() => hasObjects && onTool("layflat")}
            title="Lay flat — click a face to lay it on the plate"
            aria-label="Lay flat on face"
            aria-pressed={tool === "layflat"}
          >
            <svg width="14" height="14" viewBox="0 0 14 14" fill="none" aria-hidden="true">
              {/* a face (parallelogram) on the bed + a down arrow onto it */}
              <path d="M2 9.4 7 6.6l5 2.8-5 2.6z" stroke="currentColor" strokeWidth="1.3" strokeLinejoin="round" />
              <path d="M7 1.3v3.4M5.4 3.1 7 4.7l1.6-1.6" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round" strokeLinejoin="round" />
            </svg>
          </button>
          <button
            type="button"
            disabled={!hasObjects}
            className={tool === "alignY" ? "px-2 py-1.5 bg-neutral-700" : btn(hasObjects)}
            onClick={() => runAlign("alignY")}
            title={selectedIds.length ? "Align selection to Y" : "Align — click an object to align to Y"}
            aria-label="Align to Y"
            aria-pressed={tool === "alignY"}
          >
            <svg width="14" height="14" viewBox="0 0 14 14" fill="none" aria-hidden="true">
              <path d="M7 12.5v-11M7 1.5 5.2 3.3M7 1.5 8.8 3.3" stroke="currentColor" strokeWidth="1.2" strokeLinecap="round" strokeLinejoin="round" />
              <text x="8.5" y="11" fontSize="5" fill="currentColor" stroke="none">Y</text>
            </svg>
          </button>
          <button
            type="button"
            disabled={!hasObjects}
            className={tool === "alignX" ? "px-2 py-1.5 bg-neutral-700" : btn(hasObjects)}
            onClick={() => runAlign("alignX")}
            title={selectedIds.length ? "Align selection to X" : "Align — click an object to align to X"}
            aria-label="Align to X"
            aria-pressed={tool === "alignX"}
          >
            <svg width="14" height="14" viewBox="0 0 14 14" fill="none" aria-hidden="true">
              <path d="M1.5 7h11M12.5 7 10.7 5.2M12.5 7 10.7 8.8" stroke="currentColor" strokeWidth="1.2" strokeLinecap="round" strokeLinejoin="round" />
              <text x="3" y="5.5" fontSize="5" fill="currentColor" stroke="none">X</text>
            </svg>
          </button>
          <button
            type="button"
            disabled={!hasObjects}
            className={tool === "facematch" ? "px-2 py-1.5 bg-neutral-700" : btn(hasObjects)}
            onClick={() => hasObjects && onTool("facematch")}
            title="Match face — click a reference face, then the face to align to it"
            aria-label="Match a face to a reference face on another object"
            aria-pressed={tool === "facematch"}
          >
            <svg width="14" height="14" viewBox="0 0 14 14" fill="none" aria-hidden="true">
              {/* two faces (vertical bars) + an arrow bringing the right one onto the left reference */}
              <path d="M3 2.5v9M11 2.5v9M10 7H5.5M7 5.5 5.5 7 7 8.5" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round" strokeLinejoin="round" />
            </svg>
          </button>
          <button
            type="button"
            disabled={!hasObjects}
            className={tool === "clone" ? "px-2 py-1.5 bg-neutral-700" : btn(hasObjects)}
            onClick={() => hasObjects && onClone()}
            title={selectedIds.length ? "Clone selection" : "Clone — click an object to clone"}
            aria-label="Clone objects"
            aria-pressed={tool === "clone"}
          >
            <svg width="14" height="14" viewBox="0 0 14 14" fill="none" aria-hidden="true">
              {/* two overlapping rounded rects — the copy/duplicate glyph */}
              <rect x="5" y="5" width="7" height="7" rx="1.2" stroke="currentColor" strokeWidth="1.3" />
              <path d="M9 5V3.2A1.2 1.2 0 0 0 7.8 2H3.2A1.2 1.2 0 0 0 2 3.2v4.6A1.2 1.2 0 0 0 3.2 9H5" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round" strokeLinejoin="round" />
            </svg>
          </button>
        </div>
      </div>
      <ViewportLegend hints={toolHint(tool, faceMatchRefSet)} prompt={tool !== "none"} />
    </>
  );
}

function toolHint(tool: Tool, faceMatchRefSet: boolean): string {
  switch (tool) {
    case "layflat":
      return "Click a face to lay it on the plate · Esc to cancel";
    case "alignX":
      return "Click an object to align it to X · Esc to cancel";
    case "alignY":
      return "Click an object to align it to Y · Esc to cancel";
    case "facematch":
      return faceMatchRefSet
        ? "Now click the face to match to the reference · Esc to cancel"
        : "Select the reference face · Esc to cancel";
    case "clone":
      return "Click an object to clone · Esc to cancel";
    default:
      return "LMB rotate · RMB pan · scroll zoom";
  }
}
