// Module-level cache of the exact priming-tower mesh from the last slice,
// keyed by plate id.
//
// The mesh is a slice artifact (like the G-code preview): it's produced by
// a slice and rendered read-only, not part of the Rust scene model. It has
// to outlive the ViewportCanvas, which unmounts/remounts on every
// prepare↔preview↔devices switch — and the app *auto-switches to preview*
// the moment a slice finishes, so a viewport-local listener would miss the
// `slice:plate_finished` event entirely. So the cache lives at module
// scope and is fed by an **app-lifetime** listener (wired once from App,
// which is always mounted), guaranteeing the event is caught regardless of
// viewport mount state. The viewport then just reads the cache on mount.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { TowerGeometry, TowerMesh } from "./types";

interface CachedTowerMesh {
  mesh: TowerMesh;
  /** Distinct material count the slice ran with — the viewport drops the
   *  mesh once the plate's live count diverges (the tower reshapes; moving
   *  it does not). */
  materialCount: number;
}

const cache = new Map<number, CachedTowerMesh>();
const subscribers = new Set<(plateId: number) => void>();
// Per-plate sequence: each plate_finished event bumps it. The async
// material-count query below only writes the cache if its event is still the
// latest for that plate — so two slices of the same plate finishing close
// together can't apply out of resolution order (last-to-resolve-wins would
// otherwise cache the older slice's mesh).
const latestSeq = new Map<number, number>();
let seqCounter = 0;

/** The last-sliced tower mesh for `plateId`, or null if none cached. */
export function getCachedTowerMesh(plateId: number): CachedTowerMesh | null {
  return cache.get(plateId) ?? null;
}

/** Subscribe to cache changes (fired with the affected plate id after a
 *  store/clear). The viewport uses this to re-render once the async cache
 *  write lands — `slice:plate_finished` itself can race ahead of it.
 *  Returns an unsubscribe fn. */
export function onTowerMeshCacheChange(
  cb: (plateId: number) => void,
): () => void {
  subscribers.add(cb);
  return () => subscribers.delete(cb);
}

function notify(plateId: number): void {
  for (const cb of subscribers) cb(plateId);
}

/** Wire the app-lifetime `slice:plate_finished` → cache listener. Call once
 *  from an always-mounted component (App); await + invoke the returned
 *  unlisten on teardown. */
export async function setupTowerMeshCache(): Promise<UnlistenFn> {
  // SliceEvent serializes tagged (`#[serde(tag="kind", content="data")]`),
  // so the payload is `{ kind, data: {...} }` — the fields are under `.data`.
  return listen<{ data: { plate_id: number; tower_mesh: TowerMesh | null } }>(
    "slice:plate_finished",
    (e) => {
      const { plate_id, tower_mesh } = e.payload.data;
      const seq = ++seqCounter;
      latestSeq.set(plate_id, seq);
      if (!tower_mesh) {
        cache.delete(plate_id);
        notify(plate_id);
        return;
      }
      // Pair the mesh with the material count it sliced at (read the sliced
      // plate's geometry — it may not be the active plate).
      void invoke<TowerGeometry | null>("plate_tower_geometry", {
        plateId: plate_id,
      })
        .then((g) => {
          if (latestSeq.get(plate_id) !== seq) return; // superseded
          if (g) cache.set(plate_id, { mesh: tower_mesh, materialCount: g.material_count });
          else cache.delete(plate_id);
          notify(plate_id);
        })
        .catch(() => {
          // Query failed (plate removed mid-flight, backend error): drop any
          // stale mesh and re-render, rather than leaving the prior slice's
          // tower on screen with no notify.
          if (latestSeq.get(plate_id) !== seq) return;
          cache.delete(plate_id);
          notify(plate_id);
        });
    },
  );
}
