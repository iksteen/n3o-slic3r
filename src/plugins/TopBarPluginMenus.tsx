// Top-bar dropdowns that host the plugin surfaces:
//   - the n3o-slic3r brand menu → "Global plugins…"
//   - the project menu → "Plugins…"
//
// Minimal dropdowns (the brand/project menu CSS already lives in
// index.css). Each toggles on click and closes on outside-click or Esc.

import { useEffect, useRef, useState } from "react";

function useDropdown(): {
  open: boolean;
  setOpen: (v: boolean) => void;
  ref: React.RefObject<HTMLDivElement | null>;
} {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement | null>(null);
  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent): void => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    window.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      window.removeEventListener("keydown", onKey);
    };
  }, [open]);
  return { open, setOpen, ref };
}

const Chevron = (): React.JSX.Element => (
  <svg className="tb-chevron" width="9" height="9" viewBox="0 0 10 10" fill="none" aria-hidden>
    <path
      d="M2 3.5l3 3 3-3"
      stroke="currentColor"
      strokeWidth="1.4"
      strokeLinecap="round"
      strokeLinejoin="round"
    />
  </svg>
);

export interface BrandMenuProps {
  onOpenGlobalPlugins: () => void;
  globalPluginCount?: number;
}

export function BrandMenu({
  onOpenGlobalPlugins,
  globalPluginCount = 0,
}: BrandMenuProps): React.JSX.Element {
  const { open, setOpen, ref } = useDropdown();
  return (
    <div className="brand-menu-wrap" ref={ref}>
      <button type="button" className="brand" onClick={() => setOpen(!open)}>
        <span className="brand-mark" aria-hidden />
        n3o-slic3r
        <Chevron />
      </button>
      {open && (
        <div className="brand-menu" role="menu">
          <div className="brand-menu-app">
            <span className="brand-menu-app-name">n3o-slic3r</span>
          </div>
          <button
            type="button"
            className="tb-menu-item"
            role="menuitem"
            onClick={() => {
              setOpen(false);
              onOpenGlobalPlugins();
            }}
          >
            <span>Global plugins…</span>
            {globalPluginCount > 0 && <span className="tb-menu-count">{globalPluginCount}</span>}
          </button>
        </div>
      )}
    </div>
  );
}

export interface ProjectMenuProps {
  /** The current project's filename (e.g. "thing.3mf"), or
   * "Untitled.3mf" when the project has never been saved. */
  projectName?: string | null;
  onNewProject: () => void;
  onOpenProject: () => void;
  onSaveProject: () => void;
  onSaveProjectAs: () => void;
  onOpenProjectPlugins: () => void;
  projectPluginCount?: number;
}

export function ProjectMenu({
  projectName,
  onNewProject,
  onOpenProject,
  onSaveProject,
  onSaveProjectAs,
  onOpenProjectPlugins,
  projectPluginCount = 0,
}: ProjectMenuProps): React.JSX.Element {
  const { open, setOpen, ref } = useDropdown();
  // Run an action and close the menu.
  const act = (fn: () => void) => () => {
    setOpen(false);
    fn();
  };
  return (
    <div className="tb-file-menu-wrap" ref={ref}>
      <button type="button" className="tb-btn" onClick={() => setOpen(!open)}>
        <span>{projectName ?? "Untitled.3mf"}</span>
        <Chevron />
      </button>
      {open && (
        <div className="tb-menu" role="menu">
          <div className="tb-menu-section">Project</div>
          <button type="button" className="tb-menu-item" role="menuitem" onClick={act(onNewProject)}>
            <span>New project</span>
          </button>
          <button type="button" className="tb-menu-item" role="menuitem" onClick={act(onOpenProject)}>
            <span>Open project…</span>
          </button>
          <button type="button" className="tb-menu-item" role="menuitem" onClick={act(onSaveProject)}>
            <span>Save project</span>
          </button>
          <button type="button" className="tb-menu-item" role="menuitem" onClick={act(onSaveProjectAs)}>
            <span>Save project as…</span>
          </button>
          <button
            type="button"
            className="tb-menu-item"
            role="menuitem"
            onClick={act(onOpenProjectPlugins)}
          >
            <span>Plugins…</span>
            {projectPluginCount > 0 && <span className="tb-menu-count">{projectPluginCount}</span>}
          </button>
        </div>
      )}
    </div>
  );
}
