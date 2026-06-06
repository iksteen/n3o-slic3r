# PR-3-11 — Tool-change minimization investigation (carried from PR-0.5-3)

Status: ❌ open.

**Scope.** Re-open the PR-0.5-3 spike's unanswered question:
slicing the 4-color Benchy through our cascade-adapter pipeline
produces **76 tool changes**, while Orca and BBS produce **7** on
the same input. The spike documented the disparity but deferred the
fix; now that Phase 3 has the G-code parser, parser-driven A/B
diffs of the slice output are cheap and the right time to bite this
off is here.

Per the `libslic3r_vs_our_invocation` memory + the user's direction
("It might require some extra insight into libslic3r and orca
slicer, but I doubt we'll have to adjust anything in libslic3r
itself"), the working hypothesis is that we're missing a setup step
or feeding the adapter a different config from what Orca uses —
**not** that libslic3r needs patching.

**Acceptance criteria.**

- **Investigation phase** (~3 days):
  - Slice `examples/spike3/fourcolor.3mf` through our pipeline →
    `examples/spike3/n3o_output.gcode`.
  - Slice the same fixture through OrcaSlicer CLI →
    `examples/spike3/orca_output.gcode` (already done in PR-0.5-3
    — reuse the fixture).
  - Parse both via PR-3-6's parser. Filter to `Line::ToolChange`.
    Compare positions + sequence.
  - For the first divergence point, walk the surrounding
    `Line::Other` / `Line::Comment` records to see what differs.
  - Diff the `DynamicPrintConfig` our adapter emits vs. what Orca
    emits for the same project (Orca's `_project_settings.json`
    is in the 3MF; our adapter's output is straightforward to
    dump as TOML).
  - Document each finding in `docs/dev/spikes/spike-3.md` (extend the
    existing spike doc — don't create a new file).

- **Fix phase** (~1 day, if the investigation surfaces a clear
  cause):
  - Apply the fix on our side of the FFI (cascade adapter, slicing
    context construction, or `Print::apply` call). Per the memory
    rule, **do not patch libslic3r**.
  - Add a regression test under `src-tauri/tests/` that re-slices
    the fourcolor fixture, parses, and asserts ≤ 8 tool changes
    (matching Orca + a small headroom for our slicing-context
    differences).

- If the investigation surfaces something that genuinely requires a
  libslic3r-side change, **stop and surface it to the user** before
  proceeding. The memory rule says we shouldn't assume libslic3r is
  the bug; if the evidence overturns that, get a green light first.

- Surface artifacts:
  - Extended `docs/dev/spikes/spike-3.md` with the diff finding +
    root cause.
  - Regression test that fails if the disparity returns.
  - (Conditional) commit to the cascade adapter or scene/project
    side that closes the gap.

**Effort.** ~3-4 days. Investigation-heavy; the fix itself is
likely a few-line change once the cause is named.

**Dependencies.** PR-3-2 (slice loop runnable end-to-end), PR-3-6
(parser for A/B diffs), PR-3-7 (serializer for canonicalizing
output before diffing — though raw byte-diff via `diff(1)` is
probably enough here).

**Out of scope.** Mixed-nozzle tool-change minimization on U1 —
Phase 5+ when U1 hardware validation lands. Filament-map override
UX (FR-FS-*) — Phase 7c. Refactoring the cascade adapter beyond
what the fix requires.

**Risk.** This is the one Phase 3 ticket whose effort estimate is
soft because the cause is unknown. If the investigation balloons,
the right move is to land the parser + 3MF tickets first (so the
exit smoke can ship without this) and carry the investigation into
Phase 4 / Phase 5 as a separate work stream. The 76-vs-7 disparity
isn't a slice-correctness bug per se — the print works, it just
has more tool changes than necessary — so it's safe to defer if
the underlying cause is deep.
