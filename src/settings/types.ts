// Settings wire-format types.
//
// Mirrors the Rust shapes in `src-tauri/src/core/cascade/mod.rs`:
//
//   - `OptionSummary`        (introspection fields)
//   - `OptMode`              (lowercase string enum)
//   - `OptScopeFlags`        (struct of bools)
//   - `CapabilityPredicate`  (tagged enum)
//   - `PrinterAwareOptionSummary` (summary + hidden bool)
//
// Wire-shape drift between this file and the Rust serde output is
// the most common cause of silent UI bugs (see the Phase 2 viewport
// gizmo Transform shape regression for prior art). Mirror the
// serde output literally; the Rust tests assert the shapes.

/** libslic3r option mode (FR-UI-2 Simple/Advanced/Expert filter). */
export type OptMode = "simple" | "advanced" | "expert" | "develop";

/** Project/object/region scope bitmask flattened to bools (FR-3D-3). */
export type OptScopeFlags = {
  project: boolean;
  object: boolean;
  region: boolean;
};

/** Whether a setting can be edited as a per-object override on the Object
 *  tab. Mirrors the slice-time gate (`object_overrides_for_slice`): only
 *  object- or region-scoped (PrintObjectConfig / PrintRegionConfig) keys
 *  actually reach libslic3r per object. Everything else — project/print-
 *  scope settings *and* dangling options with no scope bit at all (e.g.
 *  `ironing_expansion`, defined but not in any config class) — must be
 *  disabled on the Object tab, or the user sets an override the slicer
 *  silently drops. */
export function isObjectOverridable(scope: OptScopeFlags): boolean {
  return scope.object || scope.region;
}

/** Printer capability predicate that gates option visibility
 *  (FR-UI-7). Tagged enum — `kind` is the variant name from
 *  `core::schema::capability::CapabilityPredicate`. `None` on the
 *  Rust side serializes as `null` in the parent's `capability`
 *  field, never as `{kind: "None"}`. */
export type CapabilityPredicate =
  | { kind: "RequiresMultiSlot" }
  | { kind: "RequiresToolchanger" }
  | { kind: "RequiresPurgeTower" }
  | { kind: "RequiresBblPrinter" };

/** Typed default-value wire shape (mirrors Rust's `cascade::DefaultValue`).
 *
 *  Vector entries are pre-split server-side so the frontend doesn't
 *  have to know about libslic3r's per-type serialization quirks
 *  (`escape_strings_cstyle` for coStrings, comma-joined for coFloats /
 *  coInts / coPercents, etc.). Use `defaultScalarFor(opt, slot)` to
 *  pick the right entry for the active slot. */
export type DefaultValue =
  | { kind: "scalar"; value: string }
  | { kind: "vector"; values: string[] };

/** One libslic3r option's introspection record. Carries everything
 *  a settings row needs at render time except the resolved value
 *  (which comes from `cascade_resolve`). */
export type OptionSummary = {
  key: string;
  /** Debug-formatted enum tag from the FFI (`"Float"`, `"FloatOrPercent"`,
   *  `"Enum"`, etc.). Use the helper `optionTypeKind()` to discriminate. */
  ty: string;
  label: string | null;
  category: string | null;
  /** Optgroup within the category/page — e.g. "Printable space" under
   *  "Basic information". The printer panel renders these as sub-headers.
   *  Null for options with no sub-group. */
  group: string | null;
  default_value: DefaultValue | null;
  /** True for libslic3r options flagged `multiline` — freeform
   *  textareas (start_gcode, end_gcode, the small-area infill flow
   *  compensation model). The panel renders these as a `\n`-joined
   *  textarea via [`defaultMultilineText`]. */
  multiline: boolean;
  /** True when libslic3r's `gui_type` marks this a color picker
   *  (`filament_colour`, `extruder_colour`, …). Drives the color input —
   *  the authoritative classification, not a hand-curated key list. */
  is_color: boolean;
  /** `[value, label]` pairs in libslic3r declaration order for Enum
   *  options. Empty for non-enum types. DropdownInput consumes these
   *  directly — no per-key lookup at render time. */
  enum_values: ReadonlyArray<readonly [string, string]>;
  tooltip: string | null;
  /** Unit suffix (mm, mm/s, %, °C, …) from libslic3r's `sidetext`, shown
   *  after the input. Null for unitless options. */
  sidetext: string | null;
  mode: OptMode;
  scope: OptScopeFlags;
  capability: CapabilityPredicate | null;
};

