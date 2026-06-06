# PR-0.5-5 — platecycler portability

Status: ✅ done. Finding doc: `docs/dev/spikes/spike-5-platecycler.md`. Platecycler operates on libslic3r-emitted gcode bodies; the transform pipeline is dialect-agnostic at the gcode level. Phase 5's .gcode.3mf wrapper just needs to emit BBS-shaped metadata.

**Scope.** Confirm that the existing platecycler Python tool — the
G-code-transform pipeline that drives the compose-hook plugin in
Phase 8 — still works against G-code produced by libslic3r (vs the
Bambu Studio output it was originally validated on).

**Acceptance criteria.**

- Clone platecycler from `https://github.com/iksteen/platecycler/`
  and run its install/setup steps. Record the exact commit used in
  the finding doc.
- Pick one G-code file from PR-0.5-3 (`/tmp/spike3.gcode`) and run
  platecycler against it end-to-end. Compare the transform output
  against platecycler's expected output (either by re-running
  against a Bambu Studio gcode and diffing, or by checking against
  a stored golden if platecycler has one).
- The finding doc at `docs/dev/spikes/spike-5-platecycler.md`
  documents:
  - any divergences in the transform output (gcode comment
    differences, layer marker differences, extruder reset
    differences, anything else);
  - whether platecycler's regex/parser assumptions hold against
    libslic3r's emitter dialect or break in concrete spots;
  - the concrete shopping list for Phase 8's compose-hook
    implementation (what platecycler needs to be portable to either
    emitter, or what we adjust on our side).

**Effort.** 1 day.

**Dependencies.** PR-0.5-3 produces the test gcode.

**Out of scope.** Re-implementing platecycler in Lua / Rust —
Phase 8. Adding compose-hook support to the slicer — Phase 8.
Fixing any platecycler bugs encountered — file an issue against
platecycler, don't fix in this repo.
