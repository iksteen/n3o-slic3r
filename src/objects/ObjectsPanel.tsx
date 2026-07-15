// Objects panel — the left workspace column: the active plate's object
// list, with selection, add/remove, per-object material, and grouping.
//
// An object's "material" is its `extruder_id`, resolved to a spool colour
// through the plate's `material_to_slot` table + the bound instance's
// slots — the same routing the Materials section of `SlotBindingPanel`
// uses, so the colours agree across both surfaces. A "group" is a set of
// objects sharing a `group` (3MF multi-volume *and* user grouping are
// the same thing); the panel renders groups with ≥2 members as
// collapsible blocks.

import { useMemo, useState, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import type {
  PlateSnapshot,
  SceneObject,
  ObjectId,
  GroupId,
  PlateId,
} from "../viewport/types";
import {
  type FlatSlotOption,
  type PrinterInstance,
} from "../printer/printerInstance";
import {
  addPrimitive,
  createMaterialForObject,
  deleteObject,
  groupObjects,
  loadModelFromDialog,
  loadModelWithSettingsFromDialog,
  moveObjectsToPlate,
  renameGroup,
  setObjectMaterial,
  ungroupObjects,
  PRIMITIVE_KINDS,
} from "./objectCommands";
import { addPlate } from "../plates/plateCommands";
import { usePlateTabs } from "../plates/usePlateTabs";
import { SendToPlatePicker } from "./SendToPlatePicker";
import { useFilamentCatalog } from "../material/useFilamentCatalog";
import {
  materialOf,
  boundMaterials,
  slotColor,
  slotForMaterial,
  swatchStyle,
} from "../material/materials";
import { MaterialPicker } from "./MaterialPicker";

export interface ObjectsPanelProps {
  plate: PlateSnapshot | null;
  instance: PrinterInstance | null;
  printerName: string;
  /** Build-plate [x, y] mm for the footer; null when unbound. */
  plateSize: [number, number] | null;
  /** Preview mode: selection still works for cross-reference, but the
   *  panel presents read-only (no add/remove/group/material edits). */
  readOnly?: boolean;
}

/** Lowest 1-based material index not yet referenced on the plate, so a
 *  new material reuses a freed gap (e.g. M2 after M2 was removed) rather
 *  than always climbing past the highest. */
function firstAvailableMaterial(used: number[]): number {
  const set = new Set(used);
  let m = 1;
  while (set.has(m)) m++;
  return m;
}

export function ObjectsPanel({
  plate,
  instance,
  printerName,
  plateSize,
  readOnly = false,
}: ObjectsPanelProps) {
  const [showLibrary, setShowLibrary] = useState(false);
  const [materialPicker, setMaterialPicker] = useState<{
    objId: ObjectId;
    rect: DOMRect;
  } | null>(null);
  const [collapsed, setCollapsed] = useState<Set<GroupId>>(() => new Set());
  const [editingGroup, setEditingGroup] = useState<GroupId | null>(null);
  const [sendPicker, setSendPicker] = useState<DOMRect | null>(null);
  const { byIdentity: filamentByIdentity } = useFilamentCatalog();
  const { plates } = usePlateTabs();

  const slots: FlatSlotOption[] = instance?.slots ?? [];
  // boundMaterials, not referencedMaterials: include materials bound only via
  // MMU face-paint (no object carries their extruder_id) so they're pickable.
  const materials = useMemo(() => boundMaterials(plate), [plate]);
  const nextMaterial = firstAvailableMaterial(materials);
  const materialToSlot = plate?.material_to_slot ?? {};
  const groups = plate?.groups ?? {};

  const objects = plate?.objects ?? [];
  const selection = useMemo(
    () => new Set(plate?.selection ?? []),
    [plate?.selection],
  );

  // Members per group; only groups with ≥2 members render as a block.
  const groupMembers = useMemo(() => {
    const m = new Map<GroupId, SceneObject[]>();
    for (const o of objects) {
      if (o.group != null) {
        const arr = m.get(o.group) ?? [];
        arr.push(o);
        m.set(o.group, arr);
      }
    }
    return m;
  }, [objects]);

  // 1-based ordinal per group in appearance order — a friendly fallback
  // label for groups without a name (e.g. multi-volume objects from a 3MF
  // import), since the GroupId itself is an opaque UUID.
  const groupOrdinal = useMemo(() => {
    const m = new Map<GroupId, number>();
    for (const o of objects) {
      if (o.group != null && !m.has(o.group)) m.set(o.group, m.size + 1);
    }
    return m;
  }, [objects]);
  const realGroups = useMemo(
    () =>
      new Set(
        [...groupMembers.entries()]
          .filter(([, mem]) => mem.length >= 2)
          .map(([g]) => g),
      ),
    [groupMembers],
  );

  const colorForMaterial = (material: number): string | null =>
    slotColor(slotForMaterial(material, materialToSlot, slots));
  const overrideCount = (id: ObjectId): number =>
    Object.keys(plate?.object_overrides?.[String(id)] ?? {}).length;

  const sceneSelect = (ids: ObjectId[], mode = "Replace"): void => {
    void invoke("scene_select", { ids, mode }).catch((err) =>
      console.error("[objects] scene_select failed", err),
    );
  };
  const clearSelection = (): void => {
    void invoke("scene_deselect").catch(() => {});
  };
  // The scene selection is the single source of truth — ⌘/Ctrl/Shift
  // toggles membership, a plain click replaces. The action bar + Group
  // act on it, so a single-selected object is already in the set when
  // you ctrl-click a second (no separate grouping state to fall out of
  // sync).
  const handleRowClick = (id: ObjectId, additive: boolean): void => {
    sceneSelect([id], !readOnly && additive ? "Toggle" : "Replace");
  };

  const onAddPrimitive = (kind: (typeof PRIMITIVE_KINDS)[number]): void => {
    setShowLibrary(false);
    void addPrimitive(kind).catch((err) =>
      console.error("[objects] addPrimitive failed", err),
    );
  };
  const onAddModel = (): void => {
    setShowLibrary(false);
    void loadModelFromDialog().catch((err) =>
      console.error("[objects] loadModelFromDialog failed", err),
    );
  };
  const onAddModelWithSettings = (): void => {
    setShowLibrary(false);
    void loadModelWithSettingsFromDialog().catch((err) =>
      console.error("[objects] loadModelWithSettingsFromDialog failed", err),
    );
  };
  const onGroup = (): void => {
    if (selection.size < 2) return;
    const name = `Group ${Object.keys(groups).length + 1}`;
    void (async () => {
      try {
        await groupObjects([...selection], name);
        await invoke("scene_deselect");
      } catch (err) {
        console.error("[objects] group failed", err);
      }
    })();
  };
  // Send the current selection to another plate, keeping each object's
  // authored XYZ. The backend clears the source selection on move, which
  // collapses the selbar (and so this picker) once the snapshot lands.
  const onSendToPlate = (toPlate: PlateId): void => {
    if (!plate || selection.size === 0) return;
    void moveObjectsToPlate(plate.plate_id, toPlate, [...selection]).catch(
      (err) => console.error("[objects] moveObjectsToPlate failed", err),
    );
  };
  const onSendToNewPlate = (): void => {
    if (!plate || selection.size === 0) return;
    const ids = [...selection];
    const from = plate.plate_id;
    void (async () => {
      try {
        // `null` → the new plate inherits the active plate's printer binding.
        const toPlate = await addPlate(null);
        await moveObjectsToPlate(from, toPlate, ids);
      } catch (err) {
        console.error("[objects] send to new plate failed", err);
      }
    })();
  };
  const toggleCollapse = (g: GroupId): void => {
    setCollapsed((prev) => {
      const n = new Set(prev);
      if (n.has(g)) n.delete(g);
      else n.add(g);
      return n;
    });
  };

  const renderRow = (obj: SceneObject, inGroup = false): ReactNode => {
    const material = materialOf(obj);
    const color = colorForMaterial(material);
    const x = Math.round(obj.transform[12] ?? 0);
    const y = Math.round(obj.transform[13] ?? 0);
    const overrides = overrideCount(obj.id);
    return (
      <div
        key={obj.id}
        role="button"
        tabIndex={0}
        className={`objects-item ${selection.has(obj.id) ? "selected" : ""} ${
          inGroup ? "in-group" : ""
        }`}
        onClick={(e) =>
          handleRowClick(obj.id, e.metaKey || e.ctrlKey || e.shiftKey)
        }
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            handleRowClick(obj.id, e.metaKey || e.ctrlKey || e.shiftKey);
          }
        }}
      >
        <div className="objects-item-main">
          <div className="objects-item-name">
            <span className="objects-color-tag" style={swatchStyle(color)} />
            <span className="objects-name-text">{obj.name}</span>
          </div>
          <div className="objects-item-meta">
            {readOnly ? (
              <span
                className="objects-material-badge"
                title={`Material ${material}`}
              >
                M{material}
              </span>
            ) : (
              <button
                className={`objects-material-badge objects-material-badge-btn ${
                  materialPicker?.objId === obj.id ? "open" : ""
                }`}
                title={`Material ${material} — click to change`}
                onClick={(e) => {
                  e.stopPropagation();
                  const rect = e.currentTarget.getBoundingClientRect();
                  setMaterialPicker((p) =>
                    p && p.objId === obj.id ? null : { objId: obj.id, rect },
                  );
                }}
              >
                M{material}
              </button>
            )}
            <span className="dim">
              {x}, {y} mm
            </span>
          </div>
        </div>
        {overrides > 0 && (
          <span
            className="objects-overrides"
            title={`${overrides} per-object override${overrides === 1 ? "" : "s"}`}
          >
            {overrides}
          </span>
        )}
        {!readOnly && (
          <button
            className="objects-remove"
            title="Remove"
            aria-label={`Remove ${obj.name}`}
            onClick={(e) => {
              e.stopPropagation();
              void deleteObject(obj.id).catch((err) =>
                console.error("[objects] deleteObject failed", err),
              );
            }}
          >
            <svg width="10" height="10" viewBox="0 0 12 12" fill="none" aria-hidden>
              <path d="M2.5 2.5l7 7M9.5 2.5l-7 7" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
            </svg>
          </button>
        )}
      </div>
    );
  };

  const renderGroup = (g: GroupId, members: SceneObject[]): ReactNode => {
    // `||`, not `??`: the override write path creates unnamed entries
    // (name "") — an empty label would be an unclickable rename target.
    const name = groups[g]?.name || `Group ${groupOrdinal.get(g) ?? "?"}`;
    const isCollapsed = collapsed.has(g);
    const swatches: string[] = [];
    for (const m of members) {
      const c = colorForMaterial(materialOf(m));
      if (c && !swatches.includes(c)) swatches.push(c);
    }
    return (
      <div key={`g${g}`} className="objects-group">
        <div
          className="objects-group-head"
          onClick={() => sceneSelect(members.map((m) => m.id))}
          title="Select the whole group"
        >
          <button
            className={`objects-group-caret ${isCollapsed ? "collapsed" : ""}`}
            title={isCollapsed ? "Expand group" : "Collapse group"}
            aria-label="Toggle group"
            onClick={(e) => {
              e.stopPropagation();
              toggleCollapse(g);
            }}
          >
            <svg width="9" height="9" viewBox="0 0 10 10" fill="none" aria-hidden>
              <path d="M3 1.5l4 3.5-4 3.5" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" />
            </svg>
          </button>
          {editingGroup === g && !readOnly ? (
            <input
              className="objects-group-name-input"
              defaultValue={name}
              autoFocus
              onClick={(e) => e.stopPropagation()}
              onBlur={(e) => {
                const v = e.target.value.trim();
                if (v && v !== name) void renameGroup(g, v).catch(() => {});
                setEditingGroup(null);
              }}
              onKeyDown={(e) => {
                // Don't let Enter/Escape bubble to row/group handlers.
                e.stopPropagation();
                if (e.key === "Enter") {
                  e.preventDefault();
                  const v = e.currentTarget.value.trim();
                  if (v && v !== name) void renameGroup(g, v).catch(() => {});
                  setEditingGroup(null);
                } else if (e.key === "Escape") {
                  e.preventDefault();
                  setEditingGroup(null);
                }
              }}
            />
          ) : (
            <span
              className="objects-group-name"
              title={readOnly ? name : "Click to rename"}
              onClick={
                readOnly
                  ? undefined
                  : (e) => {
                      e.stopPropagation();
                      setEditingGroup(g);
                    }
              }
            >
              {name}
            </span>
          )}
          <span className="objects-group-count">{members.length}</span>
          <span className="objects-group-swatches">
            {swatches.slice(0, 4).map((c) => (
              <span key={c} className="objects-group-swatch" style={{ background: c }} />
            ))}
          </span>
          {!readOnly && (
            <button
              className="objects-group-ungroup"
              title="Ungroup"
              aria-label="Ungroup"
              onClick={(e) => {
                e.stopPropagation();
                void ungroupObjects(g).catch((err) =>
                  console.error("[objects] ungroupObjects failed", err),
                );
              }}
            >
              <svg width="11" height="11" viewBox="0 0 12 12" fill="none" aria-hidden>
                <rect x="1.5" y="1.5" width="4" height="4" rx="0.8" stroke="currentColor" strokeWidth="1.1" />
                <rect x="6.5" y="6.5" width="4" height="4" rx="0.8" stroke="currentColor" strokeWidth="1.1" />
                <path d="M10.5 1.5l-9 9" stroke="currentColor" strokeWidth="1.1" strokeLinecap="round" />
              </svg>
            </button>
          )}
        </div>
        {!isCollapsed && (
          <div className="objects-group-members">
            {members.map((m) => renderRow(m, true))}
          </div>
        )}
      </div>
    );
  };

  // Walk objects in order; emit a group block the first time one of its
  // (≥2-member) members is seen, otherwise a single row.
  const renderList: ReactNode[] = [];
  const seenGroups = new Set<GroupId>();
  for (const obj of objects) {
    if (obj.group != null && realGroups.has(obj.group)) {
      if (seenGroups.has(obj.group)) continue;
      seenGroups.add(obj.group);
      renderList.push(renderGroup(obj.group, groupMembers.get(obj.group)!));
    } else {
      renderList.push(renderRow(obj));
    }
  }

  return (
    <aside className={`objects-panel ${readOnly ? "readonly" : ""}`}>
      <div className="objects-panel-head">
        <h3 title={printerName}>
          Plate · <span className="objects-panel-printer">{printerName}</span>
        </h3>
        {!readOnly && (
          <div className="objects-add">
            <button
              className="objects-add-btn"
              title="Add object"
              aria-label="Add object"
              onClick={() => setShowLibrary((s) => !s)}
            >
              <svg width="13" height="13" viewBox="0 0 14 14" fill="none" aria-hidden>
                <path d="M7 2v10M2 7h10" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
              </svg>
            </button>
            {showLibrary && (
              <>
                <div className="objects-add-backdrop" onClick={() => setShowLibrary(false)} />
                <div className="objects-add-menu" role="menu">
                  <div className="objects-add-section">Primitives</div>
                  {PRIMITIVE_KINDS.map((k) => (
                    <button
                      key={k}
                      className="objects-add-item"
                      role="menuitem"
                      onClick={() => onAddPrimitive(k)}
                    >
                      {k}
                    </button>
                  ))}
                  <div className="objects-add-sep" />
                  <button className="objects-add-item" role="menuitem" onClick={onAddModel}>
                    Add model…
                  </button>
                  <button
                    className="objects-add-item"
                    role="menuitem"
                    onClick={onAddModelWithSettings}
                    title="Load a 3MF's models with their print settings applied as object overrides"
                  >
                    Add model + settings…
                  </button>
                </div>
              </>
            )}
          </div>
        )}
      </div>

      {!readOnly && selection.size >= 1 && (
        <div className="objects-selbar">
          <span className="objects-selbar-count">{selection.size} selected</span>
          <div className="objects-selbar-actions">
            {selection.size >= 2 && (
              <button
                className="objects-selbar-btn primary"
                title="Group selected into one object"
                onClick={onGroup}
              >
                Group
              </button>
            )}
            <button
              className={`objects-selbar-btn ${sendPicker ? "open" : ""}`}
              title="Send selected to another plate"
              onClick={(e) => {
                const rect = e.currentTarget.getBoundingClientRect();
                setSendPicker((p) => (p ? null : rect));
              }}
            >
              Send to ▾
            </button>
            <button className="objects-selbar-btn" onClick={clearSelection}>
              Clear
            </button>
          </div>
        </div>
      )}

      <div className="objects-list">
        {objects.length === 0 ? (
          <div className="objects-empty">
            {readOnly
              ? "No objects on this plate."
              : "No objects yet — click + to add one."}
          </div>
        ) : (
          renderList
        )}
      </div>

      <div className="objects-panel-foot">
        <div className="objects-stat">
          <span className="k">Build plate</span>
          <span className="v">
            {plateSize
              ? `${Math.round(plateSize[0])} × ${Math.round(plateSize[1])} mm`
              : "—"}
          </span>
        </div>
        <div className="objects-stat">
          <span className="k">Objects</span>
          <span className="v">{objects.length}</span>
        </div>
      </div>

      {!readOnly && sendPicker && plate && selection.size > 0 && (
        <SendToPlatePicker
          count={selection.size}
          plates={plates}
          currentPlateId={plate.plate_id}
          anchorRect={sendPicker}
          onSend={onSendToPlate}
          onSendNew={onSendToNewPlate}
          onClose={() => setSendPicker(null)}
        />
      )}

      {!readOnly &&
        materialPicker &&
        plate &&
        (() => {
          const obj = objects.find((o) => o.id === materialPicker.objId);
          if (!obj) return null;
          return (
            <MaterialPicker
              objectName={obj.name}
              currentMaterial={obj.extruder_id ?? 1}
              materials={materials}
              nextMaterial={nextMaterial}
              slots={slots}
              materialToSlot={materialToSlot}
              filamentByIdentity={filamentByIdentity}
              allowCreate={objects.length > 1}
              anchorRect={materialPicker.rect}
              onAssign={(m) => {
                void setObjectMaterial(obj.id, m).catch((err) =>
                  console.error("[objects] setObjectMaterial failed", err),
                );
              }}
              onCreate={(m, slot) => {
                void createMaterialForObject(
                  plate.plate_id,
                  obj.id,
                  m,
                  slot,
                ).catch((err) =>
                  console.error("[objects] createMaterialForObject failed", err),
                );
              }}
              onClose={() => setMaterialPicker(null)}
            />
          );
        })()}
    </aside>
  );
}
