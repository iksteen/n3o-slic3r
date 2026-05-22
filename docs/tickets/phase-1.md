# Phase 1 — tickets

Phase 1 (Rule cascade resolver + translation adapter, ~4
person-weeks) implements the production-grade cascade described in
`docs/profiles.md` and §6.1 / §8.2 of the PRD. By the end of the
phase, the resolver loads TOML cascades + user/project overrides,
returns resolved settings with trace metadata, and the adapter
translates the result into a `slic3r_ffi::Config` that libslic3r
accepts. No UI yet — Settings UI is Phase 4.

Source: `docs/Execution_Plan.md` §3. Stated goal:

> Working rule cascade resolver and translation adapter, fully
> tested, with no UI yet.

Individual tickets live one-per-file in `phase-1/`. This file is
the index plus phase-level status and notes.

## Inputs from Phase 0.5

The Phase 0.5 spikes hand off concrete shopping lists to Phase 1:

- **PR-0.5-1** (cascade adapter spike): walking-skeleton validated.
  Production resolver replaces the stub in `src-tauri/examples/
  spike1.rs`. The 67 OrcaSlicer-specific keys logged at adapt-time
  (Bambu) and the 13 from Prusa become the translation manifest's
  initial drop list, including the five Orca-side typos to silently
  remap rather than warn on.
- **PR-0.5-2** (mixed nozzle): `*_line_width` keys are *scalars*
  in libslic3r — per-tool extrusion volume is computed dynamically
  by the flow calculator. The adapter doesn't need per-tool width
  plumbing; cascade rules authoring `wall_filament` etc. as scalars
  is fine.
- **PR-0.5-3** (Bambu AMS): the `.gcode.3mf` wrapper sits *above*
  the FFI (Phase 5 work, not Phase 1). The tool-change disparity
  vs Bambu Studio (76 vs 7 on the canonical input) remains
  unresolved — see `docs/spikes/spike-3-bambu-ams.md`. Phase 1
  should keep a half-open ticket on it (**PR-1-12**) and investigate
  whenever the adapter is touched.

## Status by deliverable

| Deliverable | Status | Ticket |
|-------------|--------|--------|
| Schema generator (libslic3r option introspection → typed Rust) | ✅ done | [PR-1-1](phase-1/PR-1-1%20Schema%20generator.md) |
| TOML cascade loader + parser + load-time validation | ✅ done | [PR-1-2](phase-1/PR-1-2%20Cascade%20loader%20and%20parser.md) |
| Rule resolver (authored cascade — specificity + source order) | ✅ done | [PR-1-3](phase-1/PR-1-3%20Rule%20resolver%20authored%20cascade.md) |
| Override tiers (user + project, !important-style) | ✅ done | [PR-1-4](phase-1/PR-1-4%20Override%20tiers.md) |
| Trace tooling | ✅ done | [PR-1-5](phase-1/PR-1-5%20Trace%20tooling.md) |
| Translation adapter + manifest | ❌ open | [PR-1-6](phase-1/PR-1-6%20Translation%20adapter%20and%20manifest.md) |
| Context-state structures (printer / build plate / filament) | ❌ open | [PR-1-7](phase-1/PR-1-7%20Context-state%20structures.md) |
| Reference profiles (A1 mini, U1, plates, filaments) | ❌ open | [PR-1-8](phase-1/PR-1-8%20Reference%20profiles.md) |
| Tauri command surface | ❌ open | [PR-1-9](phase-1/PR-1-9%20Tauri%20command%20surface.md) |
| Resolver benchmarks | ❌ open | [PR-1-10](phase-1/PR-1-10%20Resolver%20benchmarks.md) |
| Phase 1 exit-criteria smoke | ❌ open | [PR-1-11](phase-1/PR-1-11%20Exit-criteria%20smoke.md) |
| Tool-change minimization investigation (carried from PR-0.5-3) | ❌ open | [PR-1-12](phase-1/PR-1-12%20Tool-change%20minimization%20investigation.md) |

## Dependency graph

```
PR-1-1 (schema)
  ├── PR-1-2 (loader uses schema for load-time validation)
  ├── PR-1-6 (adapter uses schema for type-safe Config::set)
  └── PR-1-7 (context-state structures reference schema for valid options)

PR-1-2 (loader) ──► PR-1-3 (resolver consumes parsed cascade)
                  ├── PR-1-4 (overrides apply on top of authored cascade)
                  └── PR-1-5 (trace integrates with resolver internals)

PR-1-6 (adapter) ──► PR-1-9 (Tauri surface exposes adapt())

PR-1-7 (context shapes) ──► PR-1-8 (reference profiles instantiate them)

PR-1-3 + PR-1-6 + PR-1-8 ──► PR-1-10 (benchmarks need real data)
                          └── PR-1-11 (exit smoke needs real data)

PR-1-12 is independent; can run in parallel with adapter work.
```

## Exit criteria for the phase (from Execution Plan §3)

- Resolver returns correct effective values + trace for A1 mini +
  PEI + PLA in slot 0 and U1 + textured PEI + PLA in slot 0 +
  PETG in slot 1.
- Adapter produces a `DynamicPrintConfig` that libslic3r accepts;
  slicing produces gcode (correctness validation is Phase 3 work).
- Trace tool reports winner + losers correctly for a 3-rule case
  at specificities 0/1/2.
- Absolute override behavior: a `project.bed_temp = 50` beats a
  filament+plate rule at specificity 2; trace reports both.
- Load-time validation catches: misspelled predicate, set key not
  in schema, scope violation.
- Resolver benchmarks: <10 ms full 4-slot resolve, <100 ms with
  adapter expansion.
- Comprehensive test coverage on `core/cascade/` and
  `core/cascade_adapter/`.

## Notes on what's *not* in Phase 1

- **Settings UI** — Phase 4. The Tauri command surface (PR-1-9) is
  enough to drive it from a CLI test harness or future UI; no
  visual rendering of cascade traces lives here.
- **3D viewport** — Phase 2. Phase 1's adapter dumps to a slice
  call but doesn't render anything.
- **End-to-end slice through a UI** — Phase 3.
- **Multi-printer projects** — Phase 5.
- **G-code preview** — Phase 6.
- **Printer connectivity** — Phase 7.
- **Plugin / Lua hook system** — Phase 8.

## Cut candidates (from Execution Plan)

If pressed for time:

- **Trace tool's "matching-but-losing rules" list** (PR-1-5) →
  winner only saves ~1 day. Hurts FR-CAS-7 UX but the source badge
  still works.
- **Property tests** (PR-1-3) → golden tests only saves ~2 days.
  Reduces confidence in cascade-edge-case correctness.
- **Reference profiles beyond a minimum set** (PR-1-8) → single
  PLA + single PEI saves ~2 days. Pushes profile authoring into
  Phase 7 hardware testing.

PR-1-12 (tool-change minimization) is *not* a Phase 1 cut candidate
in the sense that the work itself can defer to Phase 5; the
*investigation* should happen in Phase 1 while the spike3 context
is still fresh.
