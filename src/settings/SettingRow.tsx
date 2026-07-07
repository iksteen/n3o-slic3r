import { Field } from "./inputs";
import type { OptionSummary } from "./types";
import {
  defaultMultilineText,
  defaultScalarFor,
  isMultilineTextField,
  isObjectOverridable,
  isVectorKind,
  optionTypeKind,
} from "./types";
import type { ResolvedMap } from "./resolve";
import { winningLayerFor, type CascadeLayer } from "./layers";
import { renderScalarInput } from "./renderScalarInput";
import type { ContextLayer } from "./settingsPanelHelpers";
import type { PlateObject } from "./SettingsPanel";

export interface SettingRowProps {
  schema: OptionSummary;
  resolved: ResolvedMap;
  contextLayer: ContextLayer;
  projectOverrides: Record<string, string>;
  objectOverrides: Record<string, string>;
  onSetProjectOverride: (key: string, value: string) => void;
  onClearProjectOverride: (key: string) => void;
  onSetObjectOverride: (key: string, value: string) => void;
  onClearObjectOverride: (key: string) => void;
  /** True when this option is capability-hidden but surfaced via
   *  search; renders the "not applicable" badge inline. */
  notApplicable?: boolean;
  /** True when this option's tier is above the active mode — it's shown only
   *  because it's been modified. Renders an ADV/EXP tier-tag. */
  outOfMode?: boolean;
  /** Cascade ladder hover hooks. The panel owns the
   *  open/close lifecycle centrally; SettingRow just forwards the
   *  row's DOM node + leave. The label hover hooks retired with
   *  the SettingTooltip merge — description lives in the ladder. */
  onRowEnter?: (el: HTMLElement) => void;
  onRowLeave?: () => void;
  /** All objects on the plate — drives the objects-overriding
   *  badge on Project-tab rows. Empty by default. */
  allObjects: ReadonlyArray<PlateObject>;
}

