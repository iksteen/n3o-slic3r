// Point input — two number fields (X, Y) side by side for a libslic3r
// `coPoint` option. A single `coPoint` (bed_mesh_min) serializes "x,y"; one
// element of a `coPoints` vector (an extruder_offset entry) serializes "XxY".
// `separator` picks which. Vector `coPoints` groups (polygons) aren't handled.

import { useEffect, useState } from "react";
import type { OptionSummary } from "../types";

export interface PointInputProps {
  schema: OptionSummary;
  /** Serialized point, or null to fall back to the default. */
  value: string | null;
  onChange: (next: string) => void;
  disabled?: boolean;
  /** Coordinate separator — "," for a single coPoint, "x" for a coPoints
   *  element. Both are what libslic3r emits for the respective type. */
  separator?: string;
}

const splitXY = (v: string, sep: string): [string, string] => {
  const [x = "", y = ""] = v.split(sep);
  return [x.trim(), y.trim()];
};

export function PointInput({
  schema,
  value,
  onChange,
  disabled = false,
  separator = ",",
}: PointInputProps): React.JSX.Element {
  const initial = value ?? defaultPoint(schema, separator);
  const [[x, y], setXY] = useState<[string, string]>(
    splitXY(initial, separator),
  );

  // Re-sync when the upstream value changes (cascade refresh, reset).
  useEffect(() => {
    setXY(splitXY(value ?? defaultPoint(schema, separator), separator));
  }, [value, schema, separator]);

  const commit = (nx: string, ny: string): void => {
    setXY([nx, ny]);
    onChange(`${nx.trim()}${separator}${ny.trim()}`);
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

function defaultPoint(schema: OptionSummary, separator: string): string {
  const dv = schema.default_value;
  // A single coPoint carries its "x,y" default as a scalar; a coPoints element
  // has a vector default (per-extruder) — fall back to origin in either form.
  if (dv?.kind === "scalar") return dv.value;
  return `0${separator}0`;
}
