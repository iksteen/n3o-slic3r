// PluginsModal — the menu-launched Global / Project plugin surfaces.
//
// Wraps `PluginManager` in a `ModalBackdrop` with the level-tinted
// header + a footer (active count + Done). Esc closes (via
// `useEscapeKey`). The plate surface embeds `PluginManager` directly
// inside the settings panel instead of using this modal.

import { ModalBackdrop, ModalCloseButton } from "../ui/Modal";
import type { PluginSummary } from "./pluginCommands";
import {
  PLUGIN_LEVEL_META,
  countActiveAtLevel,
  type CascadeSources,
  type PluginLevel,
} from "./pluginCascade";
import { PluginManager, type PluginWriters } from "./PluginManager";
import { useEscapeKey } from "./usePlugins";

export interface PluginsModalProps {
  level: PluginLevel;
  plugins: PluginSummary[];
  sources: CascadeSources;
  writers: PluginWriters;
  onClose: () => void;
  /** Project name for the Project-level subtitle. */
  projectName?: string | null;
  plateName?: string | null;
  readOnly?: boolean;
}

export function PluginsModal({
  level,
  plugins,
  sources,
  writers,
  onClose,
  projectName,
  plateName,
  readOnly = false,
}: PluginsModalProps): React.JSX.Element {
  useEscapeKey(onClose);

  const meta = PLUGIN_LEVEL_META[level];
  const count = countActiveAtLevel(plugins, level, sources);
  const total = plugins.filter((p) => p.scopes.includes(level)).length;

  return (
    <ModalBackdrop
      onDismiss={onClose}
      cardClassName="plugins-modal"
      ariaLabelledBy="plg-title"
    >
      <header className="plg-header">
        <div
          className="plg-header-mark"
          style={{ "--lvl-hue": meta.hue } as React.CSSProperties}
          aria-hidden="true"
        >
          <svg width="19" height="19" viewBox="0 0 16 16" fill="none">
            <path
              d="M6 2v2M10 2v2M4 4h8v3a4 4 0 0 1-4 4 4 4 0 0 1-4-4V4zM8 11v3"
              stroke="currentColor"
              strokeWidth="1.3"
              strokeLinecap="round"
              strokeLinejoin="round"
            />
          </svg>
        </div>
        <div className="plg-header-text">
          <h2 id="plg-title">{meta.label} plugins</h2>
          <p>
            {level === "global"
              ? "Applies to every project on this machine."
              : `Scoped to ${projectName ?? "this project"}. Inherits from Global.`}
          </p>
        </div>
        <ModalCloseButton onClick={onClose} />
      </header>

      <div className="plg-modal-body">
        <PluginManager
          level={level}
          plugins={plugins}
          sources={sources}
          writers={writers}
          readOnly={readOnly}
          plateName={plateName}
        />
      </div>

      <footer className="plg-footer">
        <span className="plg-foot-hint">
          Lower levels override higher ones. Unset = inherit.
        </span>
        <div className="plg-footer-right">
          <span className="plg-enabled-count">
            {count} of {total} active here
          </span>
          <button className="apm-btn primary" onClick={onClose} type="button">
            Done
          </button>
        </div>
      </footer>
    </ModalBackdrop>
  );
}
