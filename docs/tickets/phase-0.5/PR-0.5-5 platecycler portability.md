# PR-0.5-5 — platecycler portability

Status: ❌ open.

**Scope.** Confirm that the existing platecycler Python tool — the
G-code-transform pipeline that drives the compose-hook plugin in
Phase 8 — still works against G-code produced by libslic3r (vs the
Bambu Studio output it was originally validated on).

**Acceptance criteria.**

- Locate the platecycler tool. **Open question:** where does it
  live? Local path, a separate GitHub repo, a Gist? Confirmed
  location goes in the finding doc.
- Pick one G-code file from PR-0.5-3 (`/tmp/spike3.gcode`) and run
  platecycler against it end-to-end. Compare the transform output
  against platecycler's expected output (either by re-running
  against a Bambu Studio gcode and diffing, or by checking against
  a stored golden if platecycler has one).
- The finding doc at `docs/spikes/spike-5-platecycler.md`
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

**Dependencies.** PR-0.5-3 produces the test gcode. Platecycler
tool location must be confirmed.

**Out of scope.** Re-implementing platecycler in Lua / Rust —
Phase 8. Adding compose-hook support to the slicer — Phase 8.
Fixing any platecycler bugs encountered — file an issue against
platecycler, don't fix in this repo.
