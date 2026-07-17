// Top-bar dropdowns:
//   - the n3o-slic3r brand menu → "Global plugins…" + Appearance picker
//   - the project menu → "Plugins…"
//
// Minimal dropdowns (the brand/project menu CSS already lives in
// index.css). Each toggles on click and closes on outside-click or Esc.

import { useRef, useState } from "react";
import { type ThemeMode, useTheme } from "../theme/useTheme";
import { usePopoverDismiss } from "../ui/usePopoverDismiss";

/** Platform-appropriate accelerator label for a menu item (must match the
 *  chords bound in App.tsx): `⌘S` / `⇧⌘S` on macOS, `Ctrl+S` /
 *  `Ctrl+Shift+S` elsewhere. */
const IS_MAC =
  typeof navigator !== "undefined" && /mac/i.test(navigator.userAgent);
function shortcut(key: string, shift = false): string {
  if (IS_MAC) return `${shift ? "⇧" : ""}⌘${key}`;
  return `Ctrl+${shift ? "Shift+" : ""}${key}`;
}

function useDropdown(): {
  open: boolean;
  setOpen: (v: boolean) => void;
  ref: React.RefObject<HTMLDivElement | null>;
} {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement | null>(null);
  usePopoverDismiss(ref, () => setOpen(false), open);
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
  const { mode, setMode } = useTheme();
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
          <div className="tb-menu-divider" />
          <div className="tb-menu-section">Appearance</div>
          <AppearanceItem label="System" value="system" current={mode} onSelect={setMode} />
          <AppearanceItem label="Light" value="light" current={mode} onSelect={setMode} />
          <AppearanceItem label="Dark" value="dark" current={mode} onSelect={setMode} />
        </div>
      )}
    </div>
  );
}

interface AppearanceItemProps {
  label: string;
  value: ThemeMode;
  current: ThemeMode;
  onSelect: (mode: ThemeMode) => void;
}

function AppearanceItem({ label, value, current, onSelect }: AppearanceItemProps): React.JSX.Element {
  const selected = value === current;
  return (
    <button
      type="button"
      className="tb-menu-item"
      role="menuitemradio"
      aria-checked={selected}
      onClick={() => onSelect(value)}
    >
      <span className="tb-menu-radio" aria-hidden>
        <svg width="12" height="12" viewBox="0 0 12 12" fill="none">
          <circle cx="6" cy="6" r="5" stroke="currentColor" strokeWidth="1.2" />
          {selected && <circle cx="6" cy="6" r="2.6" fill="currentColor" />}
        </svg>
      </span>
      <span className="tb-menu-radio-label">{label}</span>
    </button>
  );
}

export interface ProjectMenuProps {
  /** The current project's filename (e.g. "thing.3mf"), or
   * "Untitled.n3o" when the project has never been saved. */
  projectName?: string | null;
  /** True when the project has unsaved edits — shows a `•` marker. */
  dirty?: boolean;
  onNewProject: () => void;
  onOpenProject: () => void;
  onSaveProject: () => void;
  onSaveProjectAs: () => void;
  onOpenProjectPlugins: () => void;
  projectPluginCount?: number;
}

export function ProjectMenu({
  projectName,
  dirty = false,
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
        {dirty && (
          <span
            className="tb-dirty-dot"
            title="Unsaved changes"
            aria-label="Unsaved changes"
          >
            •
          </span>
        )}
        <span>{projectName ?? "Untitled.n3o"}</span>
        <Chevron />
      </button>
      {open && (
        <div className="tb-menu" role="menu">
          <div className="tb-menu-section">Project</div>
          <button type="button" className="tb-menu-item" role="menuitem" onClick={act(onNewProject)}>
            <span>New project</span>
            <span className="tb-menu-shortcut">{shortcut("N")}</span>
          </button>
          <button type="button" className="tb-menu-item" role="menuitem" onClick={act(onOpenProject)}>
            <span>Open project…</span>
            <span className="tb-menu-shortcut">{shortcut("O")}</span>
          </button>
          <button type="button" className="tb-menu-item" role="menuitem" onClick={act(onSaveProject)}>
            <span>Save project</span>
            <span className="tb-menu-shortcut">{shortcut("S")}</span>
          </button>
          <button type="button" className="tb-menu-item" role="menuitem" onClick={act(onSaveProjectAs)}>
            <span>Save project as…</span>
            <span className="tb-menu-shortcut">{shortcut("S", true)}</span>
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
