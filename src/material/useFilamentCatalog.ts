// Shared loader for the bundled filament catalog. The slot-binding panel and
// the Devices monitor both need to map a slot's `filament_identity` to a
// display name / base type; this centralizes the one-shot fetch + identity
// index so it isn't reimplemented (and allowed to drift) per surface.
//
// State-layer: the catalog is a query with no invalidation event (static for
// the session — `invalidateOn: []`). The query cache holds the one fetched
// list, so every consumer shares one `filament_profile_list` round-trip and a
// remount reads the cached value immediately. When user-library editing lands,
// give this query a `filament:changed`-style invalidation event.

import { useMemo } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { FilamentSummary } from "./filamentSummary";
import { defineQuery, useQuery } from "../state/queryCache";

export interface FilamentCatalog {
  /** Raw bundled filament fragments. */
  list: FilamentSummary[];
  /** Indexed by `identity` for O(1) display lookup. */
  byIdentity: Map<string, FilamentSummary>;
}

/** Stable empty reference for the pre-first-fetch window. */
const NO_FILAMENTS: FilamentSummary[] = [];

export const filamentCatalogQuery = defineQuery<FilamentSummary[]>({
  key: "filament_catalog",
  fetch: () => invoke<FilamentSummary[]>("filament_profile_list"),
  // User-library edits (duplicate / delete / rename) emit `filament:changed`
  // backend-side; bundled fragments are static, so that's the only churn.
  invalidateOn: ["filament:changed"],
});

/** The bundled filament catalog, indexed by identity. Shared across every
 *  consumer via the query cache. */
export function useFilamentCatalog(): FilamentCatalog {
  const { data } = useQuery(filamentCatalogQuery);
  const list = data ?? NO_FILAMENTS;
  const byIdentity = useMemo(() => {
    const map = new Map<string, FilamentSummary>();
    for (const f of list) map.set(f.identity, f);
    return map;
  }, [list]);
  return { list, byIdentity };
}
