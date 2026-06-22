import type { ReactNode } from "react";

/**
 * The viewport's axis indicator + input-binding hints. Shared by the Three.js
 * (`ViewportCanvas`) and wgpu (`ViewportChrome`) viewports: the X/Y/Z axes are
 * identical, but each passes its own `hints` because their input bindings differ
 * (the Three.js viewport pans; the wgpu one doesn't yet).
 */
export function ViewportLegend({ hints }: { hints: ReactNode }) {
  return (
    <div className="gizmo-hint pointer-events-none">
      <span className="axes" aria-label="Axes">
        <span className="axis axis-x">X</span>
        <span className="axis axis-y">Y</span>
        <span className="axis axis-z">Z</span>
      </span>
      <span className="gizmo-hint-sep" aria-hidden>
        ·
      </span>
      {hints}
    </div>
  );
}
