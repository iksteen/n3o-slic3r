// One category of filament settings, rendered as scalar rows. Filament
// options are per-filament *vectors* in libslic3r, but a user filament is
// a single filament: the fragment stores (and we override) one scalar per
// key, which the composer zips into the vector at compose time. So unlike
// the per-extruder panel there's no vector round-trip here — each row is a
// plain scalar.
//
// value = override scalar if set, else the base fragment's resolved
// scalar. Editing writes a scalar override to the user filament;
// resetting clears it (back to the base value).

import { Field } from "../settings/inputs";
import { MultilineEditor } from "../settings/inputs/SettingControl";
import { renderScalarInput } from "../settings/renderScalarInput";
import {
  defaultScalarFor,
  optionTypeKind,
  scalarElementKind,
  type PrinterAwareOptionSummary,
} from "../settings/types";

export interface FilamentSettingsSectionProps {
  settings: PrinterAwareOptionSummary[];
  /** The user filament's overrides (`key → scalar`). */
  overrides: Record<string, string>;
  /** Base (pre-override) scalar values from the bound fragment. */
  resolved: Record<string, string>;
  onSet: (key: string, value: string) => void;
  onClear: (key: string) => void;
}

export function FilamentSettingsSection({
  settings,
  overrides,
  resolved,
  onSet,
  onClear,
}: FilamentSettingsSectionProps): React.JSX.Element {
  const visible = settings.filter((s) => !s.hidden);

  // Group rows by their optgroup sub-group (e.g. "Print temperature" under
  // the "Filament" page), preserving first-seen order — same as the
  // machine panel. Ungrouped rows collect under a leading headerless block.
  const groups: { name: string | null; rows: PrinterAwareOptionSummary[] }[] =
    [];
  for (const s of visible) {
    const name = s.group ?? null;
    const last = groups.find((g) => g.name === name);
    if (last) last.rows.push(s);
    else groups.push({ name, rows: [s] });
  }

  const renderRow = (schema: PrinterAwareOptionSummary): React.JSX.Element => {
    const key = schema.key;
    const elemKind = scalarElementKind(optionTypeKind(schema));
    const overridden = key in overrides;
    // Fall back to the libslic3r engine default (its scalar/first-vector
    // entry) for keys the base fragment doesn't author, so the editor shows
    // the value that would actually slice rather than a blank field.
    const value = overridden
      ? overrides[key]
      : (resolved[key] ?? defaultScalarFor(schema, 0));
    const set = (next: string): void => onSet(key, next);
    const resetButton = overridden ? (
      <button
        type="button"
        className="reset-btn"
        title="Reset to filament default"
        aria-label={`Reset ${key}`}
        onClick={() => onClear(key)}
      >
        ↺
      </button>
    ) : null;
    return (
      <Field
        key={key}
        schema={schema}
        value={value}
        onChange={set}
        resetButton={resetButton}
        winningLayer={overridden ? "user" : "cascade"}
      >
        {/* Multiline coStrings (filament_start_gcode, the adaptive-PA
            measurement blob, …) get the same pop-out editor as the machine
            panel; everything else is a scalar element of the per-filament
            vector. */}
        {schema.multiline ? (
          <MultilineEditor schema={schema} value={value} onChange={set} />
        ) : (
          renderScalarInput(elemKind, schema, value, set, false)
        )}
      </Field>
    );
  };

  return (
    <div className="machine-settings">
      {groups.map((g) => (
        <div className="mc-group" key={g.name ?? "__ungrouped"}>
          {g.name && <h4 className="mc-group-header">{g.name}</h4>}
          {g.rows.map(renderRow)}
        </div>
      ))}
    </div>
  );
}
