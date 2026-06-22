import type { ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ViewportLegend } from "./ViewportLegend";

/**
 * Chrome for the wgpu viewport: the mode toggle, plate-level actions (arrange,
 * auto-orient), and the axis/input legend. The gizmo-mode, lay-flat / align /
 * face-align and clone controls stay in the Three.js `ViewportCanvas` for now —
 * they depend on the wgpu gizmo + face-picking, which aren't built yet.
 */
export function ViewportChrome({
  leading,
  objectIds,
  selectedIds,
}: {
  leading: ReactNode;
  objectIds: number[];
  selectedIds: number[];
}) {
  const hasObjects = objectIds.length > 0;

  const runArrange = () => {
    void invoke("scene_auto_arrange").catch((e) => console.error("arrange failed", e));
  };
  const runAutoOrient = () => {
    // Orient the selection, or everything on the plate if nothing's selected.
    const ids = selectedIds.length ? selectedIds : objectIds;
    if (ids.length === 0) return;
    void invoke("scene_object_auto_orient", { ids }).catch((e) =>
      console.error("auto-orient failed", e),
    );
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
        </div>
      </div>
      <ViewportLegend hints="LMB rotate · RMB pan · scroll zoom" />
    </>
  );
}
