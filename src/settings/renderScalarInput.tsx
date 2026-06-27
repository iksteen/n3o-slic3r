import {
  BoolInput,
  ColorInput,
  DropdownInput,
  NumberInput,
  PercentInput,
  PointInput,
} from "./inputs";
import type { OptionSummary, OptionTypeKind } from "./types";

/** Render a scalar input for a single-value option. */
export function renderScalarInput(
  kind: OptionTypeKind,
  schema: OptionSummary,
  value: string | null,
  onChange: (next: string) => void,
  disabled: boolean,
) {
  switch (kind) {
    case "bool":
      return (
        <BoolInput
          schema={schema}
          value={value}
          onChange={onChange}
          disabled={disabled}
        />
      );
    case "float":
    case "int":
      return (
        <NumberInput
          schema={schema}
          value={value}
          onChange={onChange}
          disabled={disabled}
          unit={schema.sidetext}
        />
      );
    case "percent":
    case "float-or-percent":
      return (
        <PercentInput
          schema={schema}
          value={value}
          onChange={onChange}
          disabled={disabled}
        />
      );
    case "point":
      return (
        <PointInput
          schema={schema}
          value={value}
          onChange={onChange}
          disabled={disabled}
        />
      );
    case "color":
      return (
        <ColorInput
          schema={schema}
          value={value}
          onChange={onChange}
          disabled={disabled}
        />
      );
    case "enum":
      return (
        <DropdownInput
          schema={schema}
          value={value}
          onChange={onChange}
          disabled={disabled}
          options={schema.enum_values}
        />
      );
    case "string":
    case "point3":
    case "unknown":
    default:
      // Fallback to a plain text input for scalar kinds the form
      // library doesn't yet specialize for. Vector kinds are
      // handled in SettingRow above and never reach here.
      return (
        <input
          className="val-input val-input-fallback"
          type="text"
          value={value ?? ""}
          disabled={disabled}
          onChange={(e) => onChange(e.target.value)}
        />
      );
  }
}
