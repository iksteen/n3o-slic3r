// ObjectsPanel.jsx — left panel: object library + plate object list.

const { useState: useStateOP, useRef: useRefOP } = React;
const { resolveObjectFilament: resolveOP } = window.SLICER_DATA;

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

function ObjectsPanel({
  objects, setObjects,
  selectedId, setSelectedId,
  filaments,
  slotMap,
  materialMap,
  printerName,
  countObjectOverrides,
  plateSize,
}) {
  const [showLibrary, setShowLibrary] = useStateOP(false);
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
      overrides: {},
    }]);
    setSelectedId(id);
    setShowLibrary(false);
  };

  const removeObject = (id, e) => {
    e?.stopPropagation();
    setObjects(prev => prev.filter(o => o.id !== id));
    if (selectedId === id) setSelectedId(null);
  };

  return (
    <aside className="objects-panel">
      <div className="panel-head">
        <h3>Plate · {printerName}</h3>
        <button className="icon-btn" title="Add object" onClick={() => setShowLibrary(s => !s)}>
          <svg width="12" height="12" viewBox="0 0 12 12" fill="none">
            <path d="M6 1.5v9M1.5 6h9" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round"/>
          </svg>
        </button>
      </div>

      <div className="object-list">
        {objects.length === 0 && (
          <div className="empty-state">
            Drag an object from the library<br/>or click <span className="kbd-inline">+</span> to add one.
          </div>
        )}
        {objects.map(obj => {
          const { materialId, filament } = resolveOP(obj, materialMap, slotMap, filaments);
          const color = (filament && filament.color) || "#888";
          const filLabel = (filament && filament.label) || "unassigned";
          const overrideCount = countObjectOverrides(obj.id);
          return (
            <div
              key={obj.id}
              className={`object-item ${selectedId === obj.id ? "selected" : ""}`}
              onClick={() => setSelectedId(obj.id)}
            >
              <div className="object-thumb">{obj.kind.slice(0, 2).toUpperCase()}</div>
              <div style={{ minWidth: 0 }}>
                <div className="object-name">
                  <span className="object-color-tag" style={{ background: color }}/>
                  {obj.name}
                </div>
                <div className="object-meta">
                  <span className="object-material-badge" title={`Material ${materialId} → ${filLabel}`}>{materialId || "M?"}</span>
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
                <button
                  className="icon-btn"
                  title="Remove"
                  onClick={(e) => removeObject(obj.id, e)}
                >
                  <svg width="10" height="10" viewBox="0 0 12 12" fill="none">
                    <path d="M2.5 2.5l7 7M9.5 2.5l-7 7" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round"/>
                  </svg>
                </button>
              </div>
            </div>
          );
        })}
      </div>

      {showLibrary && (
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
        <div className="plate-stat-row">
          <span className="k">Est. material</span>
          <span className="v">{(objects.length * 12.4).toFixed(1)} g</span>
        </div>
        <div className="plate-stat-row">
          <span className="k">Est. time</span>
          <span className="v">
            {objects.length === 0 ? "—" : `${Math.floor(objects.length * 0.42 + 0.6)}h ${((objects.length * 25) % 60).toString().padStart(2,"0")}m`}
          </span>
        </div>
      </div>
    </aside>
  );
}

window.OBJECT_LIBRARY = OBJECT_LIBRARY;
window.ObjectsPanel = ObjectsPanel;
