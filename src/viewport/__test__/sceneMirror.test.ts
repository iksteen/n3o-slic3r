// SceneMirror state-machine tests (PR-2-9).
//
// No real graphics — assertions ride on the mirror's internal
// registries and Three.js object metadata. The buffer provider is
// stubbed so tests can run synchronously without the Tauri IPC.
//
// What we're checking: the event stream "load → select → translate
// → deselect" produces the right side-effects on the local mirror
// (mesh registered, object placed, color tinted on selection,
// matrix updated on transform, color untinted on deselect).

import { describe, expect, it } from "vitest";
import { SceneMirror } from "../sceneMirror";
import type {
  MeshHeader,
  SceneObject,
} from "../types";

function unitCubeHeader(id = 1): MeshHeader {
  return {
    id,
    vertex_count: 24,
    index_count: 36,
    bounding_box: { min: [-1, -1, -1], max: [1, 1, 1] },
    provenance: { kind: "Primitive", data: "cube" },
  };
}

function unitCubeBuffers() {
  // Geometry doesn't matter for the test, but the sizes must match
  // the header's vertex_count / index_count. Empty buffers are fine
  // — the renderer wouldn't draw them visibly, but we never render.
  return {
    vertices: new Float32Array(24 * 3),
    normals: new Float32Array(24 * 3),
    indices: new Uint32Array(36),
  };
}

function sceneObjectAt(id: number, mesh: number, tx: number): SceneObject {
  // Column-major translation matrix. Rust's Transform is
  // `#[serde(transparent)]` over `[f32; 16]`, so the wire shape is
  // a bare array — not an object with a `matrix` field.
  // prettier-ignore
  const m = [
    1, 0, 0, 0,
    0, 1, 0, 0,
    0, 0, 1, 0,
    tx, 0, 0, 1,
  ];
  return {
    id,
    mesh,
    transform: m,
    name: `obj-${id}`,
    visible: true,
    extruder_id: null,
    parent: null,
  };
}

describe("SceneMirror", () => {
  it("registers a mesh + adds an object on MeshLoaded → ObjectAdded", async () => {
    const mirror = new SceneMirror(async () => unitCubeBuffers());
    await mirror.applyEvent({ kind: "MeshLoaded", data: unitCubeHeader(1) });
    await mirror.applyEvent({
      kind: "ObjectAdded",
      data: sceneObjectAt(101, 1, 0),
    });
    expect(mirror.hasMesh(1)).toBe(true);
    expect(mirror.hasObject(101)).toBe(true);
    expect(mirror.objectGroup.children).toHaveLength(1);
  });

  it("tints + untints on selection events", async () => {
    const mirror = new SceneMirror(async () => unitCubeBuffers());
    await mirror.applyEvent({ kind: "MeshLoaded", data: unitCubeHeader(1) });
    await mirror.applyEvent({
      kind: "ObjectAdded",
      data: sceneObjectAt(101, 1, 0),
    });
    const baseline = mirror.objectColor(101)!;
    await mirror.applyEvent({
      kind: "SelectionChanged",
      data: { selected: [101] },
    });
    expect(mirror.objectColor(101)).not.toBe(baseline);
    expect(mirror.selectedIds()).toEqual([101]);
    await mirror.applyEvent({
      kind: "SelectionChanged",
      data: { selected: [] },
    });
    expect(mirror.objectColor(101)).toBe(baseline);
    expect(mirror.selectedIds()).toEqual([]);
  });

  it("applies a new transform on ObjectUpdated", async () => {
    const mirror = new SceneMirror(async () => unitCubeBuffers());
    await mirror.applyEvent({ kind: "MeshLoaded", data: unitCubeHeader(1) });
    await mirror.applyEvent({
      kind: "ObjectAdded",
      data: sceneObjectAt(101, 1, 0),
    });
    const before = mirror.objectMatrix(101)!;
    expect(before[12]).toBe(0);

    await mirror.applyEvent({
      kind: "ObjectUpdated",
      data: sceneObjectAt(101, 1, 25),
    });
    const after = mirror.objectMatrix(101)!;
    // tx is the 13th element (index 12) in column-major.
    expect(after[12]).toBe(25);
  });

  it("removes the object on ObjectRemoved", async () => {
    const mirror = new SceneMirror(async () => unitCubeBuffers());
    await mirror.applyEvent({ kind: "MeshLoaded", data: unitCubeHeader(1) });
    await mirror.applyEvent({
      kind: "ObjectAdded",
      data: sceneObjectAt(101, 1, 0),
    });
    await mirror.applyEvent({
      kind: "ObjectRemoved",
      data: { id: 101 },
    });
    expect(mirror.hasObject(101)).toBe(false);
    expect(mirror.objectGroup.children).toHaveLength(0);
    // Mesh stays in the registry — it's geometry, and another object
    // might want to instance it later.
    expect(mirror.hasMesh(1)).toBe(true);
  });

  it("end-to-end: load → select → translate → deselect emits expected log", async () => {
    const mirror = new SceneMirror(async () => unitCubeBuffers());
    const log: string[] = [];
    mirror.onEvent((e) => log.push(e.kind));

    await mirror.applyEvent({ kind: "MeshLoaded", data: unitCubeHeader(1) });
    await mirror.applyEvent({
      kind: "ObjectAdded",
      data: sceneObjectAt(101, 1, 0),
    });
    await mirror.applyEvent({
      kind: "SelectionChanged",
      data: { selected: [101] },
    });
    await mirror.applyEvent({
      kind: "ObjectUpdated",
      data: sceneObjectAt(101, 1, 50),
    });
    await mirror.applyEvent({
      kind: "SelectionChanged",
      data: { selected: [] },
    });

    expect(log).toEqual([
      "MeshLoaded",
      "ObjectAdded",
      "SelectionChanged",
      "ObjectUpdated",
      "SelectionChanged",
    ]);
    expect(mirror.objectMatrix(101)![12]).toBe(50);
    expect(mirror.selectedIds()).toEqual([]);
  });

  it("renders a bed overlay on BedChanged", async () => {
    const mirror = new SceneMirror(async () => unitCubeBuffers());
    await mirror.applyEvent({
      kind: "BedChanged",
      data: {
        extents: { min: [0, 0, 0], max: [180, 180, 180] },
        grid_spacing: 10,
        origin_marker: [0, 0, 0],
        exclusion_zones: [
          {
            label: "ams",
            bounds: { min: [0, 150, 0], max: [30, 180, 5] },
          },
        ],
      },
    });
    // Grid lines + outline + 1 zone wireframe = 3 children.
    expect(mirror.bedChildCount()).toBe(3);

    // BedChanged(null) clears it.
    await mirror.applyEvent({ kind: "BedChanged", data: null });
    expect(mirror.bedChildCount()).toBe(0);
  });

  it("clear() drops everything", async () => {
    const mirror = new SceneMirror(async () => unitCubeBuffers());
    await mirror.applyEvent({ kind: "MeshLoaded", data: unitCubeHeader(1) });
    await mirror.applyEvent({
      kind: "ObjectAdded",
      data: sceneObjectAt(101, 1, 0),
    });
    await mirror.applyEvent({
      kind: "BedChanged",
      data: {
        extents: { min: [0, 0, 0], max: [180, 180, 180] },
        grid_spacing: 10,
        origin_marker: [0, 0, 0],
        exclusion_zones: [],
      },
    });
    mirror.clear();
    expect(mirror.hasObject(101)).toBe(false);
    expect(mirror.hasMesh(1)).toBe(false);
    expect(mirror.bedChildCount()).toBe(0);
  });
});
