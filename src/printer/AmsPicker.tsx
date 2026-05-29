// AMS-unit picker — tile strip showing 0..ams_max AMS units. Two
// layouts: a 0-vs-1 toggle for single-unit printers (Bambu A1
// mini's AMS Lite), or a 0..N tile row for stackable AMS hosts
// (Bambu A1's full AMS — up to 4 units). Hidden entirely when
// `amsMax == 0` (toolchangers).
//
// Used by both AddPrinterModal (initial-create flow) and
// PrinterSettingsModal (per-printer editing flow). Same component
// in both places so the visual is identical.

export interface AmsPickerProps {
  /** Maximum AMS units the printer profile supports. `0` hides the
   *  whole control; the caller is responsible for not rendering it
   *  for AMS-less profiles. */
  amsMax: number;
  /** User-facing AMS family name shown in the labels — `"AMS"`,
   *  `"AMS Lite"`, `"AMS 2 Pro"`. Drives the counter copy and the
   *  per-tile label. */
  amsType: string;
  /** Currently-selected AMS unit count. `0` means no AMS — slots
   *  collapse to a single direct-feed Ext spool. */
  value: number;
  onChange: (units: number) => void;
}

export function AmsPicker({ amsMax, amsType, value, onChange }: AmsPickerProps) {
  // A stored `value` can exceed `amsMax` if the printer profile's
  // AMS ceiling was lowered after this instance was created at a
  // higher count. Render tiles up to the larger of the two so the
  // current selection is always visible + revertable, and drop out
  // of the 0-vs-1 toggle layout (which can't represent >1) when it
  // happens.
  const overRange = value > amsMax;
  const isToggle = amsMax === 1 && !overRange;
  const maxTile = Math.max(amsMax, value);
  const totalSlots = value * 4 + 1;
  const counterText =
    value === 0
      ? "No AMS"
      : value === 1
        ? `1 × ${amsType} · 4 slots`
        : `${value} × ${amsType} · ${value * 4} slots`;
  return (
    <div className="apm-ams">
      <div className="apm-ams-head">
        <span className="apm-ams-label">{amsType} configuration</span>
        <span className="apm-ams-counter">
          {counterText}
          {value > 0 && (
            <span className="apm-ams-counter-dim">
              {" "}
              (+ ext spool = {totalSlots})
            </span>
          )}
        </span>
      </div>
      {isToggle ? (
        <div className="apm-ams-toggle">
          <button
            type="button"
            className={`apm-ams-tile ${value === 0 ? "active" : ""}`}
            onClick={() => onChange(0)}
          >
            <span className="apm-ams-tile-num">0</span>
            <span className="apm-ams-tile-label">No AMS</span>
          </button>
          <button
            type="button"
            className={`apm-ams-tile ${value === 1 ? "active" : ""}`}
            onClick={() => onChange(1)}
          >
            <span className="apm-ams-tile-num">1</span>
            <span className="apm-ams-tile-label">With {amsType}</span>
            <span className="apm-ams-tile-dots">
              {[0, 1, 2, 3].map((i) => (
                <span key={i} className="apm-ams-tile-dot" />
              ))}
            </span>
          </button>
        </div>
      ) : (
        <div className="apm-ams-row">
          {Array.from({ length: maxTile + 1 }, (_, i) => {
            const isOver = i > amsMax;
            return (
              <button
                key={i}
                type="button"
                className={`apm-ams-tile ${value === i ? "active" : ""}${isOver ? " over" : ""}`}
                // Over-range tiles aren't selectable — the profile no
                // longer supports that many units. Only the current
                // value stays clickable so it can render as active and
                // the user can see what's set before picking a valid
                // (lower) count.
                disabled={isOver && i !== value}
                onClick={() => onChange(i)}
                title={
                  isOver
                    ? `${i} × ${amsType} — exceeds this printer's current maximum of ${amsMax}`
                    : i === 0
                      ? `No ${amsType} installed`
                      : `${i} × ${amsType} (${i * 4} slots)`
                }
              >
                <span className="apm-ams-tile-num">{i}</span>
                <span className="apm-ams-tile-label">
                  {i === 0 ? "None" : `${i} unit${i > 1 ? "s" : ""}`}
                </span>
                {i > 0 && (
                  <span className="apm-ams-tile-dots">
                    {[0, 1, 2, 3].map((d) => (
                      <span key={d} className="apm-ams-tile-dot" />
                    ))}
                  </span>
                )}
              </button>
            );
          })}
        </div>
      )}
      <div className="apm-name-hint">
        {value === 0
          ? "Filaments load directly into the extruder via an external spool. You can attach an AMS later from the printer's settings."
          : `Each ${amsType} holds 4 spools and feeds them to the toolhead automatically. You'll route project materials to slots once a plate exists.`}
      </div>
      {overRange && (
        <div className="apm-name-hint error">
          This printer now supports at most {amsMax} {amsType} unit
          {amsMax === 1 ? "" : "s"}. Pick {amsMax} or fewer to apply.
        </div>
      )}
    </div>
  );
}
