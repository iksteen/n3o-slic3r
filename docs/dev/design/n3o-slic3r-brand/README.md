# n3o-slic3r — brand mark assets

The mark is a **layered “N”** — three sliced layers (bright blue → white → bright red)
stacked into an N, inside a dashed toolpath ring.

## Colors
| Role            | Value                              |
|-----------------|------------------------------------|
| Blue (top)      | linear `#00f2fe → #4facfe`         |
| Centre (dark)   | `#ffffff` on dark · `#161a22` on light |
| Red (bottom)    | linear `#ff4d5e → #ff1f3a`         |
| Icon bg (dark)  | `#14151c`                          |
| Icon bg (light) | `#eef1f5`                          |

**Rule:** the centre band is white on dark surfaces and ink (`#161a22`) on light
surfaces, so all three layers stay legible. The toolpath ring is decorative — drop it
below ~32 px (use the `icon-chip` / `mark` variants).

## Files

### `svg/` (vector, preferred — scale freely)
- `app-icon-dark.svg` / `app-icon-light.svg` — full mark, rounded bg + ring + glow
- `icon-chip-dark.svg` / `icon-chip-light.svg` — rounded bg, **no ring** (favicons, dock)
- `mark-dark.svg` / `mark-light.svg` — bare glyph, transparent bg (toolbars, inline)
- `mark-mono.svg` — bare glyph in `currentColor` (inherits text color; 1-color print)

### `png/` (raster)
- `app-icon-{dark,light}-{512,256,128,64,32,16}.png`
- `icon-chip-{dark,light}-{180,256,128,64,32,16}.png` (180 = apple-touch-icon)
- `mark-{dark,light}-{200,96,48}h.png` (height in px; 1.2 : 1 aspect)

## Usage notes
- **Favicon:** `icon-chip-light-32.png` (or `-16`) for browser tabs; `-180` for iOS home screen.
- **App / launcher icon:** `app-icon-dark` is the hero (glow reads best on dark). Use the
  matching theme variant where the OS supports it.
- **In-app toolbar:** use `mark-mono.svg` and set `color:` to your text token — it themes
  automatically. Or `mark-dark`/`mark-light` for the full-color glyph.
- **SVG > PNG** wherever the target supports it; PNGs are provided for places that don't.
