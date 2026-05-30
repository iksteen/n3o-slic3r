// TopBar.jsx — app chrome with project menu + context-aware actions.
//
// Navigation lives entirely in the tab-bar below this (plate tabs + a
// right-aligned Devices tab); Preview is a toggle inside the plate canvas.
// The topbar holds the brand, project file-menu, and whatever primary /
// secondary action makes sense for the current view.

function TopBar({
  projectName,
  primary,    // { label, onClick, kind?: 'primary'|'ghost', title?, icon? }
  secondary,  // optional second button { label, onClick, title? }
  onOpenGlobalPlugins,   // brand menu → global plugins
  onOpenProjectPlugins,  // project menu → project plugins
  globalPluginCount = 0,
  projectPluginCount = 0,
}) {
  const { useState, useRef, useEffect } = React;
  const [menuOpen, setMenuOpen] = useState(false);
  const [brandOpen, setBrandOpen] = useState(false);
  const menuRef = useRef(null);
  const brandRef = useRef(null);

  useEffect(() => {
    if (!menuOpen) return;
    const onDown = (e) => {
      if (menuRef.current && !menuRef.current.contains(e.target)) setMenuOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [menuOpen]);

  useEffect(() => {
    if (!brandOpen) return;
    const onDown = (e) => {
      if (brandRef.current && !brandRef.current.contains(e.target)) setBrandOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [brandOpen]);

  return (
    <header className="topbar">
      <div className="brand-menu-wrap" ref={brandRef}>
        <button
          className={`brand-btn ${brandOpen ? "open" : ""}`}
          onClick={() => setBrandOpen(v => !v)}
          title="n3o-slic3r"
        >
          <div className="brand-mark"/>
          n3o-slic3r
          <svg className="brand-caret" width="9" height="9" viewBox="0 0 10 10" fill="none">
            <path d="M2 4l3 3 3-3" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round"/>
          </svg>
        </button>

        {brandOpen && (
          <div className="brand-menu" onClick={() => setBrandOpen(false)}>
            <div className="brand-menu-app">
              <div className="brand-mark"/>
              <span className="brand-menu-app-name">n3o-slic3r</span>
              <span className="brand-menu-app-ver">v0.4.1</span>
            </div>
            {onOpenGlobalPlugins && (
              <button className="tb-menu-item" onClick={onOpenGlobalPlugins}>
                <span>Global plugins…</span>
                {globalPluginCount > 0 && <span className="tb-menu-count">{globalPluginCount}</span>}
              </button>
            )}
            <button className="tb-menu-item dim">
              <span>Preferences…</span><span className="kbd">⌘,</span>
            </button>
            <div className="tb-menu-divider"/>
            <button className="tb-menu-item dim"><span>About n3o-slic3r</span></button>
          </div>
        )}
      </div>
      <div className="brand-divider"/>

      <div className="tb-file-menu-wrap" ref={menuRef}>
        <button
          className={`tb-btn project-name-btn ${menuOpen ? "open" : ""}`}
          title="Project — file menu"
          onClick={() => setMenuOpen(v => !v)}
        >
          <svg width="13" height="13" viewBox="0 0 14 14" fill="none">
            <path d="M2 3.5A1.5 1.5 0 0 1 3.5 2h2.4l1.4 1.5H10.5A1.5 1.5 0 0 1 12 5v5.5A1.5 1.5 0 0 1 10.5 12h-7A1.5 1.5 0 0 1 2 10.5v-7z"
                  stroke="currentColor" strokeWidth="1.2" strokeLinejoin="round"/>
          </svg>
          {projectName}
          <svg width="9" height="9" viewBox="0 0 10 10" fill="none" style={{ opacity: 0.55, marginLeft: 2 }}>
            <path d="M2 4l3 3 3-3" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round"/>
          </svg>
        </button>

        {menuOpen && (
          <div className="tb-menu" onClick={() => setMenuOpen(false)}>
            <div className="tb-menu-section">File</div>
            <button className="tb-menu-item">
              <span>New project</span><span className="kbd">⌘N</span>
            </button>
            <button className="tb-menu-item">
              <span>Open project…</span><span className="kbd">⌘O</span>
            </button>
            <div className="tb-menu-section">Open recent</div>
            <button className="tb-menu-item recent">
              <span className="tb-menu-recent-name">balcony-clip-v4.3mf</span>
              <span className="tb-menu-recent-meta">2h ago</span>
            </button>
            <button className="tb-menu-item recent">
              <span className="tb-menu-recent-name">desk-grommet.3mf</span>
              <span className="tb-menu-recent-meta">yesterday</span>
            </button>
            <button className="tb-menu-item recent">
              <span className="tb-menu-recent-name">snapmaker-cable-cover.3mf</span>
              <span className="tb-menu-recent-meta">3d ago</span>
            </button>
            <div className="tb-menu-divider"/>
            <button className="tb-menu-item">
              <span>Save</span><span className="kbd">⌘S</span>
            </button>
            <button className="tb-menu-item">
              <span>Save as…</span><span className="kbd">⌘⇧S</span>
            </button>
            <div className="tb-menu-divider"/>
            <button className="tb-menu-item">
              <span>Import STL / 3MF…</span>
            </button>
            <button className="tb-menu-item">
              <span>Export G-code…</span>
            </button>
            {onOpenProjectPlugins && (
              <>
                <div className="tb-menu-section">Project</div>
                <button className="tb-menu-item" onClick={onOpenProjectPlugins}>
                  <span>Plugins…</span>
                  {projectPluginCount > 0
                    ? <span className="tb-menu-count">{projectPluginCount}</span>
                    : <span className="kbd">⌘⇧P</span>}
                </button>
              </>
            )}
            <div className="tb-menu-divider"/>
            <button className="tb-menu-item dim">
              <span>Preferences…</span><span className="kbd">⌘,</span>
            </button>
          </div>
        )}
      </div>

      <div className="tb-spacer"/>

      {secondary && (
        <button className="tb-btn" onClick={secondary.onClick} title={secondary.title}>
          {secondary.icon}
          {secondary.label}
        </button>
      )}
      {primary && (
        <button
          className={`tb-btn ${primary.kind === "ghost" ? "" : "primary"}`}
          onClick={primary.onClick}
          title={primary.title}
          disabled={primary.disabled}
        >
          {primary.label}
          {primary.icon !== false && primary.kind !== "ghost" && (
            <svg width="12" height="12" viewBox="0 0 12 12" fill="none">
              <path d="M3 3l6 3-6 3V3z" fill="currentColor"/>
            </svg>
          )}
        </button>
      )}
    </header>
  );
}

window.TopBar = TopBar;
