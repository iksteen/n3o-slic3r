# Phase 3 exit-criteria smoke

Walks the project's Phase 3 deliverables end-to-end on a clean
checkout. Mirrors `docs/phase-0-smoke.md` through `phase-2-smoke.md`
— half automated (Rust + frontend tests), half human-driven (the
slice button in the viewport, which needs a real GUI session).

Phase 3 is where the **independent oracle** lands: parser +
serializer + 3MF reader + 3MF writer compose so a regression in any
of them flips the smoke red. Per Execution Plan §5:

> Parse, re-serialize, byte-diff equals zero on G-code; structural-
> diff equals zero on 3MF. This is the project's independent oracle
> — no external slicer needed.

## Automated half — runs in CI

```
$ cargo test --workspace    # CI uses debug profile; see note below
$ npm test
```

CI runs the workspace tests in **debug** mode behind the swap-
provisioning step (8 GB swapfile added before the cargo step) — runs
26313140866 (zvariant in release) and 26327551678 (webkit2gtk in
debug, no swap) both got SIGTERM from the cgroup soft-OOM. Debug
profile + swap together let every crate finish; perf-budget tests
still clear their headroom (translate p99 ≈ 1 µs vs the 5 ms ceiling,
parser ≈ 100 ms / 5 MB extrapolating to ~1 s / 50 MB in release).

Expected:

| Suite                          | Tests | Notes                                                  |
| ------------------------------ | ----- | ------------------------------------------------------ |
| (Phase 0-2 baseline)           |  168  | unchanged from phase-2-smoke                           |
| gcode model + parser + serializer | 30+ | PR-3-5 / PR-3-6 / PR-3-7                              |
| gcode header (multi-slicer)    |  10+  | PR-3-8                                                 |
| threemf reader + writer        |  15+  | PR-2-4 / PR-3-9                                        |
| threemf sliced writer          |  10+  | PR-3-10                                                |
| slice summary + errors         |  20+  | PR-3-3                                                 |
| slice orchestrator (integration) | 3   | PR-3-2 — slices the 20mmbox-LF fixture                 |
| gcode_parser_perf              |   1   | PR-3-6 — 50 MB equivalent < 3 s release                |
| phase3_smoke                   |   2   | this file                                              |
| frontend vitest (slice reducer)|   8   | PR-3-4                                                 |

Total: **~250 Rust tests + ~20 frontend tests, all green**. Any red
result is a regression to fix before tagging the phase.

## What `phase3_smoke.rs` exercises

Single chained `phase3_smoke_slice_parse_roundtrip_bundle` test:

1. **Slice end-to-end** — feeds the bundled A1 mini cascade + the
   `20mmbox-LF.stl` fixture through `core::slice::orchestrator::
   run_slice_job_blocking`. Asserts `PlateFinished` arrives with a
   non-zero `estimated_time_seconds` and `layer_count`. This is the
   integration glue PR-3-1 + PR-3-2 + PR-3-3 contribute to.
2. **Parser oracle** — parses the libslic3r-emitted G-code via
   `core::gcode::parser::parse_lines`. Asserts **zero**
   `ParseError`s. Any malformed numeric parameter or stream-level
   I/O slip would surface here.
3. **Serializer oracle** — re-serializes the parsed `Vec<Line>` via
   `core::gcode::serializer::write_lines`. Asserts the output is
   **byte-equal** to the original libslic3r emission. If the parser
   ever loses information (a token, a whitespace, a synthetic-vs-
   real flag) this assertion catches it; the failure message
   pinpoints the first diverging byte so the fix lands on the right
   line.
6. **Sliced 3MF bundling** — wraps the real slice output into a
   `.gcode.3mf` via `core::threemf::write_sliced_3mf`, then unzips
   `Metadata/plate_1.gcode` and asserts byte-equality. This is the
   send-to-printer path Phase 7a will use.

Standalone `phase3_smoke_3mf_roundtrip` test:

5. **3MF round-trip (structural)** — loads
   `examples/spike3/fourcolor.3mf`, writes it via PR-3-9's
   `write_3mf`, reloads the result, asserts mesh count, object
   count, per-mesh vertex + index count, and plate assignments are
   preserved. The test skips with a message when the fixture is
   missing.

Step 4 from the ticket (50 MB parser perf gate) is covered by the
existing `gcode_parser_perf.rs` test that runs in the same `cargo
test --workspace` step — not duplicated here.

