// ObjectsPanel.jsx — left panel: object library + plate object list.

const { useState: useStateOP, useRef: useRefOP, useEffect: useEffectOP } = React;
const {
  resolveObjectFilament: resolveOP,
  slotShortLabel: slotShortOP,
  slotLongLabel: slotLongOP,
} = window.SLICER_DATA;

const OBJECT_LIBRARY = [
  { section: "Primitives", items: [
    { kind: "cube",     name: "Cube",     meta: "20 mm" },
    { kind: "cylinder", name: "Cylinder", meta: "Ø24 × 30" },
    { kind: "sphere",   name: "Sphere",   meta: "Ø28" },
    { kind: "cone",     name: "Cone",     meta: "Ø28 × 30" },
    { kind: "torus",    name: "Torus",    meta: "Ø28 / 4" },
  ]},
  { section: "Calibration", items: [
    { kind: "calicube",  name: "calibration_cube.stl",   meta: "20×20×20" },
    { kind: "temptower", name: "temp_tower.stl",         meta: "28×80×18" },
    { kind: "benchy",    name: "boat_test.stl",          meta: "60×18×20" },
  ]},
  { section: "Imported", items: [
    { kind: "stl_mount",   name: "front_mount_v3.stl",    meta: "45×30×12" },
    { kind: "stl_bracket", name: "fan_bracket_r2.stl",    meta: "36×8×28"  },
  ]},
];

// Natural-sort material ids so M2 precedes M10.
function sortMaterialIds(ids) {
  return [...ids].sort((a, b) => {
    const na = parseInt((a.match(/\d+/) || [0])[0], 10);
    const nb = parseInt((b.match(/\d+/) || [0])[0], 10);
    return na - nb || a.localeCompare(b);
  });
}

// Floating picker anchored (fixed-position, so it escapes the object list's
// scroll clip) under an object's material badge. Two views:
//   1. "Assign" — pick any existing project material (M1, M2…); each row shows
//      where that material is routed and the filament colour/label.
//   2. "Create" — mint a new material and route it to any loaded slot.
function MaterialPicker({
  obj, materialMap, slotMap, filaments, slotIds,
  anchorRect, onAssign, onCreate, onClose,
}) {
  const [creating, setCreating] = useStateOP(false);
  const menuRef = useRefOP(null);

  useEffectOP(() => {
    const onDoc = (e) => {
      if (menuRef.current && !menuRef.current.contains(e.target)) onClose();
    };
    const onKey = (e) => { if (e.key === "Escape") onClose(); };
    document.addEventListener("mousedown", onDoc);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDoc);
      document.removeEventListener("keydown", onKey);
    };
  }, []);

  const materials = sortMaterialIds(Object.keys(materialMap || {}));

  // Clamp the menu within the viewport. Width is fixed by CSS (220px).
  const MENU_W = 224;
  const style = { position: "fixed" };
  if (anchorRect) {
    const left = Math.min(anchorRect.left, window.innerWidth - MENU_W - 8);
    style.left = Math.max(8, left);
    // Prefer below the badge; flip above if it would overflow the viewport.
    const estH = creating ? 40 + slotIds.length * 34 : 64 + materials.length * 34;
    style.top = (anchorRect.bottom + estH > window.innerHeight - 8)
      ? Math.max(8, anchorRect.top - estH - 4)
      : anchorRect.bottom + 4;
  }

  const swatchOf = (slotId) => {
    const fid = slotId ? (slotMap || {})[slotId] : null;
    return fid ? filaments.find(x => x.id === fid) : null;
  };

  return (
    <div
      ref={menuRef}
      className="printer-picker-menu material-picker-menu"
      style={style}
      onClick={(e) => e.stopPropagation()}
    >
      {!creating ? (
        <React.Fragment>
          <div className="ptpm-title">Material · {obj.name}</div>
          {materials.map(mid => {
            const slotId = (materialMap || {})[mid];
            const f = swatchOf(slotId);
            const active = obj.materialId === mid;
            return (
              <button
                key={mid}
                className={`ptpm-item ptpm-row mp-item ${active ? "active" : ""}`}
                onClick={() => { onAssign(mid); onClose(); }}
              >
                <span className="ptpm-name">
                  <span className="ptpm-swatch" style={{ background: f?.color || "transparent", border: f ? "none" : "1px dashed currentColor" }}/>
                  <span className="mp-mid">{mid}</span>
                  <span className="mp-arrow">→</span>
                  <span className="mp-slot">{slotId ? slotShortOP(slotId, slotIds) : "—"}</span>
                </span>
                <span className="ptpm-detail">{f ? f.label : "unmapped"}</span>
              </button>
            );
          })}
          <div className="ptpm-sep"/>
          <button className="ptpm-item ptpm-add" onClick={() => setCreating(true)}>
            <span className="ptpm-name">+ New material</span>
            <span className="ptpm-detail">from slot…</span>
          </button>
        </React.Fragment>
      ) : (
        <React.Fragment>
          <button className="ptpm-title mp-back" onClick={() => setCreating(false)}>
            <svg width="9" height="9" viewBox="0 0 10 10" fill="none" aria-hidden="true">
              <path d="M6.5 1.5l-3.5 3.5 3.5 3.5" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round"/>
            </svg>
            New material — route to slot
          </button>
          {slotIds.map(sid => {
            const f = swatchOf(sid);
            return (
              <button
                key={sid}
                className="ptpm-item ptpm-row mp-item"
                onClick={() => { onCreate(sid); onClose(); }}
              >
                <span className="ptpm-name">
                  <span className="ptpm-swatch" style={{ background: f?.color || "transparent", border: f ? "none" : "1px dashed currentColor" }}/>
                  {slotLongOP(sid)}
                </span>
                <span className="ptpm-detail">{f ? f.label : "empty"}</span>
              </button>
            );
          })}
        </React.Fragment>
      )}
    </div>
  );
}

