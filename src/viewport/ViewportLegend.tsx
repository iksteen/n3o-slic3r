import type { ReactNode } from "react";

/**
 * The viewport's axis indicator + input-binding hints. When `prompt` is set the
 * hint is an action the viewport is waiting on (e.g. "Click a face to lay it
 * flat"), so the legend lights up (accent border + bright, bold hint) instead of
 * reading as a quiet nav reminder.
 */
export function ViewportLegend({ hints, prompt = false }: { hints: ReactNode; prompt?: boolean }) {
  return (
    <div className={`gizmo-hint pointer-events-none${prompt ? " gizmo-hint--prompt" : ""}`}>
      <span className="axes" aria-label="Axes">
        <span className="axis axis-x">X</span>
        <span className="axis axis-y">Y</span>
        <span className="axis axis-z">Z</span>
      </span>
      <span className="gizmo-hint-sep" aria-hidden>
        ·
      </span>
      <span className="gizmo-hint-text">{hints}</span>
    </div>
  );
}
