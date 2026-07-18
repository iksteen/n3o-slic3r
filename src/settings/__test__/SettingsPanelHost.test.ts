// Host projection + visibility-toggle persistence tests.
//
// The host renders SettingsPanel; the full render path needs a DOM
// (we don't have one set up). What's testable in pure form is the
// projection: snapshot → active plate → selected object → panel
// shape, plus the localStorage-backed visibility toggle.

import { describe, expect, it } from "vitest";
import {
  activePlate,
  allObjectsForPanel,
  effectiveObjectOverrides,
  selectedObject,
} from "../SettingsPanelHost";
import type { ProjectSession } from "../../project/useProjectSession";
import { SESSION_EVENT_NAMES } from "../../project/useProjectSession";
import type {
  PlateSnapshot,
  SceneObject,
  SceneSnapshot,
} from "../../viewport/types";


function obj(id: number, name: string, group: string | null = null): SceneObject {
  return {
    id,
    mesh: 1,
    transform: [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1],
    name,
    visible: true,
    extruder_id: null,
    group,
  };
}

function plate(opts: {
  id: number;
  objects?: SceneObject[];
  selection?: number[];
  object_overrides?: Record<number, Record<string, string>>;
  groups?: PlateSnapshot["groups"];
}): PlateSnapshot {
  return {
    plate_id: opts.id,
    name: `Plate ${opts.id}`,
    printer_identity: null,
    printer_instance_id: null,
    material_to_slot: {},
    project_overrides: {},
    quality_profile: null,
    objects: opts.objects ?? [],
    selection: opts.selection ?? [],
    exclusion_zones: [],
    bed: null,
    object_overrides: opts.object_overrides ?? {},
    groups: opts.groups ?? {},
  };
}

function snap(plates: PlateSnapshot[], activeId: number): SceneSnapshot {
  return {
    project_uuid: "test",
    source_path: null,
    recovery_origin: null,
    user_overrides: {},
    file_metadata: {},
    meshes: [],
    plates,
    active_plate_id: activeId,
  };
}

function session(overrides: Partial<ProjectSession> = {}): ProjectSession {
  return {
    cascadeHandle: null,
    snapshot: null,
    dirty: false,
    loading: false,
    error: null,
    ...overrides,
  };
}

describe("activePlate", () => {
  it("returns null when the snapshot hasn't loaded", () => {
    expect(activePlate(session())).toBeNull();
  });

  it("finds the plate matching active_plate_id", () => {
    const s = session({
      snapshot: snap([plate({ id: 1 }), plate({ id: 2 })], 2),
    });
    expect(activePlate(s)?.plate_id).toBe(2);
  });

  it("returns null when active_plate_id refers to a missing plate (defensive)", () => {
    const s = session({
      snapshot: snap([plate({ id: 1 })], 99),
    });
    expect(activePlate(s)).toBeNull();
  });
});

describe("selectedObject", () => {
  it("returns null on a null plate", () => {
    expect(selectedObject(null)).toBeNull();
  });

  it("returns null when selection is empty", () => {
    expect(selectedObject(plate({ id: 1 }))).toBeNull();
  });

  it("projects the first selected object to {id, name, kind}", () => {
    const p = plate({
      id: 1,
      objects: [obj(101, "Body"), obj(102, "Lid")],
      selection: [102, 101],
    });
    expect(selectedObject(p)).toEqual({ id: 102, name: "Lid", kind: "object" });
  });

  it("returns null when the selected id isn't on the plate (race protection)", () => {
    const p = plate({
      id: 1,
      objects: [obj(101, "Body")],
      selection: [999],
    });
    expect(selectedObject(p)).toBeNull();
  });

  it("presents a whole-group selection as the group", () => {
    const p = plate({
      id: 1,
      objects: [obj(101, "Body", "g1"), obj(102, "Logo", "g1"), obj(103, "Solo")],
      selection: [101, 102],
      groups: { g1: { name: "Case", overrides: {} } },
    });
    expect(selectedObject(p)).toEqual({ id: 101, name: "Case", kind: "group" });
  });

  it("falls back to the Objects-panel ordinal for unnamed groups", () => {
    const p = plate({
      id: 1,
      objects: [
        obj(100, "A", "g0"),
        obj(101, "B", "g0"),
        obj(102, "Body", "g1"),
        obj(103, "Logo", "g1"),
      ],
      selection: [102, 103],
    });
    expect(selectedObject(p)).toEqual({ id: 102, name: "Group 2", kind: "group" });
  });

  it("presents a single grouped member picked alone as that object", () => {
    const p = plate({
      id: 1,
      objects: [obj(101, "Body", "g1"), obj(102, "Logo", "g1")],
      selection: [101],
      groups: { g1: { name: "Case", overrides: {} } },
    });
    expect(selectedObject(p)).toEqual({ id: 101, name: "Body", kind: "object" });
  });
});

