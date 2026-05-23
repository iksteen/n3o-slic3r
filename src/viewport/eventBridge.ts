// Tauri ↔ SceneMirror bridge (PR-2-9).
//
// Subscribes to the `scene:*` events the Rust backend emits and
// routes them through `SceneMirror.applyEvent`. On startup or
// reconnect it pulls `scene_snapshot` and replays it via
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

/** Names match the SceneEvent::name() function on the Rust side
 * (`scene:<noun>_<verb>`). The bridge subscribes to each and
 * dispatches to the mirror. */
const EVENT_NAMES = [
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

/** Wire up the bridge. Returns an unsubscribe function. */
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
    });
    unlisteners.push(un);
  }

  // Initial sync: pull the snapshot and replay.
  const snapshot = await invoke<SceneSnapshot>("scene_snapshot");
  if (import.meta.env.DEV) {
    console.debug("[n3o] initial snapshot", snapshot);
  }
  await mirror.applySnapshot(snapshot);

  return async () => {
    for (const un of unlisteners) un();
  };
}
