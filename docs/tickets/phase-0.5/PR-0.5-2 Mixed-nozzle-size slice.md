# PR-0.5-2 — Mixed-nozzle-size slice (Prusa XL)

Status: ⚠️ partial. Finding doc: `docs/spikes/spike-2-mixed-nozzle.md`. Mixed-nozzle config validated; toolchange-emission criterion deferred to PR-0.5-3 (its 4-color fixture covers both spikes' needs).

**Scope.** Validate libslic3r's per-toolhead independence claim —
specifically, that we can drive a Prusa XL profile with 0.4mm on
tool 0 and 0.6mm on tool 1 and get sensible per-tool extrusion
widths and tool-change G-code. This is the engine-validation half
of the Snapmaker U1 toolchanger story (U1 has different toolhead
counts but the per-tool-config concern is the same).

**Acceptance criteria.**

- A test driver (`examples/` or `scripts/spikes/`) loads
  `external/OrcaSlicer/resources/profiles/Prusa/machine/Prusa XL.json`,
  forces tool 0 to a 0.4mm nozzle and tool 1 to a 0.6mm nozzle,
  and slices a small 2-color test model (a 20mm cube with two
  color regions is fine; document the model used). The driver may
  build on PR-0.5-1's stub adapter or stand alone.
- The output gcode is captured at `/tmp/spike2.gcode` and inspected
  for:
  - per-tool extrusion widths matching each nozzle (i.e. tool 0
    extrusion width ≈ 0.45mm, tool 1 ≈ 0.65mm — confirm with the
    actual libslic3r heuristic);
  - tool-change G-code (`T1`, `T0`) at color boundaries;
  - independent retraction/wipe settings per tool (or a documented
    explanation if libslic3r doesn't expose this for the XL
    profile).
- The finding doc at `docs/spikes/spike-2-mixed-nozzle.md` records:
  - whether per-tool extrusion width is honored end-to-end;
  - whether per-tool retraction/wipe/jerk/accel are honored;
  - what the U1 driver will need to model (since U1's toolchanger
    semantics differ from the XL's even though the per-tool-config
    concern is shared).

**Effort.** 1 day.

**Dependencies.** PR-0.5-1 ideally (reuses the stub adapter), but
fallback is to set the Prusa XL config directly via
`Config::set_string` without going through the cascade. Document
which path was taken.

**Out of scope.** Real U1 profile slicing — that's Phase 5 driver
work. Toolchange purge tower behavior — PR-0.5-3 looks at purges in
the AMS context. Comparing to PrusaSlicer's reference gcode — nice
to have but not required (we already trust libslic3r's tool
dispatch because OrcaSlicer ships it).
