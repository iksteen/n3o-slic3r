// One machine-settings category in the printer panel (Basic information,
// Machine G-code, Motion ability, …). Auto-populated from
// `slicer_machine_options_for_printer` — same field rendering as the
// profile settings panel. Overrides persist live to the instance's
// `config_overrides` (Printer-bucket tier of the cascade).
//
// Most rows are scalars (or multiline G-code). The machine-limits vectors
// (`machine_max_*`) are the exception: libslic3r stores them per-mode —
// index 0 Normal, index 1 Silent/Stealth — so they render a Normal field,
// plus a Silent field below it when the printer's `silent_mode` is on.

import { Field } from "../settings/inputs";
import { SettingControl } from "../settings/inputs/SettingControl";
import { renderScalarInput } from "../settings/renderScalarInput";
import {
  isVectorKind,
  optionTypeKind,
  scalarElementKind,
  type PrinterAwareOptionSummary,
} from "../settings/types";
import { elemOverridden, setVecElem, vecElem } from "./vectorOverride";

export interface MachineSettingsSectionProps {
  settings: PrinterAwareOptionSummary[];
  /** The instance's raw overrides (`config_overrides`). */
  overrides: Record<string, string>;
  /** Resolved base values (printer fragment + instance), pre-override. */
  resolved: Record<string, string>;
  /** Whether the printer exposes Silent/Stealth mode — gates the second
   *  machine-limits column. */
  silentMode: boolean;
  onSet: (key: string, value: string) => void;
  onClear: (key: string) => void;
}

export function MachineSettingsSection({
  settings,
  overrides,
  resolved,
  silentMode,
  onSet,
  onClear,
}: MachineSettingsSectionProps): React.JSX.Element {
  const visible = settings.filter((s) => !s.hidden);

  // Group rows by their optgroup sub-group (e.g. "Printable space" under
  // "Basic information"), preserving first-seen order. Ungrouped rows
  // (group == null) collect under a leading headerless block.
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

        // Machine-limits vectors → per-mode (Normal / Silent) editor.
        if (isVectorKind(optionTypeKind(schema))) {
          const elemKind = scalarElementKind(optionTypeKind(schema));
          const overridden =
            elemOverridden(overrides, resolved, key, 0) ||
            (silentMode && elemOverridden(overrides, resolved, key, 1));
          // Single column when there's no Silent mode (just the Normal value).
          const modes: ReadonlyArray<readonly [number, string | null]> =
            silentMode
              ? [
                  [0, "Normal"],
                  [1, "Silent"],
                ]
              : [[0, null]];
          return (
            <Field
              key={key}
              schema={schema}
              value={vecElem(overrides, resolved, key, 0)}
              onChange={() => {}}
              resetButton={
                overridden ? (
                  <button
                    type="button"
                    className="reset-btn"
                    title="Reset to printer default"
                    aria-label={`Reset ${key}`}
                    onClick={() => onClear(key)}
                  >
                    ↺
                  </button>
                ) : null
              }
              winningLayer={overridden ? "user" : "cascade"}
            >
              <div className="mc-modes">
                {modes.map(([index, label]) => (
                  <div className="mc-mode-row" key={index}>
                    {label && <span className="mc-mode-label">{label}</span>}
                    {renderScalarInput(
                      elemKind,
                      schema,
                      vecElem(overrides, resolved, key, index),
                      (next) =>
                        setVecElem(
                          overrides,
                          resolved,
                          key,
                          index,
                          next,
                          onSet,
                          onClear,
                        ),
                      false,
                    )}
                  </div>
                ))}
              </div>
            </Field>
          );
        }

        // Scalars + multiline G-code.
        const overridden = key in overrides;
        const value = overridden ? overrides[key] : (resolved[key] ?? null);
        const resetButton = overridden ? (
          <button
            type="button"
            className="reset-btn"
            title="Reset to printer default"
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
            onChange={(next) => onSet(key, next)}
            resetButton={resetButton}
            winningLayer={overridden ? "user" : "cascade"}
          >
            <SettingControl
              schema={schema}
              value={value}
              onChange={(next) => onSet(key, next)}
              multilineEditable
            />
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
