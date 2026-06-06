# Phase 1 — exit-criteria smoke

This document captures the smoke procedure that validates Phase 1's
exit criteria (from `docs/dev/Execution_Plan.md` §3):

> Resolver returns correct effective values + trace for A1 mini +
> PEI + PLA. Adapter produces a `DynamicPrintConfig` libslic3r
> accepts. Trace reports winner + losers correctly. Absolute
> override behavior. Load-time validation catches typos. Resolver
> benchmarks under 10 ms full resolve; under 100 ms with adapter.

Run this smoke after any of:

- bumping the libslic3r submodule
- touching `core/schema/`, `core/cascade/`, `core/cascade_adapter/`,
  `core/project/context.rs`, or any of the reference profiles
- bumping the Rust toolchain

The CI workflow (`.github/workflows/build.yml`, ticket PR-0-4)
runs steps 1 and 2 on every push. Step 3 is local-only today.

## Procedure

### 1. Workspace tests

```bash
cargo test --workspace --release
```

**Expected.** All PR-1-* unit + integration tests pass: 79 tests
across schema, loader, validate, resolver, overrides, trace,
adapter, manifest, printer/scene/filament/context, and the
`reference_profiles` integration. The slic3r-ffi crate also runs
its 16 `api.rs` tests as part of the workspace.

### 2. Phase 1 exit smoke

```bash
cargo run -p n3o-slic3r --release --example phase1_smoke
```

**Expected.** Six labelled steps complete cleanly. Key assertions:

```
=== Phase 1 exit-criteria smoke ===

[1/6] Loading reference profiles (TOML)
  printer:  Bambu A1 mini (4 slots, 1 toolheads)
  plate:    Textured PEI → libslic3r curr_bed_type = "Textured PEI Plate"
  filament: Generic PLA (PLA)

[2/6] Building SlicingContext
  active_slot = 0

[3/6] Parsing + validating cascade
  cascade: 4 rules parsed
  validation: OK

[4/6] Resolving cascade against context
  resolved keys: 20
              layer_height = 0.2          (spec=0)
        nozzle_temperature = 220          (spec=1)
                  bed_temp = 65           (spec=2)

[5/6] Tracing bed_temp
bed_temp = 65 (cascade)
  winner            spec=2 filament.type = "PLA" + plate.type = "Textured PEI" at
                   bambu-a1-mini-default.toml:57 → set.bed_temp = 65
  loser:            spec=1 plate.type = "Textured PEI" at bambu-a1-mini-default.toml:50
                   → set.bed_temp = 65

[6/6] Applying project override + running adapter
bed_temp = 50 (override)
  override: tier=project at project.toml:1 → set.bed_temp = 50
  cascade_fallback  spec=2 … → set.bed_temp = 65
  loser:            spec=1 … → set.bed_temp = 65
  adapter: 20 accepted, 0 dropped, 0 remapped, 0 unknown, 0 parse-error,
           12 keys filled by bed_temp expansion
  Config spot-check: layer_height="0.2", hot_plate_temp="50",
                     curr_bed_type="Textured PEI Plate"

=== smoke OK ===
```

This single binary exercises every Phase 1 deliverable end-to-end:
PR-1-1 (schema), PR-1-2 (loader + validation), PR-1-3 (resolver),
PR-1-4 (overrides), PR-1-5 (trace), PR-1-6 (adapter), PR-1-7
(context), PR-1-8 (reference profiles).

### 3. Validation error-message regression check

(Forthcoming as `examples/phase1_validation_errors`.) Exercises the
three documented validation error classes — unknown predicate
dimension, unknown set key, scope violation — confirming each
surfaces with a clear file:line message and (where applicable) a
Levenshtein-distance suggestion.

### 4. Resolver benchmarks

(Forthcoming as `cargo bench --bench cascade` — PR-1-10.)

**Targets:**
- A1 mini + PEI + PLA full resolve: < 10 ms (FR-CAS-11).
- Resolve + adapter expansion: < 100 ms.

## Divergence checklist

Any divergence from the documented expected output is **either**:

1. A bug — file it, fix it. Example: validation error class
   regression, missing trace entries, adapter dropping known keys.
2. Expected upstream change — update this document. Example:
   reference cascade gains a new specificity-3 rule.

If you can't decide which, lean toward (1) and investigate.

## Out of scope for this smoke

- **Actual slicing** — Phase 1's smoke verifies the cascade →
  adapter → Config pipeline, not slicing itself. End-to-end slice
  through this pipeline lands when PR-1-9 wires the adapter into
  the Tauri command surface and PR-1-11's smoke binary calls
  `slic3r_ffi::slice()` on the produced Config.
- **Tool-change minimization** — PR-1-12 owns the carry-forward
  investigation from PR-0.5-3.
- **U1 + additional plates / filaments** — deferred via the
  Execution Plan's PR-1-8 cut candidate.
- **GUI smoke** — Phase 4.
