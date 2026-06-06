# PR-8-5 — post-slice hook wired into the slice pipeline

Status: ✅ done.

**Implementation notes.**
- The `Hook` trait went **instance-based** (`&self`) so a hook can
  carry per-dispatch context — `PostSliceHook` holds the `PlateMeta`
  it hands each plugin as the second Lua arg.
- **Clean-by-copy isolation:** each plugin gets a fresh `GcodeHandle`
  over a clone of the current lines; edits are adopted only on success,
  so a plugin that errors part-way leaves the prior lines untouched.
- **No-op passthrough** is enforced by comparing re-serialized bytes to
  the original and skipping the write when equal — output stays
  byte-identical to libslic3r's when nothing mutates. `any_hook`
  short-circuits before any parse when no enabled plugin declares the
  hook.
- The host is threaded as `Option<PluginHostRef>` through new
  `*_and_plugins` / `*_with_plugins` orchestrator entry points;
  `start_slice_job` pulls it off the app state via `try_state`, so the
  existing host-less test entry points (and their ~8 callers) are
  untouched.
- `plate.object_count` is `nil` — the orchestrator slices a model file
  and doesn't count objects (surfaced as unknown rather than a wrong 0).
- **Example plugins live in `examples/plugins/`, not the auto-loaded
  bundled `plugins/` dir** — a beep/pause plugin enabled by default
  would fire on every slice. They're reference + test fixtures; the
  opt-in story for shipping enabled bundled plugins is a later concern
  (PR-8-9).
- **Mid-dispatch auto-disable does not emit `plugin:changed` yet.** The
  host state is updated (the panel will see it on next `plugin_list`),
  but live event emission needs the `AppHandle` in the worker and there
  is no panel to refresh until PR-8-9 — deferred to there.

Verified via G-code: an integration test slices a real cube with the
example plugins active and greps the output for the injected `M300` /
`M0`, plus a no-plugin baseline that lacks them.

**Scope.** The phase's **first end-to-end vertical slice**: wire the
`PluginHost`'s post-slice dispatch into the slice orchestrator so a
real `.lua` plugin, discovered from the plugins folder, modifies the
real libslic3r-emitted G-code through the typed bindings — verified by
re-slicing and grepping the output. Ships two tiny example plugins to
prove the path.

Owns **FR-PL-3** (post-slice hook).

**Acceptance criteria.**

- Hook point in `core/slice/orchestrator.rs`: after a plate's
  `slice(...)` writes its G-code and before `build_summary` /
  `PlateFinished`, the orchestrator:
  1. parses the output with `gcode::parse_lines`,
  2. builds the `Gcode` userdata (PR-8-4) + a read-only `plate`
     metadata table (`plate_id`, `printer_model`, `bed_type`,
     `object_count`),
  3. dispatches `on_post_slice(gcode, plate)` through the host
     (PR-8-3 fold), then
  4. re-serializes with `gcode::to_string` and writes the result back
     to the same output path **only if a plugin actually mutated it**
     (track a dirty flag so the no-plugin / no-op path stays a
     byte-identical passthrough and costs only a parse).
  - The post-slice transform runs before the preview pipeline loads the
    file, so preview reflects the plugin's output. Summary (time /
    filament / layers) is recomputed from the post-hook G-code.

- The host is threaded into the orchestrator (the `Arc<Mutex<PluginHost>>`
  state). When no plugins declare post-slice, the parse+reserialize is
  skipped entirely — zero overhead for the common case.

- Failure semantics inherit PR-8-3: a plugin error is isolated, the
  plate's G-code is left as libslic3r produced it, the slice
  **succeeds**, and the error surfaces via the plugin event/`last_error`
  (a plugin must never fail a slice).

- Two bundled example plugins (under a repo `examples/plugins/`
  directory that the dev build points the plugins root at):
  - **beep-at-layer** — inserts an `M300` beep at a configured layer
    number (declares a `layer` setting; for now read a hardcoded
    default until PR-8-9 wires plugin settings).
  - **pause-at-layer** — inserts an `M601` / `M0` pause at a configured
    layer.
  Each is a real `plugin.toml` + `main.lua` exercising `g:layers()` +
  `g:insert`/`g:append`.

- Verification (per the verify-via-G-code practice — green unit tests
  don't prove libslic3r accepted anything): an integration test slices
  a real model with `beep-at-layer` active and **greps the output
  G-code for the injected `M300`** at the expected layer; another
  confirms the no-plugin slice is byte-identical to a baseline.

- Tests:
  - Orchestrator post-slice dispatch fires for a plate; mutation is
    written back; dirty-flag passthrough verified (no plugins → output
    bytes unchanged, asserted).
  - The two example plugins load + run end-to-end against a sliced
    fixture.
  - A deliberately-erroring post-slice plugin does not fail the slice.

**Effort.** ~2 days. Mostly the orchestrator wiring + the verify-via-
G-code smoke; the dispatch and bindings already exist.

**Dependencies.** PR-8-3 (host + dispatch), PR-8-4 (G-code bindings),
`core/slice/orchestrator` (existing).

**Out of scope.**

- pre-slice / pre-send hooks → PR-8-6.
- The platecycler plugin (also post-slice, but its own ticket +
  hardware smoke) → PR-8-7.
- Reading the layer number from a plugin-declared setting through the
  cascade → PR-8-9 (examples hardcode a default until then).
- A real plugins-folder location via Tauri paths beyond pointing the
  dev build at `examples/plugins/`.
