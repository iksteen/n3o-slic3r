// SceneMirror state-machine tests (PR-2-9, expanded for plate
// routing in PR-5-2 phase C).
//
// No real graphics — assertions ride on the mirror's internal
// registries and Three.js object metadata. The buffer provider is
// stubbed so tests can run synchronously without the Tauri IPC.

import { describe, expect, it } from "vitest";
import { SceneMirror } from "../sceneMirror";
import type {
  CameraState,
  GizmoState,
  MeshHeader,
  PlateSnapshot,
  SceneObject,
  SceneSnapshot,
} from "../types";
import type { PrinterInstance } from "../../printer/printerInstance";

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
  return {
    vertices: new Float32Array(24 * 3),
    normals: new Float32Array(24 * 3),
    indices: new Uint32Array(36),
  };
}

function sceneObjectAt(id: number, mesh: number, tx: number): SceneObject {
  // Column-major translation matrix.
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

const DEFAULT_CAMERA: CameraState = {
  position: [200, -200, 200],
  target: [0, 0, 0],
  up: [0, 0, 1],
  fov_degrees: 45,
  projection: "Perspective",
};

const DEFAULT_GIZMO: GizmoState = { mode: "None", pivot: null };

function plateSnap(id: number, objects: SceneObject[] = []): PlateSnapshot {
  return {
    plate_id: id,
    name: `Plate ${id}`,
    metadata: { composition_order: id },
    printer: null,
    printer_instance_id: null,
    material_to_slot: {},
    project_overrides: {},
    objects,
    selection: [],
    camera: DEFAULT_CAMERA,
    gizmo: DEFAULT_GIZMO,
    build_plate: null,
    exclusion_zones: [],
    bed: null,
    object_overrides: {},
  };
}

function emptySnapshot(plates: PlateSnapshot[], activeId = 1): SceneSnapshot {
  return {
    project_uuid: "test-uuid",
    source_path: null,
    user_overrides: {},
    file_metadata: {},
    meshes: [],
    plates,
    active_plate_id: activeId,
  };
}

describe("SceneMirror", () => {
  it("registers a mesh + adds an object on MeshLoaded → ObjectAdded", async () => {
    const mirror = new SceneMirror(async () => unitCubeBuffers());
    await mirror.applySnapshot(emptySnapshot([plateSnap(1)]));
    await mirror.applyEvent({
      kind: "MeshLoaded",
      data: { mesh: unitCubeHeader(1) },
    });
    await mirror.applyEvent({
      kind: "ObjectAdded",
      data: { plate_id: 1, object: sceneObjectAt(101, 1, 0) },
    });
    expect(mirror.hasMesh(1)).toBe(true);
    expect(mirror.hasObject(101)).toBe(true);
    expect(mirror.activePlate()!.objectGroup.children).toHaveLength(1);
  });

  it("tints + untints on selection events", async () => {
    const mirror = new SceneMirror(async () => unitCubeBuffers());
    await mirror.applySnapshot(emptySnapshot([plateSnap(1)]));
    await mirror.applyEvent({
      kind: "MeshLoaded",
      data: { mesh: unitCubeHeader(1) },
    });
    await mirror.applyEvent({
      kind: "ObjectAdded",
      data: { plate_id: 1, object: sceneObjectAt(101, 1, 0) },
    });
    const baseline = mirror.objectColor(101)!;
    await mirror.applyEvent({
      kind: "SelectionChanged",
      data: { plate_id: 1, selected: [101] },
    });
    expect(mirror.objectColor(101)).not.toBe(baseline);
    expect(mirror.selectedIds()).toEqual([101]);
    await mirror.applyEvent({
      kind: "SelectionChanged",
      data: { plate_id: 1, selected: [] },
    });
    expect(mirror.objectColor(101)).toBe(baseline);
    expect(mirror.selectedIds()).toEqual([]);
  });

  it("applies a new transform on ObjectUpdated", async () => {
    const mirror = new SceneMirror(async () => unitCubeBuffers());
    await mirror.applySnapshot(emptySnapshot([plateSnap(1)]));
    await mirror.applyEvent({
      kind: "MeshLoaded",
      data: { mesh: unitCubeHeader(1) },
    });
    await mirror.applyEvent({
      kind: "ObjectAdded",
      data: { plate_id: 1, object: sceneObjectAt(101, 1, 0) },
    });
    const before = mirror.objectMatrix(101)!;
    expect(before[12]).toBe(0);

    await mirror.applyEvent({
      kind: "ObjectUpdated",
      data: { plate_id: 1, object: sceneObjectAt(101, 1, 25) },
    });
    const after = mirror.objectMatrix(101)!;
    expect(after[12]).toBe(25);
  });

  it("removes the object on ObjectRemoved", async () => {
    const mirror = new SceneMirror(async () => unitCubeBuffers());
    await mirror.applySnapshot(emptySnapshot([plateSnap(1)]));
    await mirror.applyEvent({
      kind: "MeshLoaded",
      data: { mesh: unitCubeHeader(1) },
    });
    await mirror.applyEvent({
      kind: "ObjectAdded",
      data: { plate_id: 1, object: sceneObjectAt(101, 1, 0) },
    });
    await mirror.applyEvent({
      kind: "ObjectRemoved",
      data: { plate_id: 1, object_id: 101 },
    });
    expect(mirror.hasObject(101)).toBe(false);
    expect(mirror.activePlate()!.objectGroup.children).toHaveLength(0);
    // Mesh stays in the registry — it's geometry, and another object
    // might want to instance it later.
    expect(mirror.hasMesh(1)).toBe(true);
  });

  it("end-to-end: load → select → translate → deselect emits expected log", async () => {
    const mirror = new SceneMirror(async () => unitCubeBuffers());
    await mirror.applySnapshot(emptySnapshot([plateSnap(1)]));
    const log: string[] = [];
    mirror.onEvent((e) => log.push(e.kind));

    await mirror.applyEvent({
      kind: "MeshLoaded",
      data: { mesh: unitCubeHeader(1) },
    });
    await mirror.applyEvent({
      kind: "ObjectAdded",
      data: { plate_id: 1, object: sceneObjectAt(101, 1, 0) },
    });
    await mirror.applyEvent({
      kind: "SelectionChanged",
      data: { plate_id: 1, selected: [101] },
    });
    await mirror.applyEvent({
      kind: "ObjectUpdated",
      data: { plate_id: 1, object: sceneObjectAt(101, 1, 50) },
    });
    await mirror.applyEvent({
      kind: "SelectionChanged",
      data: { plate_id: 1, selected: [] },
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
    await mirror.applySnapshot(emptySnapshot([plateSnap(1)]));
    await mirror.applyEvent({
      kind: "BedChanged",
      data: {
        plate_id: 1,
        bed: {
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
      },
    });
    // Grid lines + outline + 1 zone wireframe = 3 children.
    expect(mirror.bedChildCount()).toBe(3);

    await mirror.applyEvent({
      kind: "BedChanged",
      data: { plate_id: 1, bed: null },
    });
    expect(mirror.bedChildCount()).toBe(0);
  });

  it("clear() drops everything", async () => {
    const mirror = new SceneMirror(async () => unitCubeBuffers());
    await mirror.applySnapshot(emptySnapshot([plateSnap(1)]));
    await mirror.applyEvent({
      kind: "MeshLoaded",
      data: { mesh: unitCubeHeader(1) },
    });
    await mirror.applyEvent({
      kind: "ObjectAdded",
      data: { plate_id: 1, object: sceneObjectAt(101, 1, 0) },
    });
    await mirror.applyEvent({
      kind: "BedChanged",
      data: {
        plate_id: 1,
        bed: {
          extents: { min: [0, 0, 0], max: [180, 180, 180] },
          grid_spacing: 10,
          origin_marker: [0, 0, 0],
          exclusion_zones: [],
        },
      },
    });
    mirror.clear();
    expect(mirror.hasObject(101)).toBe(false);
    expect(mirror.hasMesh(1)).toBe(false);
    expect(mirror.bedChildCount()).toBe(0);
    expect(mirror.activePlateIdOrNull()).toBe(null);
    expect(mirror.plateOrder()).toEqual([]);
  });

  // ---- Snapshot bootstrap (PR-5-2 phase C) -----------------------

  it("applySnapshot populates plates + meshes + project metadata", async () => {
    const mirror = new SceneMirror(async () => unitCubeBuffers());
    const snap: SceneSnapshot = {
      project_uuid: "abc-123",
      source_path: "/tmp/proj.3mf",
      user_overrides: { layer_height: "0.12" },
      file_metadata: { Title: "Test" },
      meshes: [unitCubeHeader(1)],
      plates: [
        plateSnap(1, [sceneObjectAt(101, 1, 0)]),
        plateSnap(2),
      ],
      active_plate_id: 1,
    };
    await mirror.applySnapshot(snap);

    expect(mirror.projectUuid).toBe("abc-123");
    expect(mirror.sourcePath).toBe("/tmp/proj.3mf");
    expect(mirror.userOverrides).toEqual({ layer_height: "0.12" });
    expect(mirror.fileMetadata).toEqual({ Title: "Test" });
    expect(mirror.plateOrder()).toEqual([1, 2]);
    expect(mirror.activePlateIdOrNull()).toBe(1);
    expect(mirror.hasObject(101)).toBe(true);
    expect(mirror.hasObjectOnPlate(2, 101)).toBe(false);
  });

  // ---- Per-plate routing -----------------------------------------

  it("ObjectAdded routes to the named plate, not the active one", async () => {
    const mirror = new SceneMirror(async () => unitCubeBuffers());
    await mirror.applySnapshot(
      emptySnapshot([plateSnap(1), plateSnap(2)], 1),
    );
    await mirror.applyEvent({
      kind: "MeshLoaded",
      data: { mesh: unitCubeHeader(1) },
    });
    // Add to plate 2 while plate 1 is active.
    await mirror.applyEvent({
      kind: "ObjectAdded",
      data: { plate_id: 2, object: sceneObjectAt(201, 1, 0) },
    });
    expect(mirror.hasObject(201)).toBe(false); // active plate is 1
    expect(mirror.hasObjectOnPlate(2, 201)).toBe(true);
  });

  it("ActivePlateChanged swaps the top-level group's child", async () => {
    const mirror = new SceneMirror(async () => unitCubeBuffers());
    await mirror.applySnapshot(
      emptySnapshot([plateSnap(1), plateSnap(2)], 1),
    );
    await mirror.applyEvent({
      kind: "MeshLoaded",
      data: { mesh: unitCubeHeader(1) },
    });
    await mirror.applyEvent({
      kind: "ObjectAdded",
      data: { plate_id: 1, object: sceneObjectAt(101, 1, 0) },
    });
    await mirror.applyEvent({
      kind: "ObjectAdded",
      data: { plate_id: 2, object: sceneObjectAt(201, 1, 50) },
    });

    // Active plate is 1 — top-level objectGroup hosts plate 1's group.
    expect(mirror.objectGroup.children).toHaveLength(1);
    expect(mirror.objectGroup.children[0].name).toBe("n3o:plate-1:objects");

    // Switch to plate 2.
    await mirror.applyEvent({
      kind: "ActivePlateChanged",
      data: { plate_id: 2 },
    });
    expect(mirror.activePlateIdOrNull()).toBe(2);
    expect(mirror.objectGroup.children).toHaveLength(1);
    expect(mirror.objectGroup.children[0].name).toBe("n3o:plate-2:objects");
    // Active-plate accessors now report plate 2's contents.
    expect(mirror.hasObject(201)).toBe(true);
    expect(mirror.hasObject(101)).toBe(false);
  });

  it("PlateAdded appends to plateOrder; PlateRemoved drops it", async () => {
    const mirror = new SceneMirror(async () => unitCubeBuffers());
    await mirror.applySnapshot(emptySnapshot([plateSnap(1)]));
    expect(mirror.plateOrder()).toEqual([1]);

    await mirror.applyEvent({
      kind: "PlateAdded",
      data: { plate_id: 2 },
    });
    expect(mirror.plateOrder()).toEqual([1, 2]);

    await mirror.applyEvent({
      kind: "PlateRemoved",
      data: { plate_id: 2 },
    });
    expect(mirror.plateOrder()).toEqual([1]);
  });

  it("selection on plate 2 doesn't tint plate 1's objects", async () => {
    const mirror = new SceneMirror(async () => unitCubeBuffers());
    await mirror.applySnapshot(
      emptySnapshot([plateSnap(1), plateSnap(2)], 1),
    );
    await mirror.applyEvent({
      kind: "MeshLoaded",
      data: { mesh: unitCubeHeader(1) },
    });
    await mirror.applyEvent({
      kind: "ObjectAdded",
      data: { plate_id: 1, object: sceneObjectAt(101, 1, 0) },
    });
    await mirror.applyEvent({
      kind: "ObjectAdded",
      data: { plate_id: 2, object: sceneObjectAt(201, 1, 0) },
    });
    const baseline = mirror.objectColor(101)!;

    // Select object 201 on plate 2 — plate 1's 101 stays untinted.
    await mirror.applyEvent({
      kind: "SelectionChanged",
      data: { plate_id: 2, selected: [201] },
    });
    expect(mirror.objectColor(101)).toBe(baseline);
  });

  // ---- Project events --------------------------------------------

  it("ProjectSaved updates sourcePath", async () => {
    const mirror = new SceneMirror(async () => unitCubeBuffers());
    await mirror.applySnapshot(emptySnapshot([plateSnap(1)]));
    expect(mirror.sourcePath).toBe(null);
    await mirror.applyEvent({
      kind: "ProjectSaved",
      data: { path: "/tmp/saved.3mf" },
    });
    expect(mirror.sourcePath).toBe("/tmp/saved.3mf");
  });

  // ---- Unknown-plate safety --------------------------------------

  it("event for unknown plate is dropped, not thrown", async () => {
    const mirror = new SceneMirror(async () => unitCubeBuffers());
    await mirror.applySnapshot(emptySnapshot([plateSnap(1)]));
    await mirror.applyEvent({
      kind: "MeshLoaded",
      data: { mesh: unitCubeHeader(1) },
    });
    // Plate 99 doesn't exist; ObjectAdded should warn + drop.
    await mirror.applyEvent({
      kind: "ObjectAdded",
      data: { plate_id: 99, object: sceneObjectAt(999, 1, 0) },
    });
    expect(mirror.hasObjectOnPlate(99, 999)).toBe(false);
    // Mirror still functional.
    await mirror.applyEvent({
      kind: "ObjectAdded",
      data: { plate_id: 1, object: sceneObjectAt(101, 1, 0) },
    });
    expect(mirror.hasObject(101)).toBe(true);
  });
});

// ─── Spool-color paint (PR-S-7) ────────────────────────────────────
//
// `colorForObject` walks: object.extruder_id → plate.materialToSlot
// → printerInstance.extruders[e].slots[s].color. Each link can be
// missing; the resolver falls back to the neutral default so a
// half-bound plate still renders.

function bambiInstance(): PrinterInstance {
  // Hex values match the bundled fixture (Ext=red, AMS:1=black).
  return {
    id: "bambi",
    display_name: "Bambi",
    vendor_profile_ref: "bambu-a1-mini",
    printer_fragment_slug: "bambu-lab-a1-mini",
    default_filament_fragment_slug: "generic-pla",
    default_process_fragment_slug: "0.20mm-standard-bbl-a1m",
    connection: null,
    extruders: [
      {
        label: "",
        installed_nozzle: { diameter_mm: 0.4, material: "stainless" },
        slots: [
          {
            label: "Ext",
            feed: "direct",
            filament_identity: "generic-pla",
            color: "#dc2626",
          },
          {
            label: "AMS:1",
            feed: "ams",
            filament_identity: "generic-pla",
            color: "#111827",
          },
        ],
      },
    ],
    bed: { identity: "Bambu Cool Plate SuperTack" },
    config_overrides: {},
  };
}

/** Plate bound to `bambi` with material 1 routed to (0,0)=Ext red. */
function bambiBoundPlate(): PlateSnapshot {
  return {
    ...plateSnap(1),
    printer_instance_id: "bambi",
    material_to_slot: { 1: { extruder: 0, slot: 0 } },
  };
}

describe("SceneMirror spool-color paint", () => {
  it("paints an object with the bound slot's color when both are present", async () => {
    const mirror = new SceneMirror(async () => unitCubeBuffers());
    mirror.applyPrinterInstance(bambiInstance());
    await mirror.applySnapshot(emptySnapshot([bambiBoundPlate()]));
    await mirror.applyEvent({
      kind: "MeshLoaded",
      data: { mesh: unitCubeHeader(1) },
    });
    await mirror.applyEvent({
      kind: "ObjectAdded",
      data: { plate_id: 1, object: sceneObjectAt(101, 1, 0) },
    });
    expect(mirror.objectColor(101)).toBe(0xdc2626);
  });

  it("recolors live when applyPrinterInstance pushes a new spool color", async () => {
    const mirror = new SceneMirror(async () => unitCubeBuffers());
    mirror.applyPrinterInstance(bambiInstance());
    await mirror.applySnapshot(emptySnapshot([bambiBoundPlate()]));
    await mirror.applyEvent({
      kind: "MeshLoaded",
      data: { mesh: unitCubeHeader(1) },
    });
    await mirror.applyEvent({
      kind: "ObjectAdded",
      data: { plate_id: 1, object: sceneObjectAt(101, 1, 0) },
    });
    // Recolor Ext slot to gold; same instance id so the cache key
    // matches and the bound plate repaints.
    const inst = bambiInstance();
    inst.extruders[0].slots[0].color = "#d4a017";
    mirror.applyPrinterInstance(inst);
    expect(mirror.objectColor(101)).toBe(0xd4a017);
  });

  it("recolors when applyPlateMaterialToSlot reroutes the material to a different slot", async () => {
    const mirror = new SceneMirror(async () => unitCubeBuffers());
    mirror.applyPrinterInstance(bambiInstance());
    await mirror.applySnapshot(emptySnapshot([bambiBoundPlate()]));
    await mirror.applyEvent({
      kind: "MeshLoaded",
      data: { mesh: unitCubeHeader(1) },
    });
    await mirror.applyEvent({
      kind: "ObjectAdded",
      data: { plate_id: 1, object: sceneObjectAt(101, 1, 0) },
    });
    expect(mirror.objectColor(101)).toBe(0xdc2626); // Ext red
    mirror.applyPlateMaterialToSlot(1, { 1: { extruder: 0, slot: 1 } });
    expect(mirror.objectColor(101)).toBe(0x111827); // AMS:1 black
  });

  it("falls back to neutral default when the chain is unbound", async () => {
    const mirror = new SceneMirror(async () => unitCubeBuffers());
    // No applyPrinterInstance call; no material_to_slot on the plate.
    await mirror.applySnapshot(emptySnapshot([plateSnap(1)]));
    await mirror.applyEvent({
      kind: "MeshLoaded",
      data: { mesh: unitCubeHeader(1) },
    });
    await mirror.applyEvent({
      kind: "ObjectAdded",
      data: { plate_id: 1, object: sceneObjectAt(101, 1, 0) },
    });
    expect(mirror.objectColor(101)).toBe(0xb1b1b1); // DEFAULT_COLOR
  });

  it("preserves spool color across selection cycle (deselect restores baseColor, not the default)", async () => {
    const mirror = new SceneMirror(async () => unitCubeBuffers());
    mirror.applyPrinterInstance(bambiInstance());
    await mirror.applySnapshot(emptySnapshot([bambiBoundPlate()]));
    await mirror.applyEvent({
      kind: "MeshLoaded",
      data: { mesh: unitCubeHeader(1) },
    });
    await mirror.applyEvent({
      kind: "ObjectAdded",
      data: { plate_id: 1, object: sceneObjectAt(101, 1, 0) },
    });
    expect(mirror.objectColor(101)).toBe(0xdc2626);
    await mirror.applyEvent({
      kind: "SelectionChanged",
      data: { plate_id: 1, selected: [101] },
    });
    expect(mirror.objectColor(101)).toBe(0x3b82f6); // SELECTED_COLOR
    await mirror.applyEvent({
      kind: "SelectionChanged",
      data: { plate_id: 1, selected: [] },
    });
    expect(mirror.objectColor(101)).toBe(0xdc2626); // restored, not neutral
  });
});