/** Per-slot scalar view of the default — the value a single scalar
 *  Field component (NumberInput / BoolInput / …) renders.
 *
 *  - Scalar default → its value.
 *  - Vector default → entry at `slot`, falling back to `values[0]`
 *    if the slot is out of range. The vector case is the per-extruder
 *    path; multiline textareas use [`defaultMultilineText`] instead,
 *    because joining N comma-containing coStrings entries with `\n`
 *    is meaningless when the consumer is a per-slot picker.
 *
 *  Returns `null` when the option has no default or the vector is
 *  empty. */
export function defaultScalarFor(
  opt: OptionSummary,
  slot: number = 0,
): string | null {
  const dv = opt.default_value;
  if (dv === null) return null;
  if (dv.kind === "scalar") return dv.value;
  if (dv.values.length === 0) return null;
  return dv.values[slot] ?? dv.values[0] ?? null;
}

/** Multi-line textarea view of the default — one entry per line.
 *
 *  Use this when the consumer is a textarea (multiline coStrings:
 *  `start_gcode`, `end_gcode`, `small_area_infill_flow_compensation_model`,
 *  …). Returns the scalar value for scalar defaults so callers don't
 *  need to discriminate, and `\n`-joins the vector entries for vector
 *  defaults. NEVER pass this output to a per-slot wrapper — the
 *  joined form contains entry-internal commas that the wrapper's
 *  comma-split path would destroy. */
export function defaultMultilineText(opt: OptionSummary): string | null {
  const dv = opt.default_value;
  if (dv === null) return null;
  if (dv.kind === "scalar") return dv.value;
  if (dv.values.length === 0) return null;
  return dv.values.join("\n");
}

/** True for libslic3r `coStrings` options flagged `multiline` —
 *  `start_gcode`, `end_gcode`, `small_area_infill_flow_compensation_model`,
 *  etc. These are textareas with one entry per line, NOT per-extruder
 *  vectors; the panel routes them to a read-only textarea seeded
 *  from [`defaultMultilineText`]. */
export function isMultilineTextField(opt: OptionSummary): boolean {
  return opt.multiline && opt.ty === "Strings";
}

/** `slicer_options_for_printer` result. Same as `OptionSummary` with
 *  the capability predicate pre-evaluated against the active printer
 *  (`hidden` is what the panel reads to decide visibility). */
export type PrinterAwareOptionSummary = OptionSummary & {
  hidden: boolean;
};

/** Categorize the raw `ty` string the FFI ships into the shapes the
 *  form components branch on. Keeps the switch statements honest
 *  to a small fixed vocabulary. */
export type OptionTypeKind =
  | "bool"
  | "int"
  | "float"
  | "percent"
  | "float-or-percent"
  | "string"
  | "color"
  | "enum"
  | "vector-bool"
  | "vector-int"
  | "vector-float"
  | "vector-percent"
  | "vector-float-or-percent"
  | "vector-string"
  | "vector-enum"
  | "point"
  | "vector-point"
  | "point3"
  | "unknown";

const TYPE_KIND_MAP: Record<string, OptionTypeKind> = {
  Bool: "bool",
  Bools: "vector-bool",
  Int: "int",
  Ints: "vector-int",
  Float: "float",
  Floats: "vector-float",
  Percent: "percent",
  Percents: "vector-percent",
  FloatOrPercent: "float-or-percent",
  FloatsOrPercents: "vector-float-or-percent",
  String: "string",
  Strings: "vector-string",
  Enum: "enum",
  Enums: "vector-enum",
  Point: "point",
  Points: "vector-point",
  Point3: "point3",
};

export function optionTypeKind(opt: OptionSummary): OptionTypeKind {
  // Color-ness is libslic3r's `gui_type` (carried as `is_color`), not the
  // ConfigOptionType — route those to ColorInput regardless of `ty`.
  if (opt.is_color) return "color";
  return TYPE_KIND_MAP[opt.ty] ?? "unknown";
}

/** True for any vector-shaped libslic3r option. Vector options need
 *  one entry per slot/extruder; the panel's slot-adaptive layout
 *  renders the active slot's index only. */
export function isVectorKind(kind: OptionTypeKind): boolean {
  return kind.startsWith("vector-");
}

/** The scalar element kind of a vector kind (`vector-float` → `float`),
 *  for rendering one entry of a per-extruder vector as a scalar input.
 *  Returns scalar kinds unchanged. */
export function scalarElementKind(kind: OptionTypeKind): OptionTypeKind {
  return kind.startsWith("vector-")
    ? (kind.slice("vector-".length) as OptionTypeKind)
    : kind;
}