export function SettingRow({
  schema,
  resolved,
  contextLayer,
  projectOverrides,
  objectOverrides,
  onSetProjectOverride,
  onClearProjectOverride,
  onSetObjectOverride,
  onClearObjectOverride,
  notApplicable = false,
  outOfMode = false,
  onRowEnter,
  onRowLeave,
  allObjects,
}: SettingRowProps) {
  // Object tab folds the project override under the object one (object > project
  // > cascade), so a value authored at the project tier shows through as the
  // effective value here — `resolved` is fragment-only and carries no override
  // tiers. The project tab shows only the project tier.
  const tierValue = contextLayer === "object"
    ? objectOverrides[schema.key] ?? projectOverrides[schema.key]
    : projectOverrides[schema.key];
  // Multiline coStrings (`start_gcode`, the small-area infill flow
  // compensation model, …) carry one entry per line of a single
  // logical text block. The `\n`-joined textarea view is what
  // displays; per-slot indexing is meaningless here.
  const fallbackDefault = isMultilineTextField(schema)
    ? defaultMultilineText(schema)
    : defaultScalarFor(schema);
  const effectiveValue =
    tierValue ?? resolved[schema.key]?.value ?? fallbackDefault ?? null;

  // On the Object tab only object/region-scoped settings are editable —
  // they're the only ones libslic3r honors per object, mirroring the
  // slice-time gate (`object_overrides_for_slice`). Anything else (a
  // project/print-scope setting, or a dangling no-scope option like
  // `ironing_expansion`) is disabled, so the user can't author an
  // override the slicer would silently drop. The "project-scope
  // setting" badge surfaces the reason; this enforces disabled-input.
  const disabled =
    notApplicable ||
    (contextLayer === "object" && !isObjectOverridable(schema.scope));

  const setValue = (next: string) => {
    if (contextLayer === "object") onSetObjectOverride(schema.key, next);
    else onSetProjectOverride(schema.key, next);
  };

  // ADV/EXP tier-tag: this setting is above the active mode but shown because
  // it's been modified. `expert`/`develop` → EXP; everything else → ADV.
  const tierTag = outOfMode ? (
    (() => {
      const expert = schema.mode === "expert" || schema.mode === "develop";
      return (
        <span
          className={`tier-tag tier-${expert ? "expert" : "advanced"}`}
          title={`${expert ? "An Expert" : "An Advanced"} setting, shown because it's been changed`}
        >
          {expert ? "EXP" : "ADV"}
        </span>
      );
    })()
  ) : null;

  const leadingBadge =
    notApplicable || tierTag ? (
      <>
        {notApplicable && (
          <span
            className="set-badge set-badge-na"
            title="Not applicable to the active printer"
          >
            not applicable
          </span>
        )}
        {tierTag}
      </>
    ) : null;

  // Objects-overriding badge (FR-CAS-7b): on the Project
  // tab, surface the objects that override this setting via small
  // filament-color dots + a count.
  const overridingObjects =
    contextLayer === "project"
      ? allObjects.filter((o) => schema.key in o.overrides)
      : [];
  const trailingBadge = overridingObjects.length > 0 ? (
    <span
      className="objs-badge"
      title={`${overridingObjects.length} object${
        overridingObjects.length === 1 ? "" : "s"
      } override this setting`}
    >
      {overridingObjects.slice(0, 3).map((o) => (
        <span
          key={o.id}
          className="objs-badge-dot"
          style={{ background: o.color ?? "#888" }}
          aria-hidden
        />
      ))}
      {overridingObjects.length > 3 && (
        <span className="objs-badge-more">+{overridingObjects.length - 3}</span>
      )}
    </span>
  ) : null;

  // Reset button. Renders when the active tier has a
  // value for this setting; clicking drops the override and the
  // row falls back to the cascade resolution underneath.
  const hasValueAtActiveTier =
    contextLayer === "object"
      ? schema.key in objectOverrides
      : schema.key in projectOverrides;
  const resetButton = hasValueAtActiveTier ? (
    <button
      type="button"
      className="reset-btn"
      title={`Reset ${contextLayer} override (falls back to inherited value)`}
      onClick={(e) => {
        e.stopPropagation();
        if (contextLayer === "object") onClearObjectOverride(schema.key);
        else onClearProjectOverride(schema.key);
      }}
      aria-label={`Reset ${schema.key} ${contextLayer} override`}
    >
      <svg width="12" height="12" viewBox="0 0 12 12" fill="none">
        <path
          d="M2.5 5a3.5 3.5 0 1 0 1-2.5"
          stroke="currentColor"
          strokeWidth="1.4"
          strokeLinecap="round"
        />
        <path
          d="M2 2v3h3"
          stroke="currentColor"
          strokeWidth="1.4"
          strokeLinecap="round"
          strokeLinejoin="round"
        />
      </svg>
    </button>
  ) : null;

  const kind = optionTypeKind(schema);
  // Row highlight (tint). Object tab: the selected object's overrides win.
  // Project tab: any object overriding the setting tints it the object tier
  // (matching the trailing objects-overriding badge + the per-category
  // count), falling back to the project tier, then the cascade.
  const winningLayer: CascadeLayer =
    contextLayer === "object"
      ? winningLayerFor(schema.key, projectOverrides, objectOverrides)
      : overridingObjects.length > 0
        ? "object"
        : schema.key in projectOverrides
          ? "project"
          : "cascade";
  return (
    <Field
      schema={schema}
      value={effectiveValue}
      onChange={setValue}
      disabled={disabled}
      leadingBadge={leadingBadge}
      trailingBadge={trailingBadge}
      resetButton={resetButton}
      winningLayer={winningLayer}
      onRowEnter={onRowEnter}
      onRowLeave={onRowLeave}
    >
      {isMultilineTextField(schema) ? (
        // Multiline coStrings → single textarea showing all entries
        // joined with `\n`. Read-only for now — a real textarea
        // Field with cstyle-aware commit ships later.
        <textarea
          className="val-input val-input-multiline"
          value={effectiveValue ?? ""}
          disabled
          readOnly
          rows={Math.min(
            10,
            Math.max(2, (effectiveValue?.split("\n").length ?? 1)),
          )}
        />
      ) : isVectorKind(kind) ? (
        // Other vector kinds (`vector-int`, `vector-float`, etc.)
        // surface in the Process bucket through dimensional families
        // (e.g. bed_temp expands per plate type) — but the panel
        // doesn't have an editor for those yet. Show read-only text
        // so the user can see the value without a path to corrupt
        // it. Per-extruder editing surfaces (filament/printer
        // buckets) live elsewhere; the panel's Process-only filter
        // means no slot picker mounts here.
        <input
          className="val-input val-input-fallback"
          type="text"
          value={effectiveValue ?? ""}
          readOnly
          disabled
        />
      ) : (
        renderScalarInput(kind, schema, effectiveValue, setValue, disabled)
      )}
    </Field>
  );
}
