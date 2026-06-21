// Point input — two number fields (X, Y) side by side for a libslic3r
// `coPoint` option. Serializes as "x,y" (the engine's Point format, e.g.
// "0.5,0.5"). Vector `coPoints` (geometry/polygons) are not handled here.

import { useEffect, useState } from "react";
import type { OptionSummary } from "../types";

export interface PointInputProps {
  schema: OptionSummary;
  /** Serialized "x,y", or null to fall back to the default. */
  value: string | null;
  onChange: (next: string) => void;
  disabled?: boolean;
}

const splitXY = (v: string): [string, string] => {
  const [x = "", y = ""] = v.split(",");
  return [x.trim(), y.trim()];
};

export function PointInput({
  schema,
  value,
  onChange,
  disabled = false,
}: PointInputProps): React.JSX.Element {
  const initial = value ?? defaultPoint(schema);
  const [[x, y], setXY] = useState<[string, string]>(splitXY(initial));

  // Re-sync when the upstream value changes (cascade refresh, reset).
  useEffect(() => {
    setXY(splitXY(value ?? defaultPoint(schema)));
  }, [value, schema]);

  const commit = (nx: string, ny: string): void => {
    setXY([nx, ny]);
    onChange(`${nx.trim()},${ny.trim()}`);
  };

  return (
    <div className="point-input">
      <label className="point-axis">
        <span>X</span>
        <input
          className="val-input"
          type="number"
          value={x}
          disabled={disabled}
          onChange={(e) => commit(e.target.value, y)}
        />
      </label>
      <label className="point-axis">
        <span>Y</span>
        <input
          className="val-input"
          type="number"
          value={y}
          disabled={disabled}
          onChange={(e) => commit(x, e.target.value)}
        />
      </label>
      {schema.sidetext && <span className="val-unit">{schema.sidetext}</span>}
    </div>
  );
}

function defaultPoint(schema: OptionSummary): string {
  const dv = schema.default_value;
  if (dv?.kind === "scalar") return dv.value;
  return "0,0";
}
