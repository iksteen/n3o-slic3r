// Per-extruder ("toolhead") settings for one extruder index. The options
// are Printer-bucket vectors (retraction, nozzle, Z-hop, …) with one entry
// per toolhead; this renders entry `extruderIndex` as a scalar input.
//
// Layout mirrors Orca's single Extruder page: the optgroups (Basic
// information, Layer height limits, Position, Retraction, Z-Hop, …) become
// sub-headers within one scroll. A two-member optgroup (Layer height limits →
// Min/Max) renders as a single dual-entry line instead of a header + two
// context-free rows. extruder_offset is a coPoints element (per-extruder X/Y),
// rendered as a dual Point input like bed_mesh_min.
//
// Storage: `config_overrides` holds the whole serialized vector for the
// key. Editing one extruder's field reads the current vector (override if
// set, else resolved), swaps this index, and writes the full vector back —
// so the existing flat-map override + `!important` cascade injection apply
// unchanged. Resetting one index restores it to the resolved value, and
// clears the key entirely once every entry matches resolved again.

import { Field, PointInput } from "../settings/inputs";
import { renderScalarInput } from "../settings/renderScalarInput";
import {
  optionTypeKind,
  scalarElementKind,
  type PrinterAwareOptionSummary,
} from "../settings/types";
import {
  elemOverridden,
  resolvedVec,
  setVecElem,
  vecElem,
} from "./vectorOverride";
import { PairedInputRow } from "./PairedInputRow";

export interface ExtruderSettingsSectionProps {
  /** Which toolhead (0-based) this tab edits. */
  extruderIndex: number;
  settings: PrinterAwareOptionSummary[];
  overrides: Record<string, string>;
  resolved: Record<string, string>;
  onSet: (key: string, value: string) => void;
  onClear: (key: string) => void;
}

export function ExtruderSettingsSection({
  extruderIndex,
  settings,
  overrides,
  resolved,
  onSet,
  onClear,
}: ExtruderSettingsSectionProps): React.JSX.Element {
  const visible = settings.filter((s) => !s.hidden);

  // Group by optgroup (extruder keys carry it as their `category`), preserving
  // scraped display order.
  const groups: { name: string | null; rows: PrinterAwareOptionSummary[] }[] =
    [];
  for (const s of visible) {
    const name = s.category ?? null;
    const last = groups.find((g) => g.name === name);
    if (last) last.rows.push(s);
    else groups.push({ name, rows: [s] });
  }

  const elemValueOf = (key: string): string | null =>
    vecElem(overrides, resolved, key, extruderIndex);
  const overriddenOf = (key: string): boolean =>
    elemOverridden(overrides, resolved, key, extruderIndex);
  const setElem = (key: string, next: string): void =>
    setVecElem(overrides, resolved, key, extruderIndex, next, onSet, onClear);
  const resetElem = (key: string): void =>
    setElem(key, resolvedVec(resolved, key)[extruderIndex] ?? "");

  const renderRow = (schema: PrinterAwareOptionSummary): React.JSX.Element => {
    const key = schema.key;
    const kind = optionTypeKind(schema);
    const overridden = overriddenOf(key);
    const elemValue = elemValueOf(key);
    const setValue = (next: string): void => setElem(key, next);

    const resetButton = overridden ? (
      <button
        type="button"
        className="reset-btn"
        title="Reset to printer default"
        aria-label={`Reset ${key} for extruder ${extruderIndex + 1}`}
        onClick={() => resetElem(key)}
      >
        ↺
      </button>
    ) : null;

    // extruder_offset is a coPoints element ("XxY"); everything else is a
    // per-extruder scalar vector.
    const control =
      kind === "vector-point" ? (
        <PointInput
          schema={schema}
          value={elemValue}
          onChange={setValue}
          separator="x"
        />
      ) : (
        renderScalarInput(scalarElementKind(kind), schema, elemValue, setValue, false)
      );

    return (
      <Field
        key={key}
        schema={schema}
        value={elemValue}
        onChange={setValue}
        resetButton={resetButton}
        winningLayer={overridden ? "user" : "cascade"}
      >
        {control}
      </Field>
    );
  };

  return (
    <div className="machine-settings">
      {groups.map((g) => {
        // ponytail: a 2-member optgroup is a Min/Max-style pair (only "Layer
        // height limits" today) — render it as one dual-entry line. Revisit if
        // a 2-member optgroup ever isn't a natural pair.
        const asPair =
          g.rows.length === 2 &&
          g.rows.every((s) => optionTypeKind(s) !== "vector-point");
        if (asPair) {
          return (
            <PairedInputRow
              key={g.name ?? "__pair"}
              label={g.name ?? ""}
              members={g.rows}
              valueOf={(m) => elemValueOf(m.key)}
              overriddenOf={(m) => overriddenOf(m.key)}
              onSet={(m, next) => setElem(m.key, next)}
              onClear={(m) => resetElem(m.key)}
            />
          );
        }
        return (
          <div className="mc-group" key={g.name ?? "__ungrouped"}>
            {g.name && <h4 className="mc-group-header">{g.name}</h4>}
            {g.rows.map(renderRow)}
          </div>
        );
      })}
    </div>
  );
}
