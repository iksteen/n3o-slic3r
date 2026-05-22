# PR-1-8 — Reference profiles (A1 mini, U1, plates, filaments)

Status: ❌ open.

**Scope.** Author the cascade content (TOML files) and
context-state files (JSON / TOML) for the two MVP printers + their
common build plates + a baseline filament set. Lives under
`profiles/` at the workspace root — separate from the libslic3r
upstream tree to avoid colliding with OrcaSlicer's own profile
hierarchy. These become the *built-in defaults* shipped with the
app; users override via the `!important` tiers from PR-1-4.

**Acceptance criteria.**

- `profiles/printers/bambu-a1-mini.json` — PrinterProfile JSON
  with slot_count=4 (AMS Lite), single 0.4mm hotend, supported
  plates [Cool, Textured PEI, Smooth PEI, Engineering,
  SuperTack].

- `profiles/printers/snapmaker-u1.json` — PrinterProfile JSON
  with slot_count=4 (toolchanger), 4 per-toolhead configs each
  with their own nozzle diameter / hotend / max temp,
  ship-standard plate set.

- `profiles/plates/*.json` — one per build plate type the two
  printers share or unique to each:
  - `cool-plate.json`, `textured-pei.json`, `smooth-pei.json`,
    `engineering-plate.json`, `supertack-plate.json` (Bambu);
  - whatever U1 ships with on Day 1 (TBD — confirm with the
    Snapmaker manual before ticket scheduling).

- `profiles/filaments/*.json` — at minimum:
  - `generic-pla.json` with bed-temp rules across all 5 Bambu
    plate types (PLA on Cool=35, on Textured PEI=65, on SuperTack
    rejected) and any U1 plate variants.
  - `generic-petg.json` with the same plate coverage.

- `profiles/cascades/bambu-a1-mini-default.toml` and
  `profiles/cascades/snapmaker-u1-default.toml` — the default
  cascade for each printer. Authored by hand (not via the
  PR-0.5-1 converter — that's spike-throwaway). Each carries
  `[printer]` shorthand for printer-locked process settings (layer
  height = 0.2mm, wall_loops = 2, ...) plus filament/plate rules
  for the cross-product of supported filaments × plates.

- `profiles/cascades/_common/*.toml` — shared rules across both
  printers (PLA temperature curve, PETG temperature curve, etc.)
  that the per-printer cascade `include`s. (Include syntax: spike
  this if PR-1-2's parser doesn't support it yet — see PR-1-2's
  "future-proofing" item.)

- Tests:
  - The A1 mini cascade + plate + PLA filament resolves through
    PR-1-3's resolver without warnings against the canonical
    context (PLA in slot 0 on Textured PEI).
  - The U1 cascade resolves analogously for U1 + textured PEI
    + PLA in slot 0 + PETG in slot 1, with per-slot resolution
    differing where appropriate (slot 1's PETG gets PETG's
    nozzle_temperature, slot 0's PLA gets PLA's).

**Effort.** ~3-4 days. Most of the time is *deciding* values, not
authoring TOML. Cross-reference OrcaSlicer's own profiles for
sanity but don't copy verbatim — the cascade format and the
process-vs-filament-vs-plate split are different from OrcaSlicer's
preset model.

**Dependencies.** PR-1-2 (parser to validate the authored files),
PR-1-7 (context shapes to populate).

**Out of scope.** Per-printer firmware-tuning settings (Bambu's
`hotend_cooling_rate`, etc.) — those belong to Phase 5 driver
profiles, not Phase 1 cascades. Per-plate-type adhesion settings
— Phase 5+ hardware validation.

**Cut candidate.** Single PLA + single PEI = ~1 day. The other
filaments and plates can be authored against actual hardware in
Phase 7.
