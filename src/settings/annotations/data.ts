// "Why this matters" annotations (PR-4-12).
//
// Authored text per high-impact libslic3r option, surfaced by
// PR-4-11's SettingTooltip in the "💡 tip" section beneath
// libslic3r's own tooltip text. Aim for 2-4 sentences: what the
// setting controls in physical terms, the trade-off, and a quick
// rule of thumb.
//
// Authoring guidance:
// - Don't restate libslic3r's tooltip; it renders above the
//   "💡 tip" section already.
// - Prefer mechanical language ("0.2 mm layers print 2× faster
//   than 0.1 mm") over preset advice ("use 0.2 for PLA"). Users
//   own their own preset choices.
// - Cap each entry at ~4 sentences. Long explanations hurt
//   tooltip readability.
//
// PR-4-12 populates the high-impact catalog (~30 entries). Below
// is the initial seed; extend per the audit in PR-4-12's ticket.

export const ANNOTATIONS: Record<string, string> = {
  layer_height:
    "Vertical thickness of each printed layer. Lower = finer surface " +
    "detail and slower print (halving the layer height roughly doubles " +
    "print time). 0.2 mm is the typical FDM default; 0.1-0.12 mm for " +
    "visible-surface parts, 0.28-0.32 mm for fast drafts on a 0.4 mm nozzle.",

  sparse_infill_density:
    "Percentage of the part's interior that gets filled with infill. " +
    "Higher = stronger but more filament + slower. 15-20 % is a strong " +
    "all-purpose default for non-load-bearing parts; bump to 40-60 % " +
    "for functional parts under stress. Walls + top/bottom shells carry " +
    "most of the strength under bending; infill mostly resists buckling.",

  wall_loops:
    "Number of perimeter loops printed around the part's outline before " +
    "infill. More walls = stronger part with negligible filament cost. " +
    "2 is the default; bump to 3-4 for parts that need to withstand " +
    "side-loading or screw threads.",

  outer_wall_speed:
    "Speed of the outermost visible wall — the one the eye sees. Lower " +
    "than the inner walls (typically half) because slower = better " +
    "surface finish and corners that don't round off. 30-60 mm/s for " +
    "most printers; modern Klipper rigs can push it higher.",
};
