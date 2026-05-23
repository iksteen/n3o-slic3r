# Phase 3 — tickets

Phase 3 (end-to-end slice + G-code parser + 3MF I/O, ~3.5 person-weeks)
closes the **vertical slice** Execution Plan §3 calls out: by the end
of this phase the app loads a model, slices it, produces G-code, and
parses that G-code back into a typed model — all in-app, no external
slicer involved.

Phase 3 is also the foundation everything else depends on. The typed
G-code model is shared by:

- Phase 6 (preview): the renderer consumes the same typed Lines.
- Phase 7a (Bambu driver): wraps slice output as `.gcode.3mf`.
- Phase 7b (Snapmaker driver): sends raw G-code over HTTP, no 3MF
  involvement — exists in the dependency graph only to share the
  parser for header metadata extraction.
- Phase 8 (plugins): Lua bindings expose the typed model, not raw
  strings.

The 3MF writer is the **shared** I/O utility used by Phase 5 (project
save), Phase 6 (preview drag-drop of `.gcode.3mf`), and Phase 7a
(Bambu send format). Per PRD §8.2, it lives at `core/threemf/` so
each consumer takes a stable dependency on it.

Source: `docs/Execution_Plan.md` §5. Stated goal:

> Load model → slice → produce G-code → parse it into the typed model.
> Also: build the 3MF reader/writer module that this project will use
> everywhere a printer or another slicer touches a project file.

Individual tickets live one-per-file in `phase-3/`. This file is the
index plus phase-level status and notes.

## Status by deliverable

| Deliverable | Status | Ticket |
|-------------|--------|--------|
| FFI extensions: slice progress + log sink | ❌ open | [PR-3-1](phase-3/PR-3-1%20FFI%20slice%20progress%20and%20log%20sink.md) |
| Slice orchestration on a worker thread | ❌ open | [PR-3-2](phase-3/PR-3-2%20Slice%20orchestration.md) |
| Slice errors + post-slice summary | ✅ done | [PR-3-3](phase-3/PR-3-3%20Slice%20errors%20and%20summary.md) |
| Slice button + progress UI | ❌ open | [PR-3-4](phase-3/PR-3-4%20Slice%20UI.md) |
| Typed G-code model | ✅ done | [PR-3-5](phase-3/PR-3-5%20Typed%20gcode%20model.md) |
| Streaming G-code parser | ✅ done | [PR-3-6](phase-3/PR-3-6%20Streaming%20gcode%20parser.md) |
| G-code serializer (byte-equivalent round-trip) | ✅ done | [PR-3-7](phase-3/PR-3-7%20Gcode%20serializer.md) |
| Header metadata parser | ✅ done | [PR-3-8](phase-3/PR-3-8%20Header%20metadata%20parser.md) |
| Promote `core/threemf` + project-format writer | ✅ done | [PR-3-9](phase-3/PR-3-9%203MF%20writer.md) |
| `.gcode.3mf` writer (Bambu sliced format) | ✅ done | [PR-3-10](phase-3/PR-3-10%20Sliced%203MF%20writer.md) |
| Tool-change minimization investigation (carried from PR-0.5-3) | ❌ open | [PR-3-11](phase-3/PR-3-11%20Tool%20change%20investigation.md) |
| Phase 3 exit-criteria smoke | ❌ open | [PR-3-12](phase-3/PR-3-12%20Exit-criteria%20smoke.md) |

## Architecture invariant — the parser is the oracle

Phase 3 establishes the project's **independent verification approach**
for the slice loop. Per Execution Plan §5 exit criteria:

> Parse, re-serialize, byte-diff equals zero on G-code; structural-diff
> equals zero on 3MF. This is the project's independent oracle — no
> external slicer needed.

That means PR-3-7's serializer is load-bearing well beyond Phase 3 —
it's how Phase 7a validates Bambu compatibility, how plugins guarantee
they didn't corrupt a job, and how regression tests prove a refactor
didn't change the bytes that ship to a printer.

The typed model (PR-3-5) is the contract between the parser and every
downstream consumer. **Resist adding "string-only" fast paths** that
let plugins or the preview reach around the typed model. Every such
exception erodes the oracle. If the typed model isn't fast enough for
some hot path, optimize the typed model.

