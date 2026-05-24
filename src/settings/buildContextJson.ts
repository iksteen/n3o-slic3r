// PR-5-9 — build a `ContextJson` from project + active-plate state.
//
// The frontend owns the project snapshot; the backend cascade
// resolver expects a fully self-describing `ContextJson` on every
// call. This module is the projection that ties them together.
//
// Override tiers (user + project) come through as `OverrideFileSpec`
// arrays — the Rust resolver parses each as TOML at call time. We
// serialize the in-memory `Record<string, string>` maps into a
// single synthetic spec per tier (labels: `user-overrides` /
// `project-overrides`). Object-tier overrides stay as a flat
// `Record<string, string>` per the wire shape PR-5-7 introduced.
//
// Profiles for `printer`, `plate`, and `filaments`:
// - `printer` comes from `scene_load_default_printer` — App.tsx
//   caches the returned `PrinterProfile` and passes it in.
// - `plate` (build plate) defaults to the Textured PEI fixture
//   used everywhere else in the app today. The PR-4-5 build-plate
//   selector edits the active plate's `printer.build_plate_identity`
//   binding; building the `BuildPlateJson` from it is Phase 5+
//   profile-registry work, deferred. For now the constant matches
//   the bundled cascade's default branch.
// - `filaments` defaults to one Generic PLA slot. Per-slot
//   filament bindings live on the PrinterInstance (PR-S-5c) — the
//   single-slot fallback resolves the same way the slice path does
//   when no slot is bound.

import type {
  BuildPlateJson,
  ContextJson,
  FilamentProfileJson,
  OverrideFileSpec,
  PrinterProfileJson,
} from "./resolve";

/** Default A1 mini build plate. Matches
 * `core::slice::default_a1mini::canonical_plate` on the Rust side.
 * The PR-4-5 selector edits the binding identity; reading a full
 * profile out of a registry is a future-phase concern. */
export const DEFAULT_BUILD_PLATE: BuildPlateJson = {
  identity: "Textured PEI",
  libslic3r_curr_bed_type: "Textured PEI Plate",
  surface_kind: "PEI",
};

/** Default filament rendered into slot 0. Same as
 * `core::slice::default_a1mini::canonical_filament`. */
export const DEFAULT_FILAMENT: FilamentProfileJson = {
  identity: "Generic PLA",
  base_type: "PLA",
  vendor: null,
  color: null,
};

/** Escape a string for use as a TOML basic string. Only `"` and
 * `\` need escaping inside a basic string — newlines aren't
 * possible because override values are flat. */
function tomlEscape(s: string): string {
  return s.replace(/\\/g, "\\\\").replace(/"/g, '\\"');
}

/** Serialize a `key → value` override map into a single synthetic
 * TOML override file. Empty input returns `null` so the caller can
 * skip the spec entirely (the Rust resolver does the same with an
 * empty file, but skipping keeps traces cleaner). Exported for
 * tests. */
export function overridesToFileSpec(
  label: string,
  overrides: Record<string, string>,
): OverrideFileSpec | null {
  const entries = Object.entries(overrides);
  if (entries.length === 0) return null;
  // Sort for stable serialization — the resolver doesn't care
  // about source order within a flat file but cache-key stability
  // does.
  entries.sort(([a], [b]) => a.localeCompare(b));
  const body = entries.map(([k, v]) => `${k} = "${tomlEscape(v)}"`).join("\n");
  return { label, content: body + "\n" };
}

export interface BuildContextInput {
  /** The installed printer profile (from `scene_load_default_printer`).
   * `null` means no printer is set — the caller should not build a
   * context at all in that case (the panel renders an empty state). */
  printer: PrinterProfileJson;
  /** Plate-tier project overrides (the active plate's
   * `project_overrides` from the snapshot). */
  projectOverrides: Record<string, string>;
  /** Project-wide user-tier overrides (`Project.user_overrides`). */
  userOverrides: Record<string, string>;
  /** Per-object overrides for the *currently selected* object on
   * the active plate — only populated when the panel is in the
   * Object tab. The cascade resolver treats this as the
   * highest-priority tier. */
  objectOverrides: Record<string, string>;
  /** Currently active slot in the SettingsPanel slot-tab strip. */
  activeSlot: number;
}

/** Build the full `ContextJson` the cascade resolver expects. */
export function buildContextJson(input: BuildContextInput): ContextJson {
  const user = overridesToFileSpec("user-overrides", input.userOverrides);
  const project = overridesToFileSpec(
    "project-overrides",
    input.projectOverrides,
  );
  return {
    printer: input.printer,
    plate: DEFAULT_BUILD_PLATE,
    filaments: [DEFAULT_FILAMENT],
    active_slot: input.activeSlot,
    user_overrides: user === null ? [] : [user],
    project_overrides: project === null ? [] : [project],
    object_overrides: input.objectOverrides,
  };
}
