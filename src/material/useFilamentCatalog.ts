// Shared loader for the bundled filament catalog. The slot-binding
// panel and the Devices monitor both need to map a slot's
// `filament_identity` to a display name / base type; this centralizes
// the one-shot fetch + identity index so it isn't reimplemented (and
// allowed to drift) per surface.
//
// The catalog is static for a session, so the fetched list is cached at
// module scope (mirrors `usePrinterCatalog`): every consumer shares one
// `filament_profile_list` round-trip, and a remount (entering the
// Devices view, reopening the settings panel) reads the cached value
// synchronously instead of re-fetching from empty.

import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { FilamentSummary } from "./filamentSummary";

export interface FilamentCatalog {
  /** Raw bundled filament fragments. */
  list: FilamentSummary[];
  /** Indexed by `identity` for O(1) display lookup. */
  byIdentity: Map<string, FilamentSummary>;
}

// Module-level cache, shared across every hook instance and surviving
// component unmount/remount. `null` until the first fetch resolves.
let catalogCache: FilamentSummary[] | null = null;

/** Load the bundled filament catalog once per session and index it by
 *  identity. The catalog is small + stable, so there's no refetch
 *  trigger until user-library editing lands. */
export function useFilamentCatalog(): FilamentCatalog {
  const [list, setList] = useState<FilamentSummary[]>(catalogCache ?? []);
  useEffect(() => {
    if (catalogCache != null) return;
    let cancelled = false;
    void invoke<FilamentSummary[]>("filament_profile_list")
      .then((l) => {
        if (catalogCache == null) catalogCache = l;
        if (!cancelled) setList(l);
      })
      .catch((err) =>
        console.error("[filament] filament_profile_list failed", err),
      );
    return () => {
      cancelled = true;
    };
  }, []);
  const byIdentity = useMemo(() => {
    const map = new Map<string, FilamentSummary>();
    for (const f of list) map.set(f.identity, f);
    return map;
  }, [list]);
  return { list, byIdentity };
}
