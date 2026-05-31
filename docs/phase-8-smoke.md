# Phase 8 — exit-criteria smoke

> Status: **the software chain is automated** (see the integration test
> below); the hardware proof point lives in
> `docs/phase-8-platecycler-smoke.md`.

A single repeatable gate chaining Phase 8's exit criteria
(`Execution_Plan.md` §10, adjusted in `docs/tickets/phase-8.md`). It
proves the plugin architecture works end-to-end: a Lua plugin is
discovered, enabled through the cascade, runs at a real slice and
changes the output, is suppressible per plate, and a broken plugin is
caught + recoverable without crashing the host.

## The chain

| # | Criterion | Where verified |
|---|-----------|----------------|
| 1 | An example plugin in the plugins folder is **discovered + loaded**, and is **off by default** (opt-in). | `phase_8_exit_smoke` |
| 2 | **Enabled** (global tier), it **runs at post-slice** on a real libslic3r slice and the output reflects it (verify-via-G-code). | `phase_8_exit_smoke` |
| 3 | A **per-plate `off` override suppresses** it even with global on. | `phase_8_exit_smoke` |
| 4 | A **broken plugin is caught + surfaced** (disabled + `last_error`) without crashing the host, and a **manual `plugin_reload` recovers** it once the file is fixed. | `plugin_reload_recovers_a_broken_plugin` |
| 5 | A plugin **error during a multi-plate job is isolated** — later plates still finish. | `erroring_plugin_does_not_break_a_multi_plate_job` |
| 6 | The **platecycler** plugin auto-ejects the finished plate on real A1 mini + PlateCycler hardware. | `docs/phase-8-platecycler-smoke.md` (hardware) |

Tests 1–5 live in `src-tauri/tests/plugin_post_slice.rs` and run under
`cargo test -p n3o-slic3r --test plugin_post_slice` (they drive real
libslic3r slices, so they need the FFI built).

## Running it

```
cargo test -p n3o-slic3r --test plugin_post_slice
```

The `phase_8_exit_smoke` test alone walks discovery → off-by-default →
enable → slice-reflects-it → per-plate-off, using the `beep-at-layer`
example (a deterministic `M300` at a layer boundary) on a real A1 mini
slice.

## Manual / UI spot-check (one-time per release)

The automated chain covers the engine path; a quick UI pass confirms the
three surfaces:
1. **n3o-slic3r brand menu → "Global plugins…"** — toggle a plugin on;
   confirm it persists to `~/.config/n3o-slic3r/config.toml`.
2. **Project menu → "Plugins…"** — set a plugin `off` for the project;
   confirm a plate inherits it.
3. **Settings panel → "Plugins" tab** — set a plugin `on` for one plate
   and edit a setting; slice and confirm the effect.

## Result

- **Software chain (1–5):** ✅ automated and passing (`plugin_post_slice`).
- **UI spot-check:** ✅ confirmed by the project lead (2026-05-31).
- **Hardware (6):** ✅ confirmed by the project lead — platecycler
  auto-ejects on the A1 mini + PlateCycler (see
  `docs/phase-8-platecycler-smoke.md`).
