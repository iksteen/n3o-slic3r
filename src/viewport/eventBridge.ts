// Tauri ↔ SceneMirror bridge (PR-2-9, expanded for plate-routing in
// PR-5-2 phase C).
//
// Subscribes to every `scene:*` / `project:*` event the Rust backend
// emits and routes them through `SceneMirror.applyEvent`. On startup
// or reconnect it pulls `scene_snapshot` and replays it via
// `applySnapshot` — Rust is the source of truth, the renderer
// rebuilds its mirror at any time without losing state.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { SceneMirror, MeshBufferProvider } from "./sceneMirror";
import type {
  MeshHeader,
  SceneEvent,
  SceneSnapshot,
} from "./types";

/** Names match each `SceneEvent::name()` arm on the Rust side. The
 * bridge subscribes to each and dispatches to the mirror.
 *
 * PR-5-2 phase C added the plate-list mutation events
 * (`plate_added`, `plate_removed`, `active_plate_changed`) and the
 * project-state notifiers (`plate_metadata_changed`,
 * `material_binding_changed`, `object_overrides_changed`). PR-5-8
 * added `project:saved` / `project:loaded`. */
const EVENT_NAMES = [
  // Scene-graph deltas
  "scene:mesh_loaded",
  "scene:object_added",
  "scene:object_updated",
  "scene:object_removed",
  "scene:selection_changed",
  "scene:gizmo_changed",
  "scene:camera_changed",
  "scene:bed_changed",
  "scene:object_out_of_bounds",
  "scene:non_uniform_scale",
  "scene:auto_arrange_overflow",
  // Plate list mutations (PR-5-2)
  "scene:plate_added",
  "scene:plate_removed",
  "scene:active_plate_changed",
  // Project-state notifiers (PR-5-5, PR-5-7, PR-S-7)
  "scene:plate_metadata_changed",
  "scene:material_slot_changed",
  "scene:object_overrides_changed",
  // Project save/load (PR-5-8)
  "project:saved",
  "project:loaded",
] as const;

/** Build the mesh-buffer provider that calls `scene_mesh_buffers`
 * and decodes the LE-packed `[vertices][normals][indices]` blob the
 * Rust side returns. */
export function tauriMeshBufferProvider(): MeshBufferProvider {
  return async (header: MeshHeader) => {
    // Tauri 2 returns `tauri::ipc::Response` as an ArrayBuffer when
    // the front-end calls invoke; @tauri-apps/api's typings declare
    // it as `unknown`, so we cast.
    const buf = (await invoke("scene_mesh_buffers", {
      meshId: header.id,
    })) as ArrayBuffer;
    return decodeMeshBuffer(buf, header);
  };
}

/** Decode the packed binary mesh blob into the three typed arrays
 * Three.js needs. Exported for test code that wants to round-trip a
 * Rust-side `pack_buffers` output without IPC. */
export function decodeMeshBuffer(
  buf: ArrayBuffer,
  header: MeshHeader,
): { vertices: Float32Array; normals: Float32Array; indices: Uint32Array } {
  const vertexCount = header.vertex_count;
  const indexCount = header.index_count;
  // Layout: [vertices: vertex_count * 3 * f32]
  //         [normals:  vertex_count * 3 * f32]
  //         [indices:  index_count * u32]
  const vBytes = vertexCount * 3 * 4;
  const nBytes = vertexCount * 3 * 4;
  const iBytes = indexCount * 4;
  const expected = vBytes + nBytes + iBytes;
  if (buf.byteLength !== expected) {
    throw new Error(
      `mesh buffer size mismatch: got ${buf.byteLength}, expected ${expected} (vc=${vertexCount}, ic=${indexCount})`,
    );
  }
  const vertices = new Float32Array(buf, 0, vertexCount * 3);
  const normals = new Float32Array(buf, vBytes, vertexCount * 3);
  const indices = new Uint32Array(buf, vBytes + nBytes, indexCount);
  return { vertices, normals, indices };
}

/** Wire up the bridge. Returns an unsubscribe function.
 *
 * Special-cases `project:loaded` — when fired, the in-memory project
 * was just replaced wholesale and the mirror must re-fetch a fresh
 * snapshot rather than try to diff the old state forward. */
export async function attachEventBridge(
  mirror: SceneMirror,
): Promise<() => Promise<void>> {
  const unlisteners: UnlistenFn[] = [];
  for (const name of EVENT_NAMES) {
    const un = await listen<SceneEvent>(name, (e) => {
      // Tauri delivers the payload as the event's `payload` field.
      // The backend already shapes it as `{ kind, data }`.
      if (import.meta.env.DEV) {
        console.debug("[n3o] event in", name, e.payload);
      }
      void mirror.applyEvent(e.payload);
      if (e.payload.kind === "ProjectLoaded") {
        // The whole project changed — refetch the snapshot rather
        // than try to incrementally update from prior state.
        void refreshSnapshot(mirror);
      }
    });
    unlisteners.push(un);
  }

  // Initial sync: pull the snapshot and replay.
  await refreshSnapshot(mirror);

  return async () => {
    for (const un of unlisteners) un();
  };
}

async function refreshSnapshot(mirror: SceneMirror): Promise<void> {
  const snapshot = await invoke<SceneSnapshot>("scene_snapshot");
  if (import.meta.env.DEV) {
    console.debug("[n3o] initial snapshot", snapshot);
  }
  await mirror.applySnapshot(snapshot);
}
