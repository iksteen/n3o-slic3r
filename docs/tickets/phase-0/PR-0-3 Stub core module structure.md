# PR-0-3 — Stub the `core/` module structure per PRD §8.2

Status: ✅ done (commit `0cf16c0`).

**Scope.** Create the directory tree the PRD's architecture (§8.2)
calls for. Each module is an empty Rust submodule with a docstring
that names its responsibility and links to the PRD section that owns
its requirements. The point is to lock the module boundaries early
so subsequent phases can colocate work without bikeshedding the
layout.

**Acceptance criteria.**

`src-tauri/src/core/` contains:

```
core/mod.rs                    # umbrella module
core/cascade/mod.rs            # rule cascade resolver (PRD FR-CAS-1..13)
core/cascade_adapter/mod.rs    # logical → DynamicPrintConfig (FR-CAS-14..17)
core/project/mod.rs            # project model, plate-printer binding (FR-MP-*)
core/scene/mod.rs              # renderer-agnostic 3D scene state (FR-3D-7 / AD-8)
core/slice/mod.rs              # FFI orchestration, progress events (FR-SL-*)
core/gcode/mod.rs              # typed G-code model, parser, serializer (FR-GP-*)
core/threemf/mod.rs            # 3MF reader/writer
core/filament/mod.rs           # filament profile + sync (FR-FS-*)
core/plugin/mod.rs             # Lua host, hook dispatch (FR-PL-*)
core/printer/mod.rs            # driver-trait registry
core/printer/bambu/mod.rs      # Bambu MQTT (FR-BL-*)
core/printer/snapmaker/mod.rs  # Snapmaker HTTP (FR-SU-*)
```

- Each `mod.rs` has a `//!` docstring with one paragraph naming the
  responsibility plus a `//! See PRD §<n>` reference.
- `core` is declared in `src-tauri/src/lib.rs` as `pub mod core;`.
- The existing `slicer_*` Tauri commands move into
  `core/cascade/mod.rs` (option introspection is cascade territory)
  and `core/slice/mod.rs` (the slice command).
- `cargo check -p n3o-slic3r` is clean.
- `cargo build -p n3o-slic3r --bin n3o-slic3r` still produces a
  working binary.

**Effort.** ~1 day.

**Dependencies.** None. Best done before any new functionality
lands so the homes for it are pre-decided.

**Out of scope.** Implementing any of the modules. They stay empty
(beyond the docstring) until their phase. The `core::logging` module
that PR-0-2 adds is the one exception — it's a cross-cutting concern
implemented immediately rather than stubbed.

**Implementation note (post-delivery).** A `core/logging.rs` module
was added alongside the documented stubs — that's the home for the
PR-0-2 `tracing_subscriber` init, which is cross-cutting and exists
immediately rather than being stubbed.
