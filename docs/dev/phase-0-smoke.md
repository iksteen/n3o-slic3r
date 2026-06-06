# Phase 0 — exit-criteria smoke

This document captures the manual smoke procedure that validates Phase
0's stated exit criteria (from `docs/dev/Execution_Plan.md` §2):

> App launches on the project lead's primary dev machine. Frontend
> shows libslic3r version. CI green on Linux.

Run this smoke after any of the following, then update this document
with any divergence:

- bumping the OrcaSlicer submodule
- bumping the Rust toolchain or `Cargo.lock`
- bumping Node / npm dependencies
- changes to `scripts/build.sh` or the FFI cmake glue
- changes to the Tauri command surface

The CI workflow (`.github/workflows/build.yml`, ticket P0-4) runs
steps 1–3 on every push; the remaining steps require a GUI session
and are local-only.

---

## Procedure

### 1. Fresh checkout

```bash
git clone git@github.com:iksteen/n3o-slic3r.git
cd n3o-slic3r
git submodule update --init --recursive
```

### 2. Build OrcaSlicer's heavy deps tree

```bash
./scripts/build.sh deps
```

**Expected.** First run takes ~17 minutes; subsequent runs detect the
existing prefix and exit immediately with `deps: already built at …,
skipping`.

The deps prefix lands at
`external/OrcaSlicer/deps/build/OrcaSlicer_dep/usr/local/`. If it's
missing or partial, delete the entire `external/OrcaSlicer/deps/build/`
directory and rerun — the script is idempotent only when the prefix
is a complete success.

### 3. Workspace test suite

```bash
cargo test --workspace --release
```

**Expected.** All tests pass. Today that means 16/16 in
`crates/slic3r-ffi/tests/api.rs`. The release profile is required
because the FFI shim links libslic3r_ffi.so via cmake — debug-mode
tests work but take noticeably longer.

If a test fails with a libslic3r-internal panic (e.g., null
`enum_keys_map`, `is_BBL_printer` uninitialized), cross-reference
`docs/dev/libslic3r-workarounds.md`. New crashes are bugs.

### 4. Introspect example

```bash
cargo run -p slic3r-ffi --release --example introspect | head -4
```

**Expected.**

```
OrcaSlicer libslic3r_ffi v0
total options: 737
```

The option count is locked to the pinned OrcaSlicer submodule
(`external/OrcaSlicer @ 956fcea7e2`). When the submodule moves, the
new total is the new expected — update this line as part of the
submodule-bump commit.

### 5. Slice example reaches `Print::apply`

```bash
cargo run -p slic3r-ffi --release --example slice -- \
  external/OrcaSlicer/tests/data/test_stl/ASCII/20mmbox-LF.stl \
  /tmp/n3o-smoke.gcode
```

**Expected — Phase 0.** The example prints `slicing … -> …`,
libslic3r's apply-phase log lines stream to stderr, and the run exits
nonzero with:

```
slice failed: Validate: Relative extruder addressing requires
resetting the extruder position at each layer to prevent loss of
floating point accuracy. Add "G92 E0" to layer_gcode.
```

This is *the expected Phase 0 result.* libslic3r's
`Print::validate()` rejects the FullPrintConfig defaults because they
combine `use_relative_e_distances=1` with an empty `layer_gcode`. The
FFI link is healthy — we reach `Print::apply`, the engine reads our
config, and validation runs end-to-end. What's missing is a real
device-shaped configuration, which is **Phase 0.5 / Spike 1** work
(see `docs/dev/tickets/phase-0.md` "Phase 0.5 reminder"). The first
successful gcode-out is gated on the cascade adapter consuming a
converted OrcaSlicer device profile, not on patching the slice
example.

The output gcode is **not** validated as printer-safe by this smoke.
That is intentional — Phase 0 establishes the engine link only.
Multi-printer safety, AMS purges, etc. are Phase 5+ work.

### 6. App launches and shows the libslic3r version

```bash
npm install
npm run tauri dev
```

**Expected.**

- The app window opens within a few seconds.
- The header shows the libslic3r version string returned by
  `slicer_info()` ("OrcaSlicer libslic3r_ffi v0" or similar) and the
  total option count (737 for the pinned submodule).
- The "Search" box queries options live — typing `layer_height`
  returns matching rows from `slicer_options()`.
- The Slice form accepts the known-good STL above and surfaces the
  same `Print::validate()` failure as step 5 in the response. That
  is the expected Phase 0 result; gcode-out is Phase 0.5 / Spike 1.
- Backend logs (stderr in the terminal) carry `tracing` lines like
  `INFO n3o_slic3r_lib::core::cascade: slicer_info version=…
  options=737`, plus the `ERROR` line from `slicer_slice` on the
  failed validate.

### 7. Logging-format check

```bash
RUST_LOG=debug npm run tauri dev
# (in another terminal, click around)
```

**Expected.** The same events as above, now with `DEBUG` lines too,
timestamps, and span chains. Setting `LOG_FORMAT=json` swaps the
format to JSON Lines.

---

## Divergence checklist

Any divergence from the expected output above is **either**:

1. A bug — file it, fix it. Examples: tests segfault, slice produces
   empty gcode, app window fails to render, tracing emits no events.
2. An expected upstream change — update this document. Example: the
   option total moved because the OrcaSlicer submodule was bumped.

If you can't decide which, lean toward (1) and investigate. Quietly
updating the expected output to match new behavior is how silent
regressions ship.

---

## Out of scope for this smoke

- Printer hardware connectivity (Phase 7).
- Multi-plate / multi-printer projects (Phase 5).
- G-code preview rendering (Phase 6).
- Cascade resolver end-to-end (Phase 1; covered by Phase 0.5 spike 1
  before commitment).
- Settings UI (Phase 4).

These belong to their respective phases' exit smokes, not Phase 0's.
