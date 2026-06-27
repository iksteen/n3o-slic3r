# Architectural Review — Action Plan

Derived from the full multi-agent architectural review (14 subsystems, 100
verified findings: 0 critical, 2 high, 13 medium, 85 low). The codebase is
architecturally healthy; this plan is **record integrity + a pre-release
cleanup sweep + one real bug**, not a redesign.

**Sequencing rationale.** Bug first (isolated, user-facing). Then the doc
truth-up (zero code risk, fixes the HIGH "docs mislead future sessions"
theme). Then deletions (don't refactor code you're about to delete; a few
are cheaper *before* the `.n3o` format ships). Then correctness/robustness.
Mechanical refactors last (highest file churn, lowest risk).

Duplicate flags from multiple reviewers are merged into one item. Each item:
`ID — title` · files · action · verify. Check boxes as you go.

---

## Decision points — DECIDED

- **DP-1 (cascade override model) — DECIDED: wire the tier path in.**
  The `profiles.md` two-phase tier design (`overrides.rs`/`trace.rs`/
  `validate.rs`/`load_cascade`/`adapt_with_overrides`) becomes the **live**
  resolution path; the composer's ad-hoc `important`-rule override approach is
  removed; the trace ("why is X=55") UX gets a Tauri command; `profiles.md`
  stays the enforced design of record. This is now **C-1**, the central
  cascade task — the earlier "delete the dead modules" option is dropped, so
  the X-1 slot is retired (those modules are kept and wired in, not deleted).

- **DP-2 (`n3o-send` binary) — DECIDED: delete.** Remove the bin + the
  `default-run` workaround (X-9).

- **DP-3 (camera + Snapmaker control-plane scope) — DECIDED: declare
  shipped/in-scope.** Update AD-7, the capability matrix, FR-BL-5, §9.3 to
  say camera + the U1 control plane are in-scope; propagate to CLAUDE.md and
  the two stale in-code comments (D-3). Pure framing edit — the code exists.

---

## Phase 0 — The one real bug  ✅ DONE (cargo test --workspace: 951 passed)

- [x] **B-0 — Bambu reconnect backoff is not cancellable; teardown drops the task instead of aborting**
  `src-tauri/src/core/driver/bambu/connection.rs:712,730` (sleeps), `:208-232` (disconnect)
  Wrap both backoff sleeps in `select! { _ = &mut shutdown_rx => break, _ = sleep(..) => {} }` (mirror `camera.rs:281`); in `disconnect()` call `handle.abort()` when the 5s join times out (mirror `snapmaker/connection.rs:278`).
  *Verify:* trigger a settings-change `replaceDriver` against an offline printer; old socket should release immediately, not after ~60s.

---

## Phase 1 — Documentation truth-up (HIGH; zero code risk)  ✅ DONE (tsc + 275 frontend tests pass; 0 PR-refs / 0 Three.js residual)

Pure doc/comment edits. Fixes the dominant defect class (CLAUDE.md says
"the document wins," so stale docs actively mislead). Do these as one batch.

- [x] **D-1 — G-code preview is wgpu, not Three.js** *(HIGH)*
  `CLAUDE.md:75-76`; `docs/dev/PRD.md` §8.1(:414), FR-3D-7(:245), AD-8(:564), §10(:616); `docs/dev/wgpu-renderer.md:17-18,31,34-36,91-95,176-177,207-227`; `docs/dev/Execution_Plan.md:167,670`
  Rewrite to present tense: both the prepare viewport and the G-code preview are wgpu (`toolpath_render.rs` + `GcodePreview.tsx`, Strategy A). In `wgpu-renderer.md`, fold the "preview stays Three.js / out of project" framing into a "done" note — don't ledger the corpse. Confirm no `three` remains in `package.json`/`src/`.