describe("allObjectsForPanel", () => {
  it("merges per-object overrides into each entry", () => {
    const p = plate({
      id: 1,
      objects: [obj(101, "Body"), obj(102, "Lid")],
      object_overrides: { 101: { layer_height: "0.12" } },
    });
    expect(allObjectsForPanel(p)).toEqual([
      {
        id: 101,
        name: "Body",
        color: null,
        overrides: { layer_height: "0.12" },
      },
      { id: 102, name: "Lid", color: null, overrides: {} },
    ]);
  });

  it("returns an empty list on a null plate", () => {
    expect(allObjectsForPanel(null)).toEqual([]);
  });

  it("folds group overrides into every member's map", () => {
    const p = plate({
      id: 1,
      objects: [obj(101, "Body", "g1"), obj(102, "Lid", "g1"), obj(103, "Solo")],
      object_overrides: { 101: { wall_loops: "4" } },
      groups: { g1: { name: "Case", overrides: { enable_support: "1" } } },
    });
    expect(allObjectsForPanel(p).map((o) => o.overrides)).toEqual([
      { wall_loops: "4", enable_support: "1" },
      { enable_support: "1" },
      {},
    ]);
  });
});

describe("effectiveObjectOverrides", () => {
  it("merges the member's map with its group's, group value winning", () => {
    const p = plate({
      id: 1,
      objects: [obj(101, "Body", "g1")],
      // A stale member-stored copy (legacy project) loses to the group.
      object_overrides: { 101: { enable_support: "0", wall_loops: "4" } },
      groups: { g1: { name: "Case", overrides: { enable_support: "1" } } },
    });
    expect(effectiveObjectOverrides(p, 101)).toEqual({
      enable_support: "1",
      wall_loops: "4",
    });
  });

  it("is just the member's map for solos and unnamed groups", () => {
    const p = plate({
      id: 1,
      objects: [obj(101, "Body", "g-unnamed"), obj(102, "Solo")],
      object_overrides: { 102: { wall_loops: "2" } },
    });
    expect(effectiveObjectOverrides(p, 101)).toEqual({});
    expect(effectiveObjectOverrides(p, 102)).toEqual({ wall_loops: "2" });
  });
});

describe("SESSION_EVENT_NAMES", () => {
  it("covers everything the panel host needs (override channels + plate flow)", () => {
    // If any of these go missing the panel context goes stale —
    // pin the floor.
    const required = [
      "scene:plate_added",
      "scene:plate_removed",
      "scene:active_plate_changed",
      "scene:plate_changed",
      "scene:object_added",
      "scene:object_removed",
      "scene:object_updated",
      "scene:selection_changed",
      "scene:object_overrides_changed",
      "scene:project_overrides_changed",
      "scene:bed_changed",
      "project:loaded",
    ];
    for (const name of required) {
      expect(SESSION_EVENT_NAMES).toContain(name);
    }
  });
});
