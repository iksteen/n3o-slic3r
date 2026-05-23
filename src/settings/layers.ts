// Cascade-layer winning-determination + visual styling (PR-4-7).
//
// Maps a setting's resolved trace into the cascade layer that won,
// then into the visual treatment (background tint + hue) the row
// applies. The seven-layer vocabulary mirrors
// `docs/design/data.jsx`'s `CASCADE_LAYERS`:
//
//   default     hue 220   neutral grey
//   printer     hue 18    warm orange (cascade tier — Phase 5 labels these)
//   build_plate hue 95    green
//   filament    hue 175   teal
//   user        hue 235   cool blue
//   project     hue 285   purple
//   object      hue 340   rose
//
// PR-4-7 surfaces the **override-tier** layers (project / object)
// as persistent authored-tier tints — those are the two the user
// directly authors through the UI. Cascade-tier layers (printer /
// filament / plate / default) all share the "cascade" treatment
// today because the resolver's `SourceLocation` carries file paths
// the frontend can't yet map back to which authored-cascade layer
// they belong to. Phase 5's profile registry will tag the cascade
// files with their layer so the rule on hover can fan out into the
// full seven-hue palette.

/** The seven cascade layers, plus a synthetic `"cascade"` umbrella
 *  for any cascade-tier win whose source isn't yet labeled. */
export type CascadeLayer =
  | "default"
  | "printer"
  | "build_plate"
  | "filament"
  | "user"
  | "project"
  | "object"
  | "cascade";

/** HSL hue per layer — matches docs/design/data.jsx. The `cascade`
 *  umbrella uses neutral grey since we don't know which sub-layer
 *  won. */
export const LAYER_HUE: Record<CascadeLayer, number> = {
  default: 220,
  printer: 18,
  build_plate: 95,
  filament: 175,
  user: 235,
  project: 285,
  object: 340,
  cascade: 0, // not actually rendered as a hue — neutral grey via Tailwind
};

/** Tailwind background-tint class for each authored-tier layer.
 *  Light + dark theme classes paired so the existing
 *  `dark:` variants flip alongside. */
export const LAYER_TINT_CLASS: Partial<Record<CascadeLayer, string>> = {
  project: "bg-purple-50 dark:bg-purple-950/40",
  object: "bg-rose-50 dark:bg-rose-950/40",
  user: "bg-blue-50 dark:bg-blue-950/40",
};

/** Derive the winning layer from the override maps. Object wins
 *  over project (matching FR-CAS-3 tier order — higher number
 *  beats lower); project wins over the cascade. Returns `"cascade"`
 *  for anything not authored at an override tier. Phase 5's profile
 *  registry will refine `"cascade"` into the specific authored-
 *  cascade layer that won. */
export function winningLayerFor(
  key: string,
  projectOverrides: Record<string, string>,
  objectOverrides: Record<string, string>,
): CascadeLayer {
  if (key in objectOverrides) return "object";
  if (key in projectOverrides) return "project";
  return "cascade";
}

/** True when the winning layer is an override tier the user
 *  directly authored — drives the authored-tier tint + bold name.
 *  Distinct from `winningLayerFor === "cascade"` since the
 *  cascade tier MAY also include the (Phase 5) `user` layer, which
 *  is an override tier but not yet surfaced via the override maps. */
export function isAuthoredTier(layer: CascadeLayer): boolean {
  return layer === "user" || layer === "project" || layer === "object";
}
