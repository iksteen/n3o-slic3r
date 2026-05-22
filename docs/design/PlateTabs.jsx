// PlateTabs.jsx — horizontal plate-tabs strip below the topbar. Each plate
// owns its own printer/bed/nozzle/objects/overrides. Printer assignment is
// done from the settings panel; the tab just displays the current printer.

function PlateTabs({
  plates,
  activePlateId,
  setActivePlateId,
  addPlate,
  closePlate,
  renamePlate,
}) {
  const { useState, useRef, useEffect } = React;
  const [editingId, setEditingId] = useState(null);
  const [editValue, setEditValue] = useState("");
  const editInputRef = useRef(null);

  useEffect(() => {
    if (editingId && editInputRef.current) {
      editInputRef.current.focus();
      editInputRef.current.select();
    }
  }, [editingId]);

  const commitRename = () => {
    if (editingId && editValue.trim()) {
      renamePlate(editingId, editValue.trim());
    }
    setEditingId(null);
  };

  return (
    <div className="plate-tabs">
      <div className="plate-tabs-scroll">
        {plates.map(plate => {
          const isActive = plate.id === activePlateId;
          const isEditing = editingId === plate.id;
          return (
            <div
              key={plate.id}
              className={`plate-tab ${isActive ? "active" : ""}`}
              onClick={() => !isEditing && setActivePlateId(plate.id)}
            >
              <span className="plate-tab-icon" title="Build plate">
                <svg width="13" height="13" viewBox="0 0 14 14" fill="none">
                  <path d="M1 9l6 3 6-3M1 6l6 3 6-3M1 3l6 3 6-3-6-3-6 3z" stroke="currentColor" strokeWidth="1.1" strokeLinejoin="round"/>
                </svg>
              </span>
              {isEditing ? (
                <input
                  ref={editInputRef}
                  className="plate-tab-rename-input"
                  value={editValue}
                  onChange={(e) => setEditValue(e.target.value)}
                  onBlur={commitRename}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") commitRename();
                    if (e.key === "Escape") { setEditingId(null); }
                  }}
                  onClick={(e) => e.stopPropagation()}
                />
              ) : (
                <span
                  className="plate-tab-name"
                  onDoubleClick={(e) => {
                    e.stopPropagation();
                    setEditingId(plate.id);
                    setEditValue(plate.name);
                  }}
                  title="Double-click to rename"
                >
                  {plate.name}
                </span>
              )}
              <span className="plate-tab-divider"/>
              <span className="plate-tab-printer-display" title={`Assigned to ${plate.printer} — change in the settings panel`}>
                {plate.printer}
              </span>
              <span className="plate-tab-meta">
                {plate.objects.length} obj
              </span>
              {plates.length > 1 && (
                <button
                  className="plate-tab-close"
                  onClick={(e) => { e.stopPropagation(); closePlate(plate.id); }}
                  title="Close plate"
                >
                  <svg width="9" height="9" viewBox="0 0 12 12" fill="none">
                    <path d="M2.5 2.5l7 7M9.5 2.5l-7 7" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round"/>
                  </svg>
                </button>
              )}
            </div>
          );
        })}
      </div>
      <button className="plate-tab-add" onClick={addPlate} title="New plate (uses next printer preset)">
        <svg width="13" height="13" viewBox="0 0 14 14" fill="none">
          <path d="M7 2v10M2 7h10" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round"/>
        </svg>
        <span>New plate</span>
      </button>
    </div>
  );
}

window.PlateTabs = PlateTabs;
