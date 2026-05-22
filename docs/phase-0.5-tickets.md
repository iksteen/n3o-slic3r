# Phase 0.5 — tickets

Phase 0.5 (Engine validation spikes, ~1 person-week) runs five focused
experiments before Phase 1 commits to the cascade design. Each spike
is small, throwaway, and produces a written finding document. The
goal is to validate assumptions cheaply — a passed spike unblocks the
downstream phase; a failed spike triggers a plan revision.

Source: `docs/Execution_Plan.md` §2.5. Stated exit criteria:

> Five findings documents committed to the repo (one per spike), each
> with: assumption tested, method, result, implications for
> downstream phases. Any failed spike has a corresponding
> plan-revision PR open, not deferred.

## Status by spike

| Spike | Status | Notes |
|-------|--------|-------|
| Spike 1 — Cascade adapter end-to-end | ❌ open | **P0.5-1** |
| Spike 2 — Mixed-nozzle-size slice (Prusa XL) | ❌ open | **P0.5-2** |
| Spike 3 — Bambu A1 mini AMS slice | ❌ open | **P0.5-3** |
| Spike 4 — coEnums known limitation impact | ✅ done | Surfaced via `1bb3503`; nine affected options identified in `docs/libslic3r-workarounds.md` §5. No critical-path A1/U1 keys hit. |
| Spike 5 — platecycler portability | ❌ open | **P0.5-5** |

Findings docs land at `docs/spikes/spike-<n>-<slug>.md`. The Spike 4
finding is documented inline in `docs/libslic3r-workarounds.md`; the
others get dedicated files.

## Findings doc template

Every finding doc follows the same shape so they're skimmable as a
set:

```markdown
# Spike <n>: <slug>

## Assumption tested
<one paragraph — the precise claim being verified>

## Method
<numbered steps that someone else could re-run>

## Result
<pass / partial / fail, with the concrete evidence>

## Implications for downstream phases
<one paragraph per downstream phase impacted; explicit
recommendations: proceed as planned / revise the plan in X way /
defer Y>

## Artifacts
<paths to scripts, test inputs, captured gcode, screenshots>
```

If a spike fails, the implications section names the plan-revision
PR (link it once filed).

---

## P0.5-1 — Cascade adapter end-to-end

**Scope.** The walking-skeleton of the Phase 1 architecture: a real
OrcaSlicer device profile, converted into our cascade format, fed
through a stub resolver and stub adapter, dispatched to libslic3r,
producing valid gcode. Throwaway code; the goal is to find FFI gaps
and dispatch-quirk surprises early, not to build the production
resolver.

The seed config **must** be a converted OrcaSlicer device profile —
not a hand-rolled minimum config. P0-5 confirmed that
`Print::validate()` rejects FullPrintConfig defaults before slicing
starts; the spike's value comes from exercising the full round-trip
against config shapes we'll actually see in Phase 1+.

**Acceptance criteria.**

