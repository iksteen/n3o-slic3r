// TopBar.jsx — app chrome + project name + primary actions.
// Per-print config (printer/bed/nozzle/filaments) lives at the top of the
// settings panel where it scopes the editable settings below.

function TopBar({
  projectName,
  onSlice,
  onResetCamera,
}) {
  return (
    <header className="topbar">
      <div className="brand">
        <div className="brand-mark"/>
        n3o-slic3r
      </div>
      <div className="brand-divider"/>

      <button className="tb-btn project-name-btn" title="Open recent / new / save">
        <svg width="13" height="13" viewBox="0 0 14 14" fill="none">
          <path d="M2 3.5A1.5 1.5 0 0 1 3.5 2h2.4l1.4 1.5H10.5A1.5 1.5 0 0 1 12 5v5.5A1.5 1.5 0 0 1 10.5 12h-7A1.5 1.5 0 0 1 2 10.5v-7z" stroke="currentColor" strokeWidth="1.2" strokeLinejoin="round"/>
        </svg>
        {projectName}
        <svg width="9" height="9" viewBox="0 0 10 10" fill="none" style={{ opacity: 0.55, marginLeft: 2 }}>
          <path d="M2 4l3 3 3-3" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round"/>
        </svg>
      </button>

      <div className="tb-spacer"/>

      <button className="tb-btn" onClick={onResetCamera} title="Reset view">
        <svg width="13" height="13" viewBox="0 0 14 14" fill="none">
          <path d="M2 7a5 5 0 1 0 1.5-3.5" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round"/>
          <path d="M1.5 1.5v3h3" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round"/>
        </svg>
        View
      </button>
      <button className="tb-btn">
        Preview <span className="kbd">P</span>
      </button>
      <button className="tb-btn primary" onClick={onSlice}>
        Slice
        <svg width="12" height="12" viewBox="0 0 12 12" fill="none">
          <path d="M3 3l6 3-6 3V3z" fill="currentColor"/>
        </svg>
      </button>
    </header>
  );
}

window.TopBar = TopBar;
