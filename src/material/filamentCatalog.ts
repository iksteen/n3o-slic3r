// Stub filament catalog (PR-5-6 UI).
//
// The real filament registry — bundled profiles + per-printer
// loaded state — arrives with PR-7c (filament sync). Until then
// the binding panel offers a small hand-authored list covering
// the common families so the picker is usable end-to-end.
//
// The identity strings are stable (`Generic PLA` etc., matching
// the bundled `profiles/filaments/generic-pla.toml` slug used
// elsewhere in the codebase). When PR-7c ships, this constant
// is replaced by a Tauri `filament_catalog` command; consumers
// keep importing the same `FILAMENT_CATALOG` name so the panel
// transitions without JSX changes.

export interface FilamentCatalogEntry {
  /** Stable identity used as `MaterialBinding.filament_identity`. */
  identity: string;
  /** Human-readable label for the picker dropdown. */
  label: string;
  /** Filament family — drives the auto-bind heuristic in PR-7c. */
  family: "PLA" | "PETG" | "ABS" | "TPU" | "PC" | "PA";
  /** Display color the swatch dot renders. Hex. */
  color: string;
}

export const FILAMENT_CATALOG: FilamentCatalogEntry[] = [
  { identity: "Generic PLA", label: "Generic PLA", family: "PLA", color: "#9CA3AF" },
  { identity: "Generic PETG", label: "Generic PETG", family: "PETG", color: "#60A5FA" },
  { identity: "Generic ABS", label: "Generic ABS", family: "ABS", color: "#1F2937" },
  { identity: "Generic TPU", label: "Generic TPU", family: "TPU", color: "#10B981" },
];

/** Look up a catalog entry by identity. Returns `null` for
 * identities not in the bundled stub list — common during
 * project load before PR-7c ships, since a saved project may
 * reference any identity string. The panel falls back to
 * rendering the raw identity in that case. */
export function lookupFilament(identity: string): FilamentCatalogEntry | null {
  return FILAMENT_CATALOG.find((f) => f.identity === identity) ?? null;
}
