// Abbreviate long option unit suffixes so they fit the narrow value field.
// libslic3r's sidetext is otherwise shown verbatim ("mm", "°C", "%", …); a few
// words ("layers") overflow the field, so map those to a short form. The full
// text stays available via the span's `title`.
const UNIT_ABBR: Record<string, string> = {
  layers: "lr",
  layer: "lr",
};

export function shortUnit(sidetext: string | null | undefined): string | null {
  if (!sidetext) return null;
  return UNIT_ABBR[sidetext] ?? sidetext;
}