- `external/OrcaSlicer/resources/profiles/BBL/machine/Bambu Lab A1
  mini 0.4 nozzle.json` is converted into a TOML rule cascade. The
  conversion script (Rust or Python, doesn't matter) lives at
  `scripts/spikes/convert_orca_profile.<ext>` and is committed.
  Output cascade lands at
  `examples/cascades/bambu-a1-mini-spike1.toml`.
- The cascade is composed of a default rule + at least one filament
  rule + at least one plate-type rule (per `docs/profiles.md`
  "Worked example"). Specificity-based resolution is exercised by
  having two rules match the test context with different
  specificities.
- A stub resolver in `src-tauri/src/core/cascade/spike1.rs` (or a
  standalone `examples/` binary) reads the cascade, resolves it
  against a context object, and produces a flat `BTreeMap<String,
  String>` of resolved (key, serialized value) pairs. No
  `!important` tier handling needed for this spike — just authored
  cascade with specificity.
- A stub adapter consumes the flat map, applies the dimensional
  expansion documented in `docs/profiles.md` (bed temp at minimum),
  and emits a `slic3r_ffi::Config` ready for `Print::apply`.
- `slic3r_ffi::slice()` produces a non-empty G-code file at
  `/tmp/spike1.gcode` against the
  `external/OrcaSlicer/resources/handy_models/OrcaCube_v2.3mf`
  model (or another known-good 3MF — record which).
- The finding doc at `docs/spikes/spike-1-cascade-adapter.md`
  documents:
  - the cascade vocabulary actually used (which Orca keys mapped 1:1,
    which needed dispatch normalization, which were unused);
  - any FFI surface gaps encountered (missing `Config::set` accepts,
    enum serialization quirks beyond the coEnums ones, etc.);
  - any libslic3r dispatch quirks discovered beyond those already
    in `docs/libslic3r-workarounds.md` (and updates to that doc if
    new ones turn up).

**Effort.** 1–2 days. The Orca-profile conversion is the unknown;
the resolver and adapter stubs are mechanical.

**Dependencies.** Phase 0 complete (FFI link, scene state, core/
modules in place).

**Out of scope.** Production resolver (Phase 1). Two-phase
`!important` resolution (Phase 1). Translation manifest as a TOML
file (Phase 1; the spike's manifest can be hardcoded Rust). Multiple
device profiles (this spike does one device end-to-end; mixed-nozzle
is Spike 2, multi-color AMS is Spike 3). Any UI work. Any unit
tests beyond "did it slice."

---

## P0.5-2 — Mixed-nozzle-size slice (Prusa XL)

**Scope.** Validate libslic3r's per-toolhead independence claim —
specifically, that we can drive a Prusa XL profile with 0.4mm on
tool 0 and 0.6mm on tool 1 and get sensible per-tool extrusion
widths and tool-change G-code. This is the engine-validation half
of the Snapmaker U1 toolchanger story (U1 has different toolhead
counts but the per-tool-config concern is the same).

**Acceptance criteria.**

- A test driver (`examples/` or `scripts/spikes/`) loads
  `external/OrcaSlicer/resources/profiles/Prusa/machine/Prusa XL.json`,
  forces tool 0 to a 0.4mm nozzle and tool 1 to a 0.6mm nozzle, and
  slices a small 2-color test model (a 20mm cube with two color
  regions is fine; document the model used). The driver may build
  on Spike 1's stub adapter or stand alone.
- The output gcode is captured at `/tmp/spike2.gcode` and inspected
  for:
  - per-tool extrusion widths matching each nozzle (i.e. tool 0
    extrusion width ≈ 0.45mm, tool 1 ≈ 0.65mm — confirm with the
    actual libslic3r heuristic);
  - tool-change G-code (`T1`, `T0`) at color boundaries;
  - independent retraction/wipe settings per tool (or a documented
    explanation if libslic3r doesn't expose this for the XL profile).
- The finding doc at `docs/spikes/spike-2-mixed-nozzle.md`
  records:
  - whether per-tool extrusion width is honored end-to-end;
  - whether per-tool retraction/wipe/jerk/accel are honored;
  - what the U1 driver will need to model (since U1's toolchanger
    semantics differ from the XL's even though the per-tool-config
    concern is shared).

**Effort.** 1 day.

**Dependencies.** P0.5-1 ideally (reuses the stub adapter), but
fallback is to set the Prusa XL config directly via
`Config::set_string` without going through the cascade. Document
which path was taken.

**Out of scope.** Real U1 profile slicing — that's Phase 5 driver
work. Toolchange purge tower behavior — Spike 3 looks at purges in
the AMS context. Comparing to PrusaSlicer's reference gcode — nice
to have but not required (we already trust libslic3r's tool dispatch
because OrcaSlicer ships it).

---

## P0.5-3 — Bambu A1 mini AMS slice

**Scope.** Slice a 4-color test model with the A1 mini profile + AMS
multi-color, and characterize the gaps between libslic3r's output
and what Bambu Studio produces for the same input. The purge-volume
model, the 3MF metadata, and the AMS bindings format are all
load-bearing for Phase 5 (Bambu connectivity) and Phase 7 (filament
sync) — finding the divergences early lets those phases plan around
them.

**Acceptance criteria.**

- Test model: a 4-color 3MF (e.g., a calibration cube with 4 color
  regions, or one of OrcaSlicer's AMS calibration fixtures —
  document the choice and provenance). Lives at
  `examples/spike3/fourcolor.3mf`.
- A1 mini machine profile (`Bambu Lab A1 mini.json` + 0.4mm nozzle
  variant) loaded via Spike 1's adapter (or directly if Spike 1's
  adapter doesn't yet handle the AMS keys — note which).
- AMS multi-color bindings: 4 filaments assigned, each to a distinct
  AMS slot (1–4). Purge volumes left at default for the first run.
- Slice produces a `.gcode` at `/tmp/spike3.gcode` plus a wrapping
  `.gcode.3mf` at `/tmp/spike3.gcode.3mf`. Bambu Studio's output for
  the same input model + same profile is captured at
  `examples/spike3/bambu-studio-reference.gcode.3mf` (run Bambu
  Studio once, locally, and check in the reference artifact).
- The finding doc at `docs/spikes/spike-3-bambu-ams.md` documents:
  - the metadata-extension gaps between libslic3r's `.gcode.3mf`
    and Bambu Studio's (plate thumbnails, filament aggregates, print
    time field, AMS bindings format — full list);
  - the purge-volume structure libslic3r emits (single number vs
    pairwise matrix; which keys feed it);
  - which gaps are blockers for "send to A1 mini and have it print"
    vs which are cosmetic;
  - the concrete shopping list for Phase 5's "wrap sliced G-code
    into .gcode.3mf for send" item.

**Effort.** 1–2 days. Most of the time is the comparison work, not
the slice itself.

**Dependencies.** P0.5-1 (cascade adapter handles at least the
non-AMS A1 mini keys). A Bambu Studio install on the developer's
machine — confirm before scheduling. The 4-color test model needs
to be picked.

**Out of scope.** Actually sending the print to a real A1 mini —
Phase 5 hardware validation. AMS calibration G-code emission — Phase
5 / Phase 7. Resolving the metadata gaps in code — Phase 5's "wrap
sliced G-code" item is what consumes this finding.

---

## P0.5-5 — platecycler portability

**Scope.** Confirm that the existing platecycler Python tool — the
G-code-transform pipeline that drives the compose-hook plugin in
Phase 8 — still works against G-code produced by libslic3r (vs the
Bambu Studio output it was originally validated on).

**Acceptance criteria.**

- Locate the platecycler tool. **Open question:** where does it
  live? Local path, a separate GitHub repo, a Gist? Confirmed
  location goes in the finding doc.
- Pick one G-code file from Spike 3 (`/tmp/spike3.gcode`) and run
  platecycler against it end-to-end. Compare the transform output
  against platecycler's expected output (either by re-running
  against a Bambu Studio gcode and diffing, or by checking against
  a stored golden if platecycler has one).
- The finding doc at `docs/spikes/spike-5-platecycler.md` documents:
  - any divergences in the transform output (gcode comment
    differences, layer marker differences, extruder reset
    differences, anything else);
  - whether platecycler's regex/parser assumptions hold against
    libslic3r's emitter dialect or break in concrete spots;
  - the concrete shopping list for Phase 8's compose-hook
    implementation (what platecycler needs to be portable to either
    emitter, or what we adjust on our side).

**Effort.** 1 day.

**Dependencies.** P0.5-3 produces the test gcode. Platecycler tool
location must be confirmed.

**Out of scope.** Re-implementing platecycler in Lua / Rust — Phase
8. Adding compose-hook support to the slicer — Phase 8. Fixing any
platecycler bugs encountered — file an issue against platecycler,
don't fix in this repo.

---

## Notes on what's *not* in Phase 0.5

Spikes are throwaway. None of the spike code is expected to live
past Phase 1; the only durable artifacts are the finding documents
and any updates to `docs/profiles.md`, `docs/libslic3r-workarounds.md`,
or the FFI shim that the spikes prompted.

In particular:

- **Don't try to make the spike's resolver production-quality.** The
  Phase 1 resolver gets a clean-room reimplementation informed by
  the finding doc.
- **Don't carry spike code into `src-tauri/src/core/` permanently.**
  Spike code can live there during the spike (it's where you'll be
  testing the FFI from), but delete or stub it back at spike end.
- **Don't bundle multiple spikes into one PR.** Each spike's PR
  carries its scripts, fixtures, and finding doc. Reviewer can
  consume them independently.

## Exit criteria for Phase 0.5

- Four new finding docs at `docs/spikes/spike-{1,2,3,5}-*.md`
  (Spike 4's finding is in `libslic3r-workarounds.md` §5).
- Any failed spike has a plan-revision PR open against
  `docs/PRD.md` or `docs/Execution_Plan.md`.
- `docs/libslic3r-workarounds.md` reflects any newly discovered
  quirks (and removes any of the existing five that turned out to
  be already fixed upstream).
- The Phase 0.5 milestone (`M0.5 — Engine assumptions validated` in
  `docs/Execution_Plan.md`) ticks over.

## Cut candidates (from `Execution_Plan.md`)

If pressed for time:

- **Spike 5 (platecycler)** can slip to "early Phase 8." Cost: Phase
  8 starts cold against an unknown.
- **Spike 4 (coEnums)** is already done; no decision needed.

Spikes 1, 2, 3 are not cut candidates — each de-risks a downstream
phase whose architecture depends on the answer.