## Dependency graph

```
PR-3-1 (FFI: progress callback + log sink)
  └── PR-3-2 (slice orchestration: needs progress callback)
       └── PR-3-3 (errors + summary)
            └── PR-3-4 (slice UI)

PR-3-5 (typed model) ──► PR-3-6 (parser: produces typed model)
                       │
                       └► PR-3-7 (serializer: consumes typed model)
                            └── PR-3-8 (header metadata: parser + serializer share)

PR-3-9 (threemf writer)  ──► PR-3-10 (sliced .gcode.3mf: writer + slice output)

PR-3-6 + PR-3-7 + PR-3-2 + PR-3-9 + PR-3-10 ──► PR-3-12 (exit smoke needs all parts)

(PR-3-11 tool-change investigation runs in parallel — depends only on
 the slice loop being end-to-end, which lands at PR-3-2.)
```

## Exit criteria for the phase (from Execution Plan §5)

- A user can load a Benchy, click slice, get G-code, and the G-code
  parses cleanly into the typed model and round-trips identically.
- Parser handles 50MB G-code in under 3 seconds.
- 3MF round-trip: read a Bambu Studio `.3mf`, write it back, the
  result is structurally equivalent (model geometry, plate metadata,
  settings preserved within Bambu format expectations).
- Verification approach: parse, re-serialize, byte-diff equals zero
  on G-code; structural-diff equals zero on 3MF. This is the
  project's independent oracle — no external slicer needed.

## Cut candidates (from Execution Plan)

If pressed for time:

- **Header metadata parser (PR-3-8)** → defer to Phase 6 (Phase 6's
  preview needs the header data anyway). Saves ~1 day.
- **Per-plate filament cost calculation** (sub-deliverable of
  PR-3-3) → saves ~1 day.
- **Complex Bambu 3MF metadata extensions in PR-3-10** → write
  minimum-viable `.gcode.3mf`, validate by sending a job to A1
  mini in Phase 7a. Saves ~2 days but raises Phase 7a risk.

## What's *not* in Phase 3

- **Slice progress UI polish** — Phase 4 (Settings UI phase).
  Phase 3 ships a functional progress bar; visual refinement waits.
- **G-code preview** (layer slider, color modes, hover inspection) —
  Phase 6. Phase 3 only builds the parser the preview will consume.
- **`.gcode.3mf` consumption by the preview** — Phase 6. Phase 3's
  `.gcode.3mf` writer ships in PR-3-10; the corresponding reader
  for drag-drop preview is Phase 6 work.
- **Sync-on-send metadata population** — Phase 7c. Phase 3 emits
  the structural slots that filament-sync metadata will live in,
  but doesn't populate them.
- **Tool-change minimization fix** — investigation only (PR-3-11);
  the fix itself, if it lands here, is contained to the cascade-
  adapter side (since per PR-0.5-3 we don't expect to touch
  libslic3r itself). If the investigation surfaces something bigger,
  it spills into Phase 5.
- **Lua plugin host** — Phase 8. Phase 3 builds the typed G-code
  model the plugins will bind to, not the host itself.

## Open questions seeded for the implementer

- **G-code-to-memory-buffer in `slic3r-ffi`.** PRD §8.3 lists this
  as "if not already supported." Survey the FFI surface before
  starting PR-3-2: if `slic3r_ffi::slice` already accepts a buffer
  callback, the orchestration ticket can skip that prerequisite. If
  not, fold it into PR-3-1.
- **OrcaSlicer comment dialect.** PR-3-6's feature-type annotation
  depends on `;TYPE:perimeter` style comments. We've inherited
  Orca's emission, but confirm against the spike outputs that the
  comments we see in production match the tokens we recognize.
  PR-0.5-1's `examples/spike1/` G-code is the canonical fixture.
- **`.gcode.3mf` Bambu metadata schema.** PR-0.5-3 (`docs/spikes/`)
  has the inventory but the writing side hasn't been exercised.
  PR-3-10 should call out which fields are populated vs. left
  empty (and why) so Phase 7a's end-to-end print test can verify
  what matters.
