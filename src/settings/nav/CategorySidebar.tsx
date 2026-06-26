// Vertical category rail — FR-UI-1.
//
// Mirrors the mockup's `.cat-rail` + `.cat-rail-item` layout
// (docs/dev/design/SettingsPanel.jsx:633-654). Each item shows icon +
// name + a count badge in `overrides/total` form, with the override
// count from the cascade trace. Active item is highlighted; clicking
// a category jumps the scroll body to its first row.

import type { CategoryCounts, CategoryGroup } from "./categories";
import type { OptionSummary } from "../types";

export interface CategorySidebarProps {
  groups: readonly CategoryGroup<OptionSummary>[];
  counts: ReadonlyMap<string, CategoryCounts>;
  activeId: string | null;
  onActivate: (id: string) => void;
  /** Collapsed = icons-only (labels hidden). Drives the toggle chevron and
   *  the per-item tooltip; the width change is CSS on `.settings-panel`. */
  collapsed: boolean;
  onToggleCollapsed: () => void;
}

export function CategorySidebar({
  groups,
  counts,
  activeId,
  onActivate,
  collapsed,
  onToggleCollapsed,
}: CategorySidebarProps) {
  return (
    <nav className="cat-rail" role="tablist" aria-label="Setting categories">
      <button
        type="button"
        className="cat-rail-toggle"
        onClick={onToggleCollapsed}
        title={collapsed ? "Expand category list" : "Collapse category list to icons"}
        aria-label={collapsed ? "Expand category list" : "Collapse category list"}
        aria-expanded={!collapsed}
      >
        <svg width="12" height="14" viewBox="0 0 12 14" fill="none" aria-hidden>
          <path
            d={collapsed ? "M5 3l4 4-4 4" : "M9 3L5 7l4 4"}
            stroke="currentColor"
            strokeWidth="1.5"
            strokeLinecap="round"
            strokeLinejoin="round"
          />
        </svg>
      </button>
      {groups.map((g) => {
        const c = counts.get(g.id) ?? { total: g.settings.length, overrides: 0 };
        const active = g.id === activeId;
        return (
          <button
            key={g.id}
            type="button"
            role="tab"
            aria-selected={active}
            className={`cat-rail-item${active ? " active" : ""}`}
            onClick={() => onActivate(g.id)}
            title={collapsed ? g.name : undefined}
          >
            <span className="cat-rail-icon" aria-hidden>
              {g.icon}
            </span>
            <span className="cat-rail-name">{g.name}</span>
            <span className="cat-rail-count">
              {c.overrides > 0 ? (
                <span className="cat-rail-count-overrides">
                  {c.overrides}/{c.total}
                </span>
              ) : (
                c.total
              )}
            </span>
          </button>
        );
      })}
    </nav>
  );
}
