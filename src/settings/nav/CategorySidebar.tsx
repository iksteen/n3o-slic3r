// Vertical category rail (PR-4-3) — FR-UI-1.
//
// Mirrors the mockup's `.cat-rail` + `.cat-rail-item` layout
// (docs/dev/design/SettingsPanel.jsx:633-654). Each item shows icon +
// name + a count badge in `overrides/total` form (PR-4-7 fills
// the override count; PR-4-3 ships the badge slot displaying just
// `total`). Active item is highlighted; clicking a category jumps
// the scroll body to its first row.

import type { CategoryCounts, CategoryGroup } from "./categories";
import type { OptionSummary } from "../types";

export interface CategorySidebarProps {
  groups: readonly CategoryGroup<OptionSummary>[];
  counts: ReadonlyMap<string, CategoryCounts>;
  activeId: string | null;
  onActivate: (id: string) => void;
}

export function CategorySidebar({
  groups,
  counts,
  activeId,
  onActivate,
}: CategorySidebarProps) {
  return (
    <nav className="cat-rail" role="tablist" aria-label="Setting categories">
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
