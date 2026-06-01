// Objects panel — the left workspace column: the active plate's object
// list, with selection, add/remove, per-object material, and grouping.
//
// An object's "material" is its `extruder_id`, resolved to a spool colour
// through the plate's `material_to_slot` table + the bound instance's
// slots — the same routing the Materials section of `SlotBindingPanel`
// uses, so the colours agree across both surfaces. A "group" is a set of
// objects sharing a `group_id` (3MF multi-volume *and* user grouping are
// the same thing); the panel renders groups with ≥2 members as
// collapsible blocks.

import { useMemo, useState, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { PlateSnapshot, SceneObject, ObjectId } from "../viewport/types";
import {
  flattenSlots,
  type FlatSlotOption,
  type PrinterInstance,
} from "../printer/printerInstance";
import {
  addPrimitive,
  createMaterialForObject,
  deleteObject,
  groupObjects,
  loadModelFromDialog,
  renameGroup,
  setObjectMaterial,
  ungroupObjects,
  PRIMITIVE_KINDS,
} from "./objectCommands";
import { referencedMaterials } from "../material/SlotBindingPanel";
import { useFilamentCatalog } from "../material/useFilamentCatalog";
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

/** An object's material index — its `extruder_id`, defaulting to 1
 *  (unassigned inherits material 1), matching `referencedMaterials`. */
function materialOf(obj: SceneObject): number {
  return obj.extruder_id ?? 1;
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
  const [collapsed, setCollapsed] = useState<Set<number>>(() => new Set());
  const [editingGroup, setEditingGroup] = useState<number | null>(null);
  const { byIdentity: filamentByIdentity } = useFilamentCatalog();

  const slots = useMemo<FlatSlotOption[]>(
    () => (instance ? flattenSlots(instance) : []),
    [instance],
  );
  const materials = useMemo(() => referencedMaterials(plate), [plate]);
  const nextMaterial = firstAvailableMaterial(materials);
  const materialToSlot = plate?.material_to_slot ?? {};
  const groupNames = plate?.group_names ?? {};

  const objects = plate?.objects ?? [];
  const selection = useMemo(
    () => new Set(plate?.selection ?? []),
    [plate?.selection],
  );

  // Members per group_id; only groups with ≥2 members render as a block.
  const groupMembers = useMemo(() => {
    const m = new Map<number, SceneObject[]>();
    for (const o of objects) {
      if (o.group_id != null) {
        const arr = m.get(o.group_id) ?? [];
        arr.push(o);
        m.set(o.group_id, arr);
      }
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

  // Material index → routed slot's spool colour, only when a filament is
  // actually loaded (a cached colour with no identity reads as empty).
  const colorForMaterial = (material: number): string | null => {
    const pick = plate?.material_to_slot?.[material];
    if (!pick) return null;
    const slot = slots.find(
      (s) => s.ref.extruder === pick.extruder && s.ref.slot === pick.slot,
    );
    return slot?.filament_identity ? slot.color : null;
  };
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
  const onGroup = (): void => {
    if (selection.size < 2) return;
    const name = `Group ${Object.keys(groupNames).length + 1}`;
    void (async () => {
      try {
        await groupObjects([...selection], name);
        await invoke("scene_deselect");
      } catch (err) {
        console.error("[objects] group failed", err);
      }
    })();
  };
  const toggleCollapse = (g: number): void => {
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
            <span
              className="objects-color-tag"
              style={{
                background: color ?? "transparent",
                border: color ? "none" : "1px dashed var(--text-muted)",
              }}
            />
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

  const renderGroup = (g: number, members: SceneObject[]): ReactNode => {
    const name = groupNames[g] ?? `Group ${g}`;
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
                if (v) void renameGroup(g, v).catch(() => {});
                setEditingGroup(null);
              }}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  const v = (e.target as HTMLInputElement).value.trim();
                  if (v) void renameGroup(g, v).catch(() => {});
                  setEditingGroup(null);
                }
                if (e.key === "Escape") setEditingGroup(null);
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
            {swatches.slice(0, 4).map((c, i) => (
              <span key={i} className="objects-group-swatch" style={{ background: c }} />
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
  const seenGroups = new Set<number>();
  for (const obj of objects) {
    if (obj.group_id != null && realGroups.has(obj.group_id)) {
      if (seenGroups.has(obj.group_id)) continue;
      seenGroups.add(obj.group_id);
      renderList.push(renderGroup(obj.group_id, groupMembers.get(obj.group_id)!));
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
                </div>
              </>
            )}
          </div>
        )}
      </div>

      {!readOnly && selection.size >= 2 && (
        <div className="objects-selbar">
          <span className="objects-selbar-count">{selection.size} selected</span>
          <div className="objects-selbar-actions">
            <button
              className="objects-selbar-btn primary"
              title="Group selected into one object"
              onClick={onGroup}
            >
              Group
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
