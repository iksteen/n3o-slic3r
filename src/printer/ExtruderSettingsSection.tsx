// Per-extruder ("toolhead") settings for one extruder index. The options
// are Printer-bucket vectors (retraction, nozzle, Z-hop, …) with one entry
// per toolhead; this renders entry `extruderIndex` as a scalar input.
//
// Storage: `config_overrides` holds the whole serialized vector for the
// key. Editing one extruder's field reads the current vector (override if
// set, else resolved), swaps this index, and writes the full vector back —
// so the existing flat-map override + `!important` cascade injection apply
// unchanged. Resetting one index restores it to the resolved value, and
// clears the key entirely once every entry matches resolved again.

import { Field } from "../settings/inputs";
import { renderScalarInput } from "../settings/SettingsPanel";
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
  // Skip per-extruder `Points` (extruder_offset) for now — it serializes
  // as "XxY" (not the comma Point form) and is geometry we've deferred.
  const visible = settings.filter(
    (s) => !s.hidden && optionTypeKind(s) !== "vector-point",
  );
  return (
    <div className="machine-settings">
      {visible.map((schema) => {
        const key = schema.key;
        const elemKind = scalarElementKind(optionTypeKind(schema));
        const elemValue = vecElem(overrides, resolved, key, extruderIndex);
        const overridden = elemOverridden(
          overrides,
          resolved,
          key,
          extruderIndex,
        );
        const setValue = (next: string): void =>
          setVecElem(overrides, resolved, key, extruderIndex, next, onSet, onClear);
        const reset = (): void =>
          setVecElem(
            overrides,
            resolved,
            key,
            extruderIndex,
            resolvedVec(resolved, key)[extruderIndex] ?? "",
            onSet,
            onClear,
          );

        const resetButton = overridden ? (
          <button
            type="button"
            className="reset-btn"
            title="Reset to printer default"
            aria-label={`Reset ${schema.key} for extruder ${extruderIndex + 1}`}
            onClick={reset}
          >
            ↺
          </button>
        ) : null;

        return (
          <Field
            key={schema.key}
            schema={schema}
            value={elemValue}
            onChange={setValue}
            resetButton={resetButton}
            winningLayer={overridden ? "user" : "cascade"}
          >
            {renderScalarInput(elemKind, schema, elemValue, setValue, false)}
          </Field>
        );
      })}
    </div>
  );
}
