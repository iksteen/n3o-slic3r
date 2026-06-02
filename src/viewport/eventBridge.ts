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
import type {
  SceneMirror,
  MeshBufferProvider,
  MeshPaintProvider,
} from "./sceneMirror";
import type {
  MeshHeader,
  SceneEvent,
  SceneSnapshot,
} from "./types";
import {
  getPrinterInstance,
  listPrinterInstances,
} from "../printer/printerInstance";

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

/** Build the paint-states provider that calls `scene_mesh_paint`. Returns
 *  one byte per triangle (`0` = unpainted, `N` = filament `N`); an empty
 *  array means the mesh has no MMU painting. */
export function tauriMeshPaintProvider(): MeshPaintProvider {
  return async (header: MeshHeader) => {
    const buf = (await invoke("scene_mesh_paint", {
      meshId: header.id,
    })) as ArrayBuffer;
    return new Uint8Array(buf);
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
      // Anything that may have silently mutated the plate's
      // material→slot routing on the backend: refetch + recolor.
      // Auto-bind runs as a side effect of register_object
      // (ObjectAdded) and rebind_plate_printer wipes + re-binds
      // (PlateMetadataChanged) — neither emits MaterialSlotChanged,
      // so the explicit-edit handler isn't enough on its own.
      if (
        e.payload.kind === "MaterialSlotChanged" ||
        e.payload.kind === "ObjectAdded" ||
        e.payload.kind === "PlateMetadataChanged"
      ) {
        void refreshPlateMaterialToSlot(mirror, e.payload.data.plate_id);
      }
    });
    unlisteners.push(un);
  }

  // Live printer-instance updates: payload is the mutated instance
  // id; fetch its post-mutation state + push so the mirror can
  // recolor any plate bound to it. Kept off the SceneEvent channel
  // since the printer-instance registry is its own concern (the
  // setter lives in `core::printer`, not `core::scene`).
  const instanceUn = await listen<string>("printer:instance_changed", (e) => {
    if (import.meta.env.DEV) {
      console.debug("[n3o] printer:instance_changed", e.payload);
    }
    void pushPrinterInstance(mirror, e.payload);
  });
  unlisteners.push(instanceUn);

  // Prime the cache before the first snapshot so initial render
  // paints with the right spool colors instead of flashing the
  // neutral default for one frame.
  await primePrinterInstances(mirror);

  // Initial sync: pull the snapshot and replay.
  await refreshSnapshot(mirror);

  return async () => {
    for (const un of unlisteners) un();
  };
}

async function primePrinterInstances(mirror: SceneMirror): Promise<void> {
  try {
    const instances = await listPrinterInstances();
    for (const inst of instances) {
      mirror.applyPrinterInstance(inst);
    }
  } catch (err) {
    console.warn("[n3o] failed to prime printer instances", err);
  }
}

async function pushPrinterInstance(
  mirror: SceneMirror,
  id: string,
): Promise<void> {
  try {
    const inst = await getPrinterInstance(id);
    if (inst) mirror.applyPrinterInstance(inst);
  } catch (err) {
    console.warn("[n3o] failed to refresh printer instance", id, err);
  }
}

async function refreshPlateMaterialToSlot(
  mirror: SceneMirror,
  plateId: number,
): Promise<void> {
  try {
    const snapshot = await invoke<SceneSnapshot>("scene_snapshot");
    const plate = snapshot.plates.find((p) => p.plate_id === plateId);
    if (plate) {
      mirror.applyPlateRouting(
        plateId,
        plate.printer_instance_id,
        plate.material_to_slot,
      );
    }
  } catch (err) {
    console.warn(
      "[n3o] failed to refresh material→slot for plate",
      plateId,
      err,
    );
  }
}

async function refreshSnapshot(mirror: SceneMirror): Promise<void> {
  const snapshot = await invoke<SceneSnapshot>("scene_snapshot");
  if (import.meta.env.DEV) {
    console.debug("[n3o] initial snapshot", snapshot);
  }
  await mirror.applySnapshot(snapshot);
}