function ObjectsPanel({
  objects, setObjects,
  selectedId, setSelectedId,
  filaments,
  slotMap,
  materialMap,
  slotIds = [],
  setObjectMaterial,
  createMaterialForObject,
  printerName,
  countObjectOverrides,
  plateSize,
  // Read-only mode (Preview): hides the add button, remove buttons, and
  // library popover. Objects can still be clicked to select for visual
  // cross-reference with the sliced canvas.
  readOnly = false,
}) {
  const [showLibrary, setShowLibrary] = useStateOP(false);
  // Multi-selection used purely for grouping operations. The single-select
  // (`selectedId`, owned by the app) still drives the 3D highlight + settings
  // "object" context; this set rides alongside it for batch actions.
  const [multiSel, setMultiSel] = useStateOP(() => new Set());
  // Which groups are collapsed in the list (local view state).
  const [collapsed, setCollapsed] = useStateOP(() => new Set());
  // Group whose name is currently being edited inline.
  const [editingGroup, setEditingGroup] = useStateOP(null);
  // Which object's material picker is open: { objId, rect } or null.
  const [materialPicker, setMaterialPicker] = useStateOP(null);
  const draggingRef = useRefOP(null);

  const handleDragStart = (e, item) => {
    const payload = {
      kind: item.kind,
      name: item.name,
      materialId: "M1",
    };
    e.dataTransfer.setData("application/json", JSON.stringify(payload));
    e.dataTransfer.effectAllowed = "copy";
    // ghost
    const ghost = document.createElement("div");
    ghost.className = "drag-preview";
    ghost.textContent = item.name;
    ghost.style.left = "-9999px";
    document.body.appendChild(ghost);
    e.dataTransfer.setDragImage(ghost, 0, 0);
    setTimeout(() => ghost.remove(), 0);
  };

  const addItem = (item) => {
    // Place it at a tidy grid slot
    const id = `obj_${Date.now()}_${Math.floor(Math.random() * 999)}`;
    const n = objects.length;
    const cols = 3;
    const spacing = 50;
    const x = ((n % cols) - 1) * spacing;
    const y = (Math.floor(n / cols) - 1) * spacing;
    setObjects(prev => [...prev, {
      id, name: item.name, kind: item.kind,
      x, y, rotZ: 0,
      materialId: "M1",
      groupId: null,
      overrides: {},
    }]);
    setSelectedId(id);
    setShowLibrary(false);
  };

  // After any removal, dissolve groups that would be left with fewer than two
  // members — a "group of one" isn't a group.
  const dissolveOrphanGroups = (list) => {
    const counts = {};
    list.forEach(o => { if (o.groupId) counts[o.groupId] = (counts[o.groupId] || 0) + 1; });
    return list.map(o => (o.groupId && counts[o.groupId] < 2)
      ? { ...o, groupId: null, groupName: undefined }
      : o);
  };

  const removeObject = (id, e) => {
    e?.stopPropagation();
    setObjects(prev => dissolveOrphanGroups(prev.filter(o => o.id !== id)));
    if (selectedId === id) setSelectedId(null);
    setMultiSel(prev => { const n = new Set(prev); n.delete(id); return n; });
  };

  // ── Selection ────────────────────────────────────────────────────────────
  const handleRowClick = (e, id) => {
    if (readOnly) { setSelectedId(id); return; }
    if (e.metaKey || e.ctrlKey || e.shiftKey) {
      // Toggle multi-selection without disturbing the primary selection.
      e.preventDefault();
      setMultiSel(prev => {
        const n = new Set(prev);
        n.has(id) ? n.delete(id) : n.add(id);
        return n;
      });
    } else {
      setMultiSel(new Set());
      setSelectedId(id);
    }
  };

  // Clicking a group header selects all its members (multi-select).
  const selectGroup = (members) => {
    if (readOnly) return;
    setMultiSel(new Set(members.map(m => m.id)));
    setSelectedId(members[0].id);
  };

  // ── Grouping actions ───────────────────────────────────────────────────────
  const existingGroupCount = new Set(objects.filter(o => o.groupId).map(o => o.groupId)).size;

  const groupSelected = () => {
    if (multiSel.size < 2) return;
    const gid = `grp_${Date.now().toString(36)}`;
    const name = `Group ${existingGroupCount + 1}`;
    setObjects(prev => prev.map(o => multiSel.has(o.id)
      ? { ...o, groupId: gid, groupName: name }
      : o));
    setMultiSel(new Set());
  };

  const ungroup = (gid, e) => {
    e?.stopPropagation();
    setObjects(prev => prev.map(o => o.groupId === gid
      ? { ...o, groupId: null, groupName: undefined }
      : o));
  };

  const renameGroup = (gid, name) => {
    setObjects(prev => prev.map(o => o.groupId === gid ? { ...o, groupName: name } : o));
  };

  const toggleCollapse = (gid) => {
    setCollapsed(prev => { const n = new Set(prev); n.has(gid) ? n.delete(gid) : n.add(gid); return n; });
  };

  // ── Build the grouped render order ─────────────────────────────────────────
  // Walk objects in their natural order; the first time a grouped object is
  // seen, emit the whole group (all members, in order) at that position.
  const renderList = [];
  const seenGroups = new Set();
  for (const obj of objects) {
    if (obj.groupId) {
      if (seenGroups.has(obj.groupId)) continue;
      seenGroups.add(obj.groupId);
      const members = objects.filter(o => o.groupId === obj.groupId);
      renderList.push({ type: "group", groupId: obj.groupId, name: members[0].groupName || "Group", members });
    } else {
      renderList.push({ type: "single", obj });
    }
  }

  // Render a single object row (used both standalone and inside a group).
  const renderRow = (obj, { inGroup = false } = {}) => {
    const { materialId, filament } = resolveOP(obj, materialMap, slotMap, filaments);
    const color = (filament && filament.color) || "#888";
    const filLabel = (filament && filament.label) || "unassigned";
    const overrideCount = countObjectOverrides(obj.id);
    const isPrimary = selectedId === obj.id;
    const isMulti = multiSel.has(obj.id);
    return (
      <div
        key={obj.id}
        className={`object-item ${isPrimary ? "selected" : ""} ${isMulti ? "multi" : ""} ${inGroup ? "in-group" : ""}`}
        onClick={(e) => handleRowClick(e, obj.id)}
      >
        <div className="object-thumb">{obj.kind.slice(0, 2).toUpperCase()}</div>
        <div style={{ minWidth: 0 }}>
          <div className="object-name">
            <span className="object-color-tag" style={{ background: color }}/>
            {obj.name}
          </div>
          <div className="object-meta">
            {readOnly ? (
              <span className="object-material-badge" title={`Material ${materialId} → ${filLabel}`}>{materialId || "M?"}</span>
            ) : (
              <button
                className={`object-material-badge object-material-badge-btn ${materialPicker && materialPicker.objId === obj.id ? "open" : ""}`}
                title={`Material ${materialId || "—"} → ${filLabel}\nClick to assign or create a material`}
                onClick={(e) => {
                  e.stopPropagation();
                  const rect = e.currentTarget.getBoundingClientRect();
                  setMaterialPicker(prev => prev && prev.objId === obj.id ? null : { objId: obj.id, rect });
                }}
              >
                {materialId || "M?"}
                <svg className="mp-badge-chev" width="7" height="7" viewBox="0 0 10 10" fill="none" aria-hidden="true">
                  <path d="M2 3.5L5 6.5l3-3" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round"/>
                </svg>
              </button>
            )}
            <span>{Math.round(obj.x)}, {Math.round(obj.y)} mm</span>
            <span className="dim">· {filLabel}</span>
          </div>
        </div>
        <div style={{ display: "flex", gap: 4, alignItems: "center" }}>
          {overrideCount > 0 && (
            <span className="object-overrides" title={`${overrideCount} per-object overrides`}>
              {overrideCount}
            </span>
          )}
          {!readOnly && (
            <button
              className="icon-btn"
              title="Remove"
              onClick={(e) => removeObject(obj.id, e)}
            >
              <svg width="10" height="10" viewBox="0 0 12 12" fill="none">
                <path d="M2.5 2.5l7 7M9.5 2.5l-7 7" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round"/>
              </svg>
            </button>
          )}
        </div>
      </div>
    );
  };

  return (
    <aside className={`objects-panel ${readOnly ? "readonly" : ""}`}>
      <div className="panel-head">
        <h3>Plate · {printerName}</h3>
        {readOnly ? (
          <span className="panel-head-readonly-tag" title="Preview mode — switch to Prepare to edit">
            <svg width="10" height="10" viewBox="0 0 12 12" fill="none">
              <rect x="2" y="5" width="8" height="6" rx="1" stroke="currentColor" strokeWidth="1.2"/>
              <path d="M4 5V3.5a2 2 0 0 1 4 0V5" stroke="currentColor" strokeWidth="1.2"/>
            </svg>
            Read-only
          </span>
        ) : (
          <button className="icon-btn" title="Add object" onClick={() => setShowLibrary(s => !s)}>
            <svg width="12" height="12" viewBox="0 0 12 12" fill="none">
              <path d="M6 1.5v9M1.5 6h9" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round"/>
            </svg>
          </button>
        )}
      </div>

      {/* Multi-select action bar — appears when 2+ objects are picked via
          ⌘/Ctrl/Shift-click, offering to combine them into a group. */}
      {!readOnly && multiSel.size > 0 && (
        <div className="object-selbar">
          <span className="object-selbar-count">{multiSel.size} selected</span>
          <div className="object-selbar-actions">
            <button
              className="object-selbar-btn primary"
              disabled={multiSel.size < 2}
              title={multiSel.size < 2 ? "Select at least two objects" : "Group selected objects"}
              onClick={groupSelected}
            >
              <svg width="11" height="11" viewBox="0 0 12 12" fill="none">
                <rect x="1.5" y="1.5" width="5" height="5" rx="1" stroke="currentColor" strokeWidth="1.2"/>
                <rect x="5.5" y="5.5" width="5" height="5" rx="1" stroke="currentColor" strokeWidth="1.2" fill="var(--surface)"/>
              </svg>
              Group
            </button>
            <button className="object-selbar-btn" title="Clear selection" onClick={() => setMultiSel(new Set())}>
              Clear
            </button>
          </div>
        </div>
      )}

      <div className="object-list">
        {objects.length === 0 && !readOnly && (
          <div className="empty-state">
            Drag an object from the library<br/>or click <span className="kbd-inline">+</span> to add one.
          </div>
        )}
        {objects.length === 0 && readOnly && (
          <div className="empty-state">
            No objects on this plate.
          </div>
        )}
        {renderList.map(entry => {
          if (entry.type === "single") return renderRow(entry.obj);

          // ── Group block ──
          const { groupId, name, members } = entry;
          const isCollapsed = collapsed.has(groupId);
          const allSelected = members.every(m => multiSel.has(m.id));
          // Distinct member colors for the header swatch stack.
          const swatches = [];
          members.forEach(m => {
            const { filament } = resolveOP(m, materialMap, slotMap, filaments);
            const c = (filament && filament.color) || "#888";
            if (!swatches.includes(c)) swatches.push(c);
          });
          return (
            <div key={groupId} className={`object-group ${allSelected ? "selected" : ""}`}>
              <div
                className="object-group-head"
                onClick={() => selectGroup(members)}
                title="Click to select the whole group"
              >
                <button
                  className={`object-group-caret ${isCollapsed ? "collapsed" : ""}`}
                  onClick={(e) => { e.stopPropagation(); toggleCollapse(groupId); }}
                  title={isCollapsed ? "Expand group" : "Collapse group"}
                >
                  <svg width="9" height="9" viewBox="0 0 10 10" fill="none">
                    <path d="M3 1.5l4 3.5-4 3.5" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round"/>
                  </svg>
                </button>
                <svg className="object-group-icon" width="12" height="12" viewBox="0 0 14 14" fill="none">
                  <rect x="1.5" y="1.5" width="6" height="6" rx="1.2" stroke="currentColor" strokeWidth="1.2"/>
                  <rect x="6.5" y="6.5" width="6" height="6" rx="1.2" stroke="currentColor" strokeWidth="1.2" fill="var(--surface)"/>
                </svg>
                {editingGroup === groupId && !readOnly ? (
                  <input
                    className="object-group-name-input"
                    defaultValue={name}
                    autoFocus
                    onClick={(e) => e.stopPropagation()}
                    onBlur={(e) => { renameGroup(groupId, e.target.value.trim() || name); setEditingGroup(null); }}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") { renameGroup(groupId, e.target.value.trim() || name); setEditingGroup(null); }
                      if (e.key === "Escape") setEditingGroup(null);
                    }}
                  />
                ) : (
                  <span
                    className="object-group-name"
                    onClick={readOnly ? undefined : (e) => { e.stopPropagation(); setEditingGroup(groupId); }}
                    title={readOnly ? name : "Click to rename group"}
                  >
                    {name}
                  </span>
                )}
                <span className="object-group-count">{members.length}</span>
                <span className="object-group-swatches">
                  {swatches.slice(0, 4).map((c, i) => (
                    <span key={i} className="object-group-swatch" style={{ background: c }}/>
                  ))}
                </span>
                {!readOnly && (
                  <button
                    className="object-group-ungroup icon-btn"
                    title="Ungroup"
                    onClick={(e) => ungroup(groupId, e)}
                  >
                    <svg width="11" height="11" viewBox="0 0 12 12" fill="none">
                      <rect x="1.5" y="1.5" width="4" height="4" rx="0.8" stroke="currentColor" strokeWidth="1.1"/>
                      <rect x="6.5" y="6.5" width="4" height="4" rx="0.8" stroke="currentColor" strokeWidth="1.1"/>
                      <path d="M10.5 1.5l-9 9" stroke="currentColor" strokeWidth="1.1" strokeLinecap="round"/>
                    </svg>
                  </button>
                )}
              </div>
              {!isCollapsed && (
                <div className="object-group-members">
                  {members.map(m => renderRow(m, { inGroup: true }))}
                </div>
              )}
            </div>
          );
        })}
      </div>

      {showLibrary && !readOnly && (
        <div className="add-menu" style={{ maxHeight: 320, overflowY: "auto" }}>
          {OBJECT_LIBRARY.map(group => (
            <React.Fragment key={group.section}>
              <div className="add-section-label">{group.section}</div>
              {group.items.map(item => (
                <button
                  key={item.kind + item.name}
                  className="add-menu-item"
                  draggable
                  onDragStart={(e) => handleDragStart(e, item)}
                  onClick={() => addItem(item)}
                >
                  <span>{item.name}</span>
                  <span className="meta">{item.meta}</span>
                </button>
              ))}
            </React.Fragment>
          ))}
        </div>
      )}

      <div className="plate-stats">
        <div className="plate-stat-row">
          <span className="k">Build plate</span>
          <span className="v">{plateSize[0]} × {plateSize[1]} mm</span>
        </div>
        <div className="plate-stat-row">
          <span className="k">Objects</span>
          <span className="v">{objects.length}</span>
        </div>
      </div>

      {materialPicker && !readOnly && (() => {
        const obj = objects.find(o => o.id === materialPicker.objId);
        if (!obj) return null;
        return (
          <MaterialPicker
            obj={obj}
            materialMap={materialMap}
            slotMap={slotMap}
            filaments={filaments}
            slotIds={slotIds}
            anchorRect={materialPicker.rect}
            onAssign={(mid) => setObjectMaterial && setObjectMaterial(obj.id, mid)}
            onCreate={(sid) => createMaterialForObject && createMaterialForObject(obj.id, sid)}
            onClose={() => setMaterialPicker(null)}
          />
        );
      })()}
    </aside>
  );
}

window.OBJECT_LIBRARY = OBJECT_LIBRARY;
window.ObjectsPanel = ObjectsPanel;