## Human-driven half — slice loop

```
$ npm run tauri dev
```

The window should open within ~2 s on a warm build.

1. **Bed renders** + **+ Cube** — same as phase-2-smoke steps 1-2.
   A 20 mm cube sits at the bed center.
2. **Pick a model.** Click the **Pick model…** button in the
   header. The Tauri file picker opens. Choose any STL — the
   bundled OrcaSlicer test fixtures under
   `external/OrcaSlicer/tests/data/test_stl/ASCII/` work
   (e.g. `20mmbox-LF.stl`). The button label updates to the
   basename.
3. **Click Slice.** The green **Slice** button appears once a model
   is picked. Clicking it should:
   - Replace the button with a red **Cancel** button while in flight.
   - Show the progress bar moving from 0 → 100% over a few seconds
     (the cube is small; large prints take longer).
   - Show stage labels updating ("perimeter", "infill", …) per
     libslic3r's progress callback.
4. **Summary card appears.** When the slice completes, the progress
   bar disappears and a summary card renders in the header showing
   the estimated print time, aggregated filament use
   ("4.2g · 1.40m"), and layer count. The output `.gcode` file lands
   at `/tmp/n3o-slice-<timestamp>/plate_1.gcode` (path on the OS
   temp dir).
5. **Cancel mid-slice.** Slice again (Clear → Pick → Slice). Within
   the first second click **Cancel**. The button label flips to
   "Cancelling…" and within a few hundred ms the status transitions
   to "cancelled". Output file may or may not exist depending on
   how far libslic3r got — the orchestrator cooperates at plate
   boundaries.
6. **Failure surfaces typed error.** (Optional) Edit the bundled
   cascade to introduce a deliberately invalid setting (e.g.,
   `layer_height = "0"`), restart the app, slice. The summary slot
   shows `invalid config (layer_height): …` instead of a summary.
7. **Reload while running.** (Optional, dev-only) Start a slice;
   while the progress bar moves, hit `Cmd-R` / `Ctrl-R` in the
   dev window to reload the renderer. The slice continues on the
   backend; the panel rebuilds from `slice_status` and resumes
   showing progress within a couple seconds.

## Out of scope here

- **Multi-plate UI** — Phase 5. PR-3-2 supports a `plate_ids` list
  but PR-3-4 wires only `[1]`.
- **Scene → temp 3MF dump** — also Phase 5. PR-3-4's slice path
  takes a file the user picks rather than the live scene; until the
  dump lands the viewport is for visual feedback only on the
  slice side.
- **Settings overrides from the UI** — Phase 4 (Settings UI). The
  smoke uses the bundled A1 mini defaults.
- **End-to-end real-print validation** — Phase 7a's responsibility.
  Phase 3's exit criterion stops at "produces a `.gcode` /
  `.gcode.3mf` that round-trips through the parser oracle."
- **Tool-change count assertion** — that's PR-3-11; the smoke logs
  the tool-change count if PR-3-11 has landed but doesn't gate on
  a budget.

## If a step fails

- **Rust tests red:** `cargo test --workspace -- --nocapture`
  surfaces the failure messages. The smoke's chained assertions
  print "step N: …" prefixes so the broken link names itself.
- **Vitest red:** `npm test -- --reporter=verbose` for per-case
  context.
- **Slice button does nothing:** open DevTools, look for a Tauri
  IPC error. Most likely cause: cascade TOML embedded in the
  binary failed to parse — fix the TOML and rebuild.
- **Progress bar stuck at 0:** the FFI progress callback didn't
  fire. PR-3-1 wires it process-globally; only one job can be
  registered at a time. If a previous slice's callback was never
  cleared this can happen — restart the app.
- **Byte-diff non-zero in step 3:** the parser dropped something
  the serializer can't reproduce. The failure message names the
  byte offset + a ±40-byte window of original vs round-trip; that
  pinpoints the line. Usually a new G-code token landed in the
  fixture that PR-3-5's `Line` enum doesn't carry — extend the
  model, don't paper over with a "best-effort" pass.
- **3MF structural drift:** the writer is missing a field the
  reader populates. Compare `Project3mf` fields between original
  and reloaded; PR-3-9's writer covers core spec + BBS metadata
  but the open-question list in `docs/3mf-format-notes.md`
  enumerates what it punts on.
