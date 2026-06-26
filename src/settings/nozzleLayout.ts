// NozzlePicker layout routing.
//
// The picker chips render in two shapes:
//
//   - **Inline** (1 or 2 extruders): chips ride in the same
//     `.sp-config-row` as the Printer + Bed chips. Reads as a
//     compact per-print config strip.
//   - **Own rows** (3+ extruders): chips move to dedicated
//     `.sp-config-nozzles` rows below the Printer/Bed row, max 4
//     chips per row. The first nozzle row carries the "Nozzles"
//     label; subsequent rows are unlabeled wraps.
//
// `chunkExtruders` produces the per-row index arrays the host
// renders. Pure helper so the routing rule is testable without a
// DOM and stays the single source of truth for "where does each
// chip land."

export const NOZZLES_INLINE_THRESHOLD = 2;
export const NOZZLES_PER_ROW = 4;

/** Decide whether extruders ride inline with Printer + Bed or get
 *  their own rows. `extruderCount` is `PrinterInstance.extruders.length`. */
export function nozzlesInline(extruderCount: number): boolean {
  return extruderCount > 0 && extruderCount <= NOZZLES_INLINE_THRESHOLD;
}

/** Split extruder indices into rows of at most `NOZZLES_PER_ROW`.
 *  Returns an empty list when `extruderCount` is 0 OR when the chips
 *  ride inline — the host already renders the inline case from the
 *  flat index list. */
export function chunkExtruders(extruderCount: number): number[][] {
  if (extruderCount <= 0 || nozzlesInline(extruderCount)) return [];
  const rows: number[][] = [];
  for (let i = 0; i < extruderCount; i += NOZZLES_PER_ROW) {
    rows.push(
      Array.from(
        { length: Math.min(NOZZLES_PER_ROW, extruderCount - i) },
        (_, j) => i + j,
      ),
    );
  }
  return rows;
}
