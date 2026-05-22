# PR-0.5-3 — Bambu A1 mini AMS slice

Status: ⚠️ done with Phase 5 prerequisite. Finding doc: `docs/spikes/spike-3-bambu-ams.md`. AMS slice + tool-change emission + filament-aggregate metadata + `.gcode.3mf` shape all confirmed. BBS comparison (via flatpak) surfaces a major issue: libslic3r-FFI emits 76 tool changes / 2h 55m print time vs BBS's 7 tool changes / 1h 3m on the *same input*. Must be solved before Phase 5 hardware validation.

**Scope.** Slice a 4-color test model with the A1 mini profile + AMS
multi-color, and characterize the gaps between libslic3r's output
and what Bambu Studio produces for the same input. The purge-volume
model, the 3MF metadata, and the AMS bindings format are all
load-bearing for Phase 5 (Bambu connectivity) and Phase 7 (filament
sync) — finding the divergences early lets those phases plan around
them.

**Acceptance criteria.**

- Test model: a 4-color 3MF painted and exported from OrcaSlicer
  itself (open a base mesh in OrcaSlicer, use its color-paint
  tooling to assign 4 filaments to distinct regions, save as
  `.3mf`). Using OrcaSlicer as the source — rather than Bambu
  Studio or a hand-authored 3MF — keeps the file format and
  metadata consistent with what libslic3r expects to consume.
  Lives at `examples/spike3/fourcolor.3mf`.
- A1 mini machine profile (`Bambu Lab A1 mini.json` + 0.4mm nozzle
  variant) loaded via PR-0.5-1's adapter (or directly if PR-0.5-1's
  adapter doesn't yet handle the AMS keys — note which).
- AMS multi-color bindings: 4 filaments assigned, each to a distinct
  AMS slot (1–4). Purge volumes left at default for the first run.
- Slice produces a `.gcode` at `/tmp/spike3.gcode` plus a wrapping
  `.gcode.3mf` at `/tmp/spike3.gcode.3mf`. Bambu Studio's output
  for the same input model + same profile is captured at
  `examples/spike3/bambu-studio-reference.gcode.3mf` (run Bambu
  Studio once, locally, and check in the reference artifact).
- The finding doc at `docs/spikes/spike-3-bambu-ams.md` documents:
  - the metadata-extension gaps between libslic3r's `.gcode.3mf`
    and Bambu Studio's (plate thumbnails, filament aggregates,
    print time field, AMS bindings format — full list);
  - the purge-volume structure libslic3r emits (single number vs
    pairwise matrix; which keys feed it);
  - which gaps are blockers for "send to A1 mini and have it print"
    vs which are cosmetic;
  - the concrete shopping list for Phase 5's "wrap sliced G-code
    into .gcode.3mf for send" item.

**Effort.** 1–2 days. Most of the time is the comparison work, not
the slice itself.

**Dependencies.** PR-0.5-1 (cascade adapter handles at least the
non-AMS A1 mini keys). A Bambu Studio install on the developer's
machine — confirm before scheduling. An OrcaSlicer install with
the AMS color-paint workflow — needed to produce the test
fixture.

**Out of scope.** Actually sending the print to a real A1 mini —
Phase 5 hardware validation. AMS calibration G-code emission —
Phase 5 / Phase 7. Resolving the metadata gaps in code — Phase 5's
"wrap sliced G-code" item is what consumes this finding.
