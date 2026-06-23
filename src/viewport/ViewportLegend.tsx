import type { ReactNode } from "react";
import type { AxisView } from "./cameraControl";

const AXIS_VIEW: { axis: AxisView; label: string; title: string }[] = [
  { axis: "x", label: "X", title: "Front view (look along +Y)" },
  { axis: "y", label: "Y", title: "Side view (look along −X)" },
  { axis: "z", label: "Z", title: "Top view (look straight down)" },
];

/**
 * The viewport's axis indicator + input-binding hints. The X/Y/Z chips are
 * clickable — each snaps the camera to that axis-aligned view (`onAxis`). When
 * `prompt` is set the hint is an action the viewport is waiting on (e.g. "Click
 * a face to lay it flat"), so the legend lights up (accent border + bright, bold
 * hint) instead of reading as a quiet nav reminder.
 */
export function ViewportLegend({
  hints,
  prompt = false,
  onAxis,
}: {
  hints: ReactNode;
  prompt?: boolean;
  onAxis?: (axis: AxisView) => void;
}) {
  return (
    <div className={`gizmo-hint pointer-events-none${prompt ? " gizmo-hint--prompt" : ""}`}>
      <span className="axes" aria-label="Axis views">
        {AXIS_VIEW.map(({ axis, label, title }) => (
          <button
            key={axis}
            type="button"
            className={`axis axis-${axis} axis-btn pointer-events-auto`}
            onClick={() => onAxis?.(axis)}
            title={title}
            aria-label={title}
          >
            {label}
          </button>
        ))}
      </span>
      <span className="gizmo-hint-sep" aria-hidden>
        ·
      </span>
      <span className="gizmo-hint-text">{hints}</span>
    </div>
  );
}