- [x] **D-2 — Native save is `.n3o`, 3MF is import-only** *(HIGH)*
  `docs/dev/PRD.md` FR-MP-4(:197), §8.1 Storage(:420), §12(:673); `docs/dev/Execution_Plan.md:319,539`; `CLAUDE.md` (".3mf/n3o_project.json" → ".n3o native, .3mf import-only"); project module docs `project/mod.rs:5-6`, `model.rs:23`, `mutation.rs:35`, `commands.rs:393`, `threemf/mod.rs:12-14`, `context.rs:9-11`
  Rewrite FR-MP-4: native = `.n3o` (zip of `project.json` + `geometry/<MeshId>.bin`); 3MF = import/migration; per-driver send format separate. (Pairs with X-2's format change — keep these together.)

- [x] **D-3 — Camera + Snapmaker control plane are shipped, not out-of-scope** *(see DP-3)*
  `docs/dev/PRD.md` AD-7(:552-558), capability matrix Camera row(:501), FR-BL-5(:301), §9.3(:592); add a domain-fact line to `CLAUDE.md`; fix the two self-contradicting comments `printer/snapmaker/mod.rs:17` and `driver/camera.rs:31`.

- [x] **D-4 — PRD §8.2 module map is wrong/incomplete**
  `docs/dev/PRD.md:444-446`
  Add a `core/driver/<vendor>` comms-layer entry; fix the `core/printer/bambu` entry to mirror snapmaker; add `core/preview`, `core/profile_library`, `core/schema`, `core/orca_import`. (The code-less `core/printer/bambu/mod.rs` stub is handled in X-3.)

- [x] **D-5 — AD-8 doc inconsistencies**
  `CLAUDE.md` AD-8 + `docs/dev/PRD.md:562` vs `:582`/code: drop "gizmo state, camera state" from the Rust-owned enumeration (renderer-local per §9.2); correct "set_object_transform is the only object-transform mutation" to the real invariant (the *renderer* drives transforms only via commands; the scene layer has several transform ops — see R-12).

- [x] **D-6 — wgpu-renderer.md: Strategy A is the shipped cross-platform path; picker is a linear scan**
  `docs/dev/wgpu-renderer.md:91-95,207-208,217-220` (reframe macOS/Windows present shims as optional zero-copy optimizations, not prerequisites — `viewport_render.rs` has no `cfg(target_os)`); §5 step 3 / §3 "Rust BVH" → "CPU ray/triangle scan" (`viewport_render.rs:833-856` is linear Möller-Trumbore).

- [x] **D-7 — libslic3r-workarounds.md stale pin + line refs**
  `docs/dev/libslic3r-workarounds.md:3,37,70,102,137,233`
  Refresh the pin to the current submodule; replace brittle absolute line numbers with **symbol-name anchors** (the shim is small and greppable). Confirm §1–9 still match `slic3r_ffi.cpp`.

- [x] **D-8 — Scene comments describe removed IPC / nonexistent type**
  `core/scene/state.rs:62-71,87-89,125-128`, `commands.rs:56,117`, `events.rs:3`
  Rewrite to: geometry uploaded GPU-side keyed by `MeshId`; `MeshHeader` is the only mesh data on the JSON wire. Replace `SceneState` with `Project`/`PlateSceneState`.

- [x] **D-9 — Frontend "stub" naming + future-tense narrative for shipped code**
  `src/settings/SettingsPanel.tsx:1-20,88-125`, `src/settings/overrideCommands.ts:1-19`, `src/state/queryCache.ts:14-20`
  Rename `SelectedObjectStub`→`SelectedObject`, `PlateObjectStub`→`PlateObject`; rewrite headers to present tense; delete the "PR-4-9 wires the real backend" narrative; fix the queryCache header (selectors are implemented; only ref-counted GC is deferred). (Backend send-path equivalents in D-12.)

- [x] **D-10 — Plugin doc drift**
  `core/plugin/mod.rs:3-9`/`host.rs` ("os: only `os.time`/`os.clock`", reconcile with CLAUDE.md), `manifest.rs:228` (add `printer-instance` to the `UnknownScope` message), `resources/plugins/platecycler/main.lua:13` (move `SWAP_GCODE` to a declared setting or drop the "once wired" note).

- [x] **D-11 — Slice/orchestrator docs overstate what's implemented**
  `core/slice/orchestrator.rs:21-23` (mid-plate cancel is plate-boundary-only — say so), `:191-201`/`job.rs` JobRegistry "worker-called cleanup" that isn't wired (see C-12), `core/gcode/model.rs:368-397` (`LayerSource::Heuristic` doc — drop the variant in X-7 and the docstring here).

- [x] **D-12 — Misc backend stale module docs / comments**
  `core/printer/mod.rs:1-8` (header says "Printer drivers" — describe the instance/profile subsystem), `core/driver/commands.rs:350-356,509-521,369` (drop "PR-7c-7 will embed" send-path comments), `core/project/model.rs:156` (prune the `plate_printer_identities` portability-hedge comment that the format doesn't carry).

- [x] **D-13 — PR/ticket reference sweep** *(project's own "no PR refs in code" rule)*
  ~331 occurrences across ~53 files (`src-tauri/src` ~163, `src/` ~168). Drop the `PR-N`/ticket tokens; rewrite any comment describing now-shipped "future" work to present tense or delete it. Keep `FR-*` / `PRD §` spec refs (explicitly allowed). Mechanical find-and-prune.

---

## Phase 2 — Delete dead & speculative code (release cleanup)

The "squeaky-clean, remove-don't-keep-dormant" sweep. Do before refactors.

> **Status (2a done, verified):** X-4/5/6/8/9/10 done as planned; X-7 trimmed to
> the *truly* dead items only (a read-only scope pass found `dispatch`,
> `set_enabled`, `PayloadKind::Gcode3mf`, `Severity`, `current_stage`, the
> `serial` field, and `SendPayload` serde all live or test-guarded — kept); X-12
> done via a non-default **`test-fixtures` cargo feature** (a plain `cfg(test)`
> gate breaks integration tests, which link the lib without `cfg(test)`) so
> release builds never compile the fixtures or the fake-printer fallback.
> Verified: release path compiles fixture-free; `--features test-fixtures` →
> 946 passed; plain `cargo test` → 912 passed (gated files skip, 0 fail); tsc +
> 275 vitest green.
>
> **2b done, verified:** X-2 dropped `Mesh.normals` everywhere + from the `.n3o`
> geometry blob; per the owner's reversal, **no legacy read-tolerance** — the
> format bumps to `"2"` and `"1"` files fail with a clean version mismatch
> (not silent mis-deserialize). X-3 computes `bucket` in the FFI's
> `DefCache::build` from `Preset::{print,filament,printer}_options()` and deletes
> the 721-line scraped table + the Python scraper; a full all-keys parity diff
> vs the old table matched 666/670 and **corrected 4** keys the old scraper
> wrongly included (its regex matched string literals inside `/* */` comments —
> 3 phantom non-options + `filament_colour`, which n3o sets via the slot UI).
> The bucket correction surfaced that the adapter used `bucket==Filament` as a
> proxy for "per-filament vector"; restored via a documented `is_per_filament_vector`
> predicate (the lone `filament_colour` exception is irreducible — libslic3r feeds
> it from the model, not any preset, so no FFI signal captures it). Verified:
> release compiles; `--features test-fixtures` → 943 passed (incl. FFI bucket
> parity test); tsc + 275 vitest green. **Phase 2 complete** (X-11: deleted the hand-curated DROP_LIST + Manifest — the Dropped/UnknownKey events it gated were unconsumed by the slice path; schema-misses now uniformly skip + debug-log).

- [ ] **X-1 — _retired_ (DP-1 chose wire-in, not delete).** The two-phase cascade override modules are kept and made live — see **C-1** in Phase 3.

- [x] **X-2 — `Mesh.normals` dead data (compute/store/persist/clone, zero consumers)** *(MEDIUM; cheaper before the format ships)*
  `core/scene/state.rs:92-94`, `loaders/mod.rs:91 compute_vertex_normals`, `core/project/format.rs:202,278`, `core/slice/input.rs:431`
  Delete `Mesh.normals`/`NewMesh.normals`/`compute_vertex_normals`; drop the normals chunk from the format geometry blob (schema-version guard already exists). Renderer/libslic3r recompute their own. (Pairs with D-2.)
  *Verify:* load + re-save a `.n3o`, slice both printers, confirm geometry + G-code unchanged; files smaller, geometry RAM ~halved.

- [x] **X-3 — `option_buckets.rs` scraper → compute C++-side like `scope`** *(MEDIUM; ~900 lines)*
  `crates/slic3r-ffi/src/option_buckets.rs` (721 lines), `scripts/scrape_option_buckets.py`, `lib.rs:532`, `ffi/slic3r_ffi.cpp:268-275` (scope pattern)
  Add a `bucket` field to `slic3r_option_def_t`; populate it in `DefCache::build` by masking each key against `Preset::print_options()/filament_options()/printer_options()` (union nozzle+machine_limits into Printer). Map it in `OptionDef::from_raw`. Delete the scraper + table + the pin-bump checklist's bucket step + the empty `core/printer/bambu/mod.rs` placeholder (or reduce to a pointer).
  *Verify:* compare `bucket` for all keys against the old table for both printers — zero diff; re-run the option scrapers note in `project_orca_pin_bump_checklist`.

- [x] **X-4 — Logical-key dimensional-expansion machinery (unused)** *(independent of DP-1)*
  `core/schema/mod.rs:76 DimensionalKind,92 LOGICAL_KEYS,173 dimensional_for_key`; `cascade_adapter/adapter.rs:141-172` bed_temp branch. No profile emits a logical `bed_temp`. This is a *separate* `profiles.md` feature from the override tiers C-1 wires in — decide on its own: delete it, or implement+keep if logical dimensional keys are wanted. Lazy default: delete until a profile needs it.

- [x] **X-5 — Speculative empty placeholders: `IncompatibleSetting`/`ClampedSetting`**
  `core/scene/events.rs:230-265`, always-empty `PrinterChangeReport.incompatible/.clamped` at `core/project/mutation.rs:663-664` (tests assert empty `:3622-3623`). Frontend discards the report anyway. Drop the two structs + two fields (re-add when a producer exists).

- [x] **X-6 — Dead orchestrator entry points**
  `core/slice/orchestrator.rs:164-190` (`start_slice_job`, `plugin_host_from_app`, `app_handle_sink`), `:278-284` (`start_slice_job_with_sink`), `core/slice/mod.rs:28` (`start_slice_job_internal` re-export). Keep `start_slice_job_with_sink_and_plugins` (prod) + `run_slice_job_blocking*` (tests).

- [x] **X-7 — Misc dead types/variants/API**
  `core/project/model.rs:487-505 ProjectMutError` (+ `mod.rs:36` re-export; dupes `SceneOpError`) · plugin host `host.rs:147 any_hook, :231 dispatch, DispatchGate::all, :341/:428 set_enabled` (rewrite affected tests onto `dispatch_gated`) · `core/plugin/hooks.rs:149-165 PayloadKind::Gcode3mf` unreachable arm · `crates/slic3r-ffi/src/lib.rs:788-803 Severity` single-variant → `diagnostics: Vec<String>` · `core/gcode/model.rs LayerSource::Heuristic` (with D-11) · dead U1 status fields `core/driver/status.rs:204 current_stage`, `snapmaker/connection.rs:50-52,83-86 serial`/`serial()` · `driver/bambu/connection.rs:66,113-123,185 raw_messages_rx/raw_messages()/device_id()` · trait Phase-8 serde derives `driver/traits.rs:5-7` (`SendPayload`, `DriverError` Serialize/Deserialize/Clone — unless C-3 adopts typed errors).

- [x] **X-8 — Stale `#[allow(dead_code)]` masking live code**
  `core/driver/snapmaker/http.rs:44,104`, `moonraker.rs:231,242,249,257`, ~16 total in `src-tauri/src`. Delete every suppression whose item is now referenced; for the genuinely-unused few, remove the item. *Verify:* `cargo build`, treat each resulting `dead_code` warning as a delete candidate.

- [x] **X-9 — Spike examples + `n3o-send`** *(n3o-send: see DP-2)*
  Delete `src-tauri/examples/spike1.rs,spike2.rs,spike3.rs` + their `[[example]]` entries; fold `phase1_smoke.rs` into `tests/` or drop. Keep `slice_repro`/`slice_to_gcode_3mf` as living repro tools. Per DP-2, delete `src-tauri/src/bin/n3o-send.rs` (632 lines) + the `default-run` line, or install+document it.

- [x] **X-10 — `RequiresChamberHeater` stub** *(`KnownDimensions` now stays — `validate.rs` is live per C-1)*
  `core/schema/capability.rs:75,105,159`. Drop `RequiresChamberHeater` until `PrinterProfile` carries the field. `core/cascade/validate.rs:64 KnownDimensions` is no longer a deletion candidate (validate.rs becomes live in C-1) — collapse it to the canonical dimension set if it's still loose, else leave. *(Reviewer tagged ACCEPT as an anticipated extension point — fine to defer.)*

- [x] **X-11 — Hand-curated `DROP_LIST`** *(low priority)*
  `core/cascade_adapter/manifest.rs:29-133`. Either treat all schema-misses uniformly (log fork-only noise at debug) or generate the list rather than hand-maintaining 80 entries. Defer if it earns its keep.

- [x] **X-12 — Gate test fixtures out of the production binary**
  `core/printer/instance_library.rs` (not `cfg(test)`), fallback `instance_registry.rs:41-50`, `printer/mod.rs:24-26`. Gate `bundled_instances()` behind `#[cfg(test)]`; the no-root production path returns empty. Removes the silent fake-printer fallback from release.

---

## Phase 3 — Correctness & robustness fixes

- [ ] **C-1 — Wire the two-phase override-tier path in as the live resolver** *(DP-1; the central cascade task)*
  Live modules to activate: `core/cascade/{overrides,trace,validate}.rs`, `loader.rs:90 load_cascade`, `cascade_adapter/adapter.rs:190 adapt_with_overrides`. Replaced: the composer's ad-hoc `important`-rule override synthesis + source attribution (`composer.rs:204-421` override portions) and `slice/orchestrator.rs:140`'s project-only fold (`input.rs:24,257,316`).
  Steps: (1) route the slice + panel resolution through `adapt_with_overrides` with `OverrideTiers` built from user/project/object; (2) this closes the user-tier→engine gap for free (the tier model already models precedence correctly); (3) add a Tauri command exposing `trace.rs` per-key source attribution for the "why is X=55" UX (`profiles.md` design of record); (4) remove the superseded composer override path once parity holds.
  *Verify:* **parity guard first** — assert the new resolver produces byte-identical `DynamicPrintConfig` to the composer output across `tests/reference_profiles.rs` for both printers *before* deleting the old path; then a test that a user-tier engine key reaches the resolved config, and that the trace returns the winning layer/source per key.

- [ ] **C-2 — `build.rs` only warns when a submodule patch fails to apply**
  `crates/slic3r-ffi/build.rs:323-361`. Make a failed apply a hard panic (the idempotent reverse-check already distinguishes "already applied" from "failed"), so the wave-overhang carry can't silently drop and ship a broken binary.

- [ ] **C-3 — Typed errors for the few UI-branching commands**
  `core/slice/orchestrator.rs:93-108` (`SliceStartError::SliceBlocked` collapses its issue list to a count in `Display`), `core/slice/commands.rs`, driver `commands.rs:317,605-607`, `src/driver/SendControls.tsx:158` (`/cancel/i.test(msg)` regex)
  For slice-start, instance-mutation, connection-validation, and driver send/command: return the already-`Serialize`-derived typed error so the frontend branches on `error.kind` (esp. `Cancelled`/`SliceBlocked` issue list). Leave the string path for the rest; this also retires the X-7 serde-derive deletion's counterpart. *(If you keep strings, at minimum give `Cancelled` a stable sentinel.)*

- [ ] **C-4 — FFI callback trampolines have no panic guard**
  `crates/slic3r-ffi/src/lib.rs:725-747,980-999`. Wrap each closure in `catch_unwind`; a panicking progress/log callback should drop a tick, not unwind across the C ABI.

- [ ] **C-5 — `slic3r_slice` mutates the caller's `Model` through a `&Model`**
  `ffi/slic3r_ffi.cpp:780-810` (writes `obj->config`) vs `:659` (cfg is copied); `src/lib.rs:885-896`. Apply the same temp-copy discipline to the per-object selectors, or change the signature to `&mut Model` and document it. At minimum, comment the deliberate write-through.

- [x] **C-6 — Non-deterministic default process for a new instance**
  `core/profile_library/mod.rs:629-635` reads `HashMap` keys; consumed at `instance_registry.rs:610-624`. Read from the existing `process_order` `BTreeMap` (`mod.rs:160`) instead. One-line, deterministic source already present.

- [x] **C-7 — Plugin declared `scopes` not enforced at resolve time**
  `core/plugin/manifest.rs:54-61,175 allows_scope` (dead), `host.rs:298-315`, `resolve.rs:86-123`. Either enforce (zero a tier's `PluginLevel` when `!allows_scope(tier)` in `resolve_plugin` — then `allows_scope` earns its keep) or delete `allows_scope` and rewrite the `PluginScope` doc to "advisory/UI-only". Don't leave a documented backend invariant unimplemented. **Recommend enforce** (small, matches the doc).

- [x] **C-8 — Plugin instruction budget doesn't bound time in C/stdlib calls**
  `core/plugin/runtime.rs:52,60-75`. Add a wall-clock deadline checked in the instruction hook, or document that the budget bounds Lua instructions only (memory limit bounds the rest). Either makes the "can't hang the app" guarantee honest.

- [x] **C-9 — `project_save` holds the Project mutex across the full zip-to-disk**
  `core/project/commands.rs:402-431`, `format.rs:112-141`. Mirror autosave: clone the skeleton under the lock (wrap mesh blobs in `Arc` so only the skeleton clones), drop the lock, then write off-lock.

- [ ] **C-10 — Temp `.3mf` serialized + written under the global Project mutex**
  `core/slice/commands.rs:79-104`, `core/slice/input.rs:278-284`. Extract geometry+context under the lock, drop it, then serialize+write off-lock.

- [x] **C-11 — Autosave deep-clones all geometry every tick before the change check**
  `core/project/autosave.rs:213-236`. Hash the skeleton (`serde_json::to_vec(&*p)` under the lock is cheap — serde skips mesh buffers), compare, clone only on change.

- [ ] **C-12 — `JobRegistry` never pruned (memory grows per slice)**
  `core/slice/job.rs:191-201`, `orchestrator.rs:520-784 run_worker`. Pass an `Arc<JobRegistry>` into `run_worker` and `remove(job_id)` after the terminal event (short grace period so `slice_status` reconnect still works). (Pairs with D-11.)

- [ ] **C-13 — Driver `&mut self` over-serializes uploads vs control/status**
  `core/driver/traits.rs:189-208,172`, `bambu/connection.rs:242-276`, `registry.rs:30-38,99-102`. Change `send`/`command`/`set_ams_filament`/`status` to `&self` (impls already clone what they touch); narrow the registry `AsyncMutex` to `connect`/`disconnect` (or store the handle in an `Arc`). Then a long upload no longer blocks `status()`.

- [ ] **C-14 — `DriverId` ↔ `instance_id` correspondence lives only in the frontend**
  `core/driver/commands.rs:301,527`, `camera.rs:301`, `snapmaker/commands.rs:41`, `printer/mod.rs:489`. Store the owning `instance_id` on the `DriverRegistry` entry (created from a per-instance `ConnectionInfo`); sync/camera/AMS commands then take one id and the backend owns the mapping.

- [ ] **C-15 — Filament lookup rebuilds the full 201-fragment summary per slot at compose time**
  `core/filament/registry.rs:31-36`, `library.rs:97-101`, driven from `composer.rs:657-676`. Add a keyed single-fragment summary lookup (`filament_fragments.get(slug)`); route `lookup`/`is_bundled` through it; leave `list_*` for genuine enumerate-all callers.

- [ ] **C-16 — Store extrusion ΔE directly instead of discard-then-reconstruct**
  `core/preview/build.rs:38-40,177-179` (forward), `stats.rs:245-256` + `commands.rs:238-244` (inverse). Store `extrusion_mm` on `Segment`/`SegmentSet`; delete both inverse computations and 2 of the 3 duplicated cross-section constants.

---

## Phase 4 — Mechanical refactors & relocations (last)

Highest file churn, lowest risk, no behavior change. Do after deletions so
you don't relocate doomed code.

- [ ] **R-1 — Convert 5 hand-rolled error enums to `thiserror`** *(already a dependency, used 21×)*
  `printer/instance_registry.rs:160-237`, `instance_storage.rs:60-81`, `filament/library.rs:47-55`, `profile_library/composer.rs:124-150`, `profile_library/mod.rs:81-105`. Message strings already exist verbatim. ~150 lines gone, no behavior change.

- [ ] **R-2 — Move settings-UI option surfacing out of `core/cascade`**
  `core/cascade/mod.rs:210-595` (the four `slicer_*_options` commands + `OptionSummary`/visibility helpers) → `core/schema` (e.g. `schema/options_ui.rs`), folding onto the cached `OptionSchema` so there's one introspection path (stop re-fetching `option_defs()` per command). Leaves `cascade/mod.rs` as just the resolver. *(Merges the "two parallel option-universe reps" finding.)*

- [ ] **R-3 — Move `AmsMappingV2` + `ams_*_for_plate` to the driver layer** *(MEDIUM; inverted dependency)*
  `core/slice/pre_slice_gate.rs:165-289` → `core/driver` (`traits.rs` already re-exports the type — the tell). Leave only binding-coherence validation in the slice gate. `driver/bambu/connection.rs:601` and `pre_slice_gate` import from the new home.

- [ ] **R-4 — Move `schema::capability` out of `schema`**
  `core/schema/capability.rs:32`, `mod.rs:13-14`. Relocate to `cascade` (its consumer) or `printer` (the type it needs) so `schema` is a true FFI leaf; fix the stale "printer::profile cross-references schema keys" line.

- [ ] **R-5 — Move read-side cascade/tower logic out of the command-wiring file**
  `core/project/commands.rs:90-368` (`layer_for_source`, `resolve_plate_cascade`, `resolve_plate`, `resolve_instance_cascade`, `tower_geometry_for_plate`) → a `core/project` domain module; leave `commands.rs` as thin Tauri wiring.

- [ ] **R-6 — Extract a driver send-orchestration module**
  `core/driver/commands.rs:359-397,421-474,631-716,737-845` (`wrap_gcode_as_3mf`, `derive_send_names`, `collect_ams_*`, `plate_printer_model`, `apply_pre_send`, AMS-write resolution) → `core/driver/send.rs` (+ a small `ams` module). `#[tauri::command]` fns become thin adapters.

- [ ] **R-7 — Split `project/mutation.rs` (4148 LOC) by topic**
  One `impl Project` across `mutation/{geometry,materials,plates,overrides}.rs` (Rust allows a split impl). Mechanical, no API change. **Do not move logic off `Project`** (the single impl is borrow-checker-forced and documented). *(Merges the scene/project "fractured subsystem" finding — also move `scene/commands.rs` into `project` since it operates only on `Project`, removing the `scene→project` cycle leg.)*

- [ ] **R-8 — Split the three ~1000-LOC frontend components + de-god App.tsx**
  `src/printer/PrinterSettingsModal.tsx` (1418), `src/settings/SettingsPanel.tsx` (958), `src/driver/DevicesView.tsx` (947): move already-named subcomponents + pure helpers into sibling files (seams already drawn; helpers shed test-only exports). `src/App.tsx`: extract `useViewportTools()` (owns the gizmo/tool/clone/faceMatch mutual-exclusion invariant), `useProjectFileMenu()`, and a `project/importReport.ts`.

- [ ] **R-9 — FFI shim tidy-ups**
  `ffi/slic3r_ffi.cpp:646-810` extract `normalize_filament_map`/`resolve_region_filaments`/`pin_bbl_quirks` from the 330-line `slic3r_slice` (each keeps its workaround comment) · `:696,865` compute `is_bbl` once · `lib.rs:527,545` `std::mem::zeroed()` → `::default()` (bindgen derives it; removes 2 unsafe blocks) · `:376-378 slic3r_version()` embed the OrcaSlicer SHA via a cmake `-D` define.

- [ ] **R-10 — Consolidate duplicated helpers**
  cstyle (un)escape ported twice (`cascade/mod.rs:156` byte-based, `composer.rs:806,857` char-based) → one shared util (or expose the C++ original over FFI) · structured-comment recognition duplicated across `gcode/header.rs:162-295`, `parser.rs:448-522`, `slice/summary.rs:107-215` → one recognizer · `merge_status` inline-duplicated in two test modules (`snapmaker/moonraker.rs`, `status.rs:697-711`) → expose `pub(super)` and call the real impl.

- [ ] **R-11 — Shared wgpu device + extract the pure gizmo solver**
  `viewport_render.rs:1109-1120` and `toolpath_render.rs:237-248` each build their own `Instance`/adapter/device → one lazy `(device, queue)` in `viewport_gpu.rs`. Lift the pure solver (`ray_plane`/`compute_pre`/`pick_gizmo`/`selection_*` + `gizmo_tests`, `viewport_render.rs:699-1071`) into `viewport_gizmo.rs`. Don't split the GPU/pipeline code — it reads fine as one unit.

- [ ] **R-12 — Shared "seat + clamp + emit" transform helper**
  `core/project/mutation.rs:1365,1382,1431,1441,1453` (direct `obj.transform` writes) — factor the duplicated seat/clamp/emit tail into one private helper so the bounds policy lives in one place. (Pairs with D-5.)

- [ ] **R-13 — `emit_instance_changed` helper**
  `core/printer/mod.rs` (~10 `printer_instance_*` commands repeat emit boilerplate) → one helper, mirroring `filament/mod.rs::emit_changed`.

- [ ] **R-14 — `slice_active_plate` burns a throwaway `JobId` to name the output dir**
  `core/slice/commands.rs:93-101`. Reserve the id once and thread it through, or name the dir with a timestamp/uuid so the registry isn't double-incremented.

- [ ] **R-15 — Break the clearest module cycle**
  `core/profile_library/composer.rs`: it needs both `PrinterInstance` and profile fragments, so move it to a layer *above* both, or take `PrinterInstance` data as a plain input rather than importing the type — makes `profile_library` a true leaf. Treat the other cycles as documented debt unless one blocks a test.

---

## Accepted tradeoffs — no action (documented, deliberate)

Verified as intentional; listed so "addresses all" is honest. Revisit only
if the named condition changes.

- **Two global-state conventions** (Tauri `.manage()` vs `OnceLock` singletons) — singletons are needed by non-Tauri test/worker contexts. If touched, document the rule in CLAUDE.md.
- **libslic3r vector-quirk assembly in the composer** (`composer.rs:204-421`) — inverts "adapter owns the vocabulary"; at minimum add a doc note (folded into D-2/R-2 scope if you relocate it).
- **Bambu pre-spawn connect probe / initial-connect give-up** (`connection.rs:143-206`) — works; moving the probe inside the retry task is the cleaner unify if initial-unreachable retry is wanted.
- **macOS disables Bambu device-cert verification** (`bambu/tls.rs:78-79`) — documented LAN trust-model constraint.
- **Binding-coherence gate runs in the command, not the orchestrator** (`slice/commands.rs:90`) — fine unless a non-command slice path appears; then move the gate (or a pre-validated token) into the orchestrator.
- **Plugin host Mutex held across the multi-plugin fold** (`host.rs:239-292`) — fine at MVP plugin count; finer-grained locks if heavy plugins ship.
- **Registry mutex held across the disk write** (`instance_registry.rs:254-268`) — fine unless sync drives high-frequency mutations.
- **Connection field-validation duplicated FE/BE** — acceptable for a Tauri app; share a constant only if it drifts.
- **Vendored `suppaftp`** (33 files for a one-method cfg patch) — works; a git-fork `[patch]` source or upstreaming the cfg-split is tidier, low priority.
