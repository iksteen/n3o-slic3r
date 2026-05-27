// PlateTabs.jsx — horizontal tab strip below the topbar.
//
// Layout:
//   [+]  [Plate 1] [Plate 2] [Plate 3] ────────────────── [Devices]
//
// Left: a single "+" button to add a new plate. Then the plate tabs scroll.
// Right-aligned at the end: a Devices tab that switches the whole workspace
// into fleet-monitor mode (the user's preferred mental model — Devices is
// "the other place", not yet another plate).

function PlateTabs({
  plates,
  activePlateId,
  setActivePlateId,
  addPlate,
  closePlate,
  renamePlate,
  devicesActive,
  onSelectDevices,
  printingCount = 0,
  errorCount = 0,
  deviceCount = 0,
}) {
  const { useState, useRef, useEffect, useLayoutEffect } = React;
  const [editingId, setEditingId] = useState(null);
  const [editValue, setEditValue] = useState("");
  const editInputRef = useRef(null);
  // Scroll container — used for active-tab-into-view, edge fades, and
  // mouse-wheel-to-horizontal-scroll.
  const scrollerRef = useRef(null);
  const activeTabRef = useRef(null);
  const [scrollState, setScrollState] = useState({ left: false, right: false });

  useEffect(() => {
    if (editingId && editInputRef.current) {
      editInputRef.current.focus();
      editInputRef.current.select();
    }
  }, [editingId]);

  // Keep the active tab in view when switching plates (e.g. opening a new
  // plate at the far right of an overflowing strip).
  useLayoutEffect(() => {
    const el = activeTabRef.current;
    const scroller = scrollerRef.current;
    if (!el || !scroller || devicesActive) return;
    const eLeft = el.offsetLeft;
    const eRight = eLeft + el.offsetWidth;
    const sLeft = scroller.scrollLeft;
    const sRight = sLeft + scroller.clientWidth;
    if (eLeft < sLeft) scroller.scrollTo({ left: eLeft - 12, behavior: "smooth" });
    else if (eRight > sRight) scroller.scrollTo({ left: eRight - scroller.clientWidth + 12, behavior: "smooth" });
  }, [activePlateId, devicesActive, plates.length]);

  // Update edge-fade indicators whenever the scroll state changes (resize,
  // tab add/remove, manual scroll).
  useEffect(() => {
    const scroller = scrollerRef.current;
    if (!scroller) return;
    const update = () => {
      const max = scroller.scrollWidth - scroller.clientWidth;
      setScrollState({
        left: scroller.scrollLeft > 1,
        right: scroller.scrollLeft < max - 1,
      });
    };
    update();
    scroller.addEventListener("scroll", update, { passive: true });
    const ro = new ResizeObserver(update);
    ro.observe(scroller);
    // Re-check after each tab mutation
    const mo = new MutationObserver(update);
    mo.observe(scroller, { childList: true, subtree: false });
    return () => {
      scroller.removeEventListener("scroll", update);
      ro.disconnect();
      mo.disconnect();
    };
  }, []);

  // Convert vertical mouse-wheel into horizontal scroll over the tab strip
  // (matches Chrome / VS Code tab behaviour). Trackpad gestures with native
  // deltaX bypass this branch and scroll directly.
  const onWheel = (e) => {
    const scroller = scrollerRef.current;
    if (!scroller) return;
    if (Math.abs(e.deltaY) > Math.abs(e.deltaX) && e.deltaY !== 0) {
      scroller.scrollLeft += e.deltaY;
      // Only prevent default if we actually consumed (have room to move).
      const max = scroller.scrollWidth - scroller.clientWidth;
      if (scroller.scrollLeft > 0 && scroller.scrollLeft < max) e.preventDefault();
    }
  };

  const commitRename = () => {
    if (editingId && editValue.trim()) {
      renamePlate(editingId, editValue.trim());
    }
    setEditingId(null);
  };

  return (
    <div className="plate-tabs">
      <button
        className="plate-tab-add"
        onClick={addPlate}
        title="New plate"
        aria-label="New plate"
      >
        <svg width="13" height="13" viewBox="0 0 14 14" fill="none">
          <path d="M7 2v10M2 7h10" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round"/>
        </svg>
      </button>

      <div
        className={`plate-tabs-scroll ${scrollState.left ? "fade-left" : ""} ${scrollState.right ? "fade-right" : ""}`}
        ref={scrollerRef}
        onWheel={onWheel}
      >
        {plates.map(plate => {
          const isActive = !devicesActive && plate.id === activePlateId;
          const isEditing = editingId === plate.id;
          return (
            <div
              key={plate.id}
              ref={isActive ? activeTabRef : null}
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

      {/* Devices: right-aligned context tab. */}
      <button
        className={`plate-tab plate-tab-devices ${devicesActive ? "active" : ""}`}
        onClick={onSelectDevices}
        title="Devices — monitor and control your printers"
      >
        <span className="plate-tab-icon">
          <svg width="13" height="13" viewBox="0 0 14 14" fill="none">
            <rect x="1.5" y="2.5" width="11" height="7.5" rx="1.2" stroke="currentColor" strokeWidth="1.2"/>
            <path d="M5 12.5h4M7 10v2.5" stroke="currentColor" strokeWidth="1.2" strokeLinecap="round"/>
          </svg>
        </span>
        <span className="plate-tab-name">Devices</span>
        <span className="plate-tab-meta">{deviceCount}</span>
        {printingCount > 0 && (
          <span className="plate-tab-badge printing" title={`${printingCount} printing`}>{printingCount}</span>
        )}
        {errorCount > 0 && (
          <span className="plate-tab-badge error" title={`${errorCount} with errors`}>!</span>
        )}
      </button>
    </div>
  );
}

window.PlateTabs = PlateTabs;
