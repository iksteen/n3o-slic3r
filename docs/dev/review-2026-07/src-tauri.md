# Review — `src-tauri` (Rust IPC backend)

Fresh architectural + over-engineering ("ponytail") review, 2026-07, ahead of
public release. Eight subsystem reviewers (IPC layer, project/scene,
cascade/profiles, drivers, slice/preview pipeline, printer instances,
renderers, cross-cutting ponytail sweep); every finding was adversarially
re-verified against the code, the `src/` frontend, and the tests. Six candidate
findings were refuted and dropped (global-cancel race, camera-restart race,
per-frame gizmo re-upload, emit_sink panic, an untested-cut claim, and a
"dead validate.rs" that is actually live).

**19 findings: 1 high, 4 medium, 14 low.** One real data-loss bug (cut
connectors silently vanish on clone/move — the canonical split-to-fit
workflow), a cluster of session-lifetime resource leaks and non-atomic writes,
and a healthy pile of dead IPC commands / speculative state to delete before
release. The backend architecture (single `Project` mutex + pure-mutation →
event-emit contract, the post-C-1 live cascade tier path, the driver registry)
is sound.

Item format: `ID — title` · files · action · *verify:*. Checkboxes unchecked;
nothing here is applied or committed.

---

## Phase 0 — The one real bug

- [ ] **T-B1 — `clone_objects` / `move_objects_to_plate` silently drop cut connectors** *(HIGH)*
  `core/project/mutation/geometry.rs:318` (clone), `:1231` (move), sidecars populated at `:773,779` (`apply_cut`), read at slice time `core/slice/input.rs:459-477`.
  Cut connectors live in per-plate sidecar maps `PlateSceneState.object_modifiers` / `object_hole_markers`, keyed by `ObjectId` (pegs = positive volume, holes = negative). Of the four object-lifecycle ops only `delete_objects` touches them. `clone_objects` copies mesh/transform/overrides/group but **not** the sidecars → a duplicated cut half has no connectors. `move_objects_to_plate` moves the object (keeping its `ObjectId`) and its overrides/selection but **not** the sidecars → the target plate slices without pegs/holes, and the source keeps a dead entry forever (which also pins the connector meshes so `prune_orphan_meshes` never frees them — bloating every subsequent `.n3o` save). Failure: split with dowel/peg connectors, then send one half to another plate (the canonical split-to-fit-build-volume workflow) or Ctrl-D — the print has flat mating faces, halves won't register, no warning. The move/clone tests assert overrides/groups/materials/transforms but never modifiers, so it's untested.
  *Action:* in `move_objects_to_plate`, move `object_modifiers.remove(&id)` / `object_hole_markers.remove(&id)` to the target scene alongside `object_overrides`; in `clone_objects`, clone them under the fresh `ObjectId` (registering fresh modifier meshes as `apply_cut` does). Add a test that a cut object's connectors survive both ops.
  *verify:* slice a moved/cloned cut half and grep the temp `.3mf` / G-code for the negative-volume holes; confirm they're present.

---

## Phase 1 — Documentation truth-up

- [ ] **T-D1 — `JobRegistry::remove` doc contradicts the now-live C-12 prune**
  `core/slice/job.rs:193-204`, `orchestrator.rs:308-317`.
  The doc says "Not currently wired… the registry is never pruned, so completed handles accumulate for the lifetime of the process." False since C-12: `spawn_worker` sleeps `JOB_RETENTION_AFTER_TERMINAL` (30s) then `registry.remove(job_id)`. Describes a removed state (violates "docs describe the present, not removals") and misleads anyone verifying the prune.
  *Action:* rewrite to present tense — the worker prunes ~30s after the terminal event; `slice_status` errors "no such job" once pruned.
  *verify:* doc read matches `orchestrator.rs` behavior.

---

## Phase 2 — Delete / simplify (release cleanup)

The "no dormant remnants" sweep. Each is grep-verified to have no live consumer
(frontend `src/` + tests checked).

- [ ] **T-X1 — `plate_cascade_trace` command + trace machinery is dead** *(ponytail)*
  `core/project/commands.rs:104-116`, `core/cascade/trace.rs`, `core/project/resolve.rs:218` (`resolve_plate_with_tiers`), `lib.rs:269`.
  Registered but no `src/` invoke (only `plate_cascade_resolve` is consumed). Distinct from the live C-1 resolve path — this is the never-wired "why is X=Y" trace variant. It keeps `resolve_plate_with_tiers`, the `cascade::trace` module, and `Trace`/`TraceRule` alive solely for an uncalled IPC entry point (plus one test that only exercises the dead methods).
  *Action:* delete the command + its registration + `cascade::trace` + `Trace`/`TraceRule` + `resolve_plate_with_tiers` + the lone test. Re-add when the panel affordance is actually built. **Note:** part-1's `plate_cascade_trace` (C-1) shipped the *seam*; deleting it here is the release-cleanup call on an unwired seam — confirm with the owner it's not imminent (see DP below).

- [ ] **T-X2 — `user_process_get` command is dead** *(ponytail)*
  `core/process/mod.rs:28`, `lib.rs:209`, frontend `src/settings/processFragment.ts:107` (`getUserProcess`, itself uncalled).
  *Action:* delete the command + registration + the dead `getUserProcess` wrapper.

- [ ] **T-X3 — `process:user_changed` event has no listener** *(ponytail)*
  `core/process/mod.rs:17-23`, emitted from `core/project/commands.rs:229,300,372,409`.
  The Quality picker refreshes by awaiting each command and bumping `processGen` (`SettingsPanelHost.tsx:490,536,569,594`), not via the event. `emit_changed`, the `PROCESS_CHANGED` const, and all four call sites are dead; the `mod.rs:15-16` doc claiming event-driven refetch is stale.
  *Action:* drop `emit_changed` + `PROCESS_CHANGED` + the four call sites (or wire a listener if cross-window refresh is actually wanted — it isn't today).

- [ ] **T-X4 — `PlateMetadata` / `composition_order` is speculative dead state** *(ponytail)*
  `core/project/metadata.rs`, `mutation/plates.rs:106-117` (renumber block), `model.rs` (`Plate.metadata`), frontend `src/viewport/types.ts:94`.
  One-field struct; `composition_order` is only written (declaration position + auto-renumber on remove) and serialized, never read by any slice/panel/plugin/UI. Its own doc calls the reordering UI "a future polish pass" and the sibling `cycle_count` "cut as MVP scope."
  *Action:* delete `metadata.rs` (+ tests), `Plate.metadata`, the renumber block, the `PlateMetadataChanged` composition-order semantics, and the frontend field. Re-introduce a real per-plate metadata type when the plate-cycler UI lands.

- [ ] **T-X5 — Dead cascade file-loaders `load_cascade` / `load_override_file`** *(ponytail)*
  `core/cascade/loader.rs:90`, `core/cascade/overrides.rs:115`.
  Zero callers in src/examples/tests. Runtime reads bundled fragments via `profile_library::load_*_fragment`; the slice path builds `FlatOverrides` from serialized specs via `parse_override_str`; the panel builds them from in-memory HashMaps via `tier()`. Neither loads an override `.toml` from disk.
  *Action:* delete both (prune `cascade/mod.rs` re-exports); keep the live `parse_cascade_str` / `parse_override_str`.

- [ ] **T-X6 — `SemanticComment::ExtruderTemp`/`BedTemp` + `LayerSource::Heuristic` dead variants** *(ponytail)*
  `core/gcode/model.rs:370-397`, `core/plugin/bindings/gcode.rs:285,310,311`.
  The parser never constructs `ExtruderTemp`/`BedTemp` and always emits `LayerSource::Marker`; the three variants exist only as unreachable match arms in the plugin binding. Speculative forward-compat (docs say "Phase 6 may upgrade this" / "not yet produced").
  *Action:* delete the variants + their unreachable consumer arms; re-add with a producer.

- [ ] **T-X7 — `default_printer_identity` + `DEFAULT_IDENTITY` OnceLock dead in the shipped binary** *(ponytail)*
  `core/printer/registry.rs:104-110`, `mod.rs:51`.
  Non-`cfg(test)` `pub fn` (ships in release) whose only exerciser is its own test `default_picks_first_printer`; no other caller anywhere. Doc even concedes "Only test setups call this," yet none do.
  *Action:* delete the fn, the OnceLock, the re-export, and the self-referential test.

- [ ] **T-X8 — `PluginHost::set_enabled` + `find_mut` dead outside tests** *(MEDIUM; ponytail)*
  `core/plugin/host.rs:436,477`, `commands.rs`, `lib.rs:314-317`.
  Implements per-session enable/disable + stale-error-clearing, but no Tauri command wraps it and the frontend never invokes a per-session enable. Recovery from an auto-disabled plugin already flows through the wired `reload()`. Only three unit tests call it; the `last_error`-clearing logic is unreachable at runtime.
  *Action:* delete `set_enabled` + `find_mut` + the tests that only exercise them — or wire a command if a per-session panel toggle is intended (it isn't today).

- [ ] **T-X9 — `PayloadKind::Gcode3mf` variant never constructed** *(ponytail)*
  `core/plugin/hooks.rs:151-156`, `core/driver/send.rs:161,184,189`.
  `apply_pre_send` early-returns for `SendPayload::Gcode3mf` before building a hook and always constructs `PayloadKind::Gcode` for the one case that reaches a plugin. So the Lua `kind` field is always `"gcode"` and the second variant is dead — no plugin can observe it.
  *Action:* drop `PayloadKind` to the single reachable case (or remove the enum + the `kind` field) until a bundle-editing hook exists.

- [ ] **T-X10 — Camera worker reimplements exponential backoff** *(ponytail)*
  `core/driver/camera.rs:54-55,287`, `core/driver/backoff.rs:30-34`.
  Third, divergent copy of reconnect backoff (1.5× capped at 30s) vs the shared `reconnect_backoff_secs` (2^n capped at 60s) that Bambu + U1 already use, with no stated reason.
  *Action:* reuse `reconnect_backoff_secs` (track an attempt counter), delete the camera-local consts + manual growth — or document why the camera wants a gentler curve.

- [ ] **T-X11 — `engine_default_serialized` rebuilds the whole ~600-entry option table to read one key's default** *(MEDIUM; ponytail)* — pairs with part-1 **FFI-CACHE**.
  `core/printer/options.rs:314-319`, `core/project/resolve.rs:342-347`, `crates/slic3r-ffi/src/lib.rs:971-983,986-996`.
  Calls `option_defs()` (one FFI round-trip + String-allocating build per def, all thrown away) then linear-`.find`s one key. The FFI already exposes the single-key `option_def(key)` (`slic3r_option_def_lookup`). The tower-geometry closure invokes it up to 5×/resolve → ~5 full-table rebuilds per prime-tower recompute (every override/material/printer edit).
  *Action:* switch to `slic3r_ffi::option_def(key)`; optionally cache `option_defs()` in a `OnceLock<Vec<OptionDef>>` (immutable post-init), which also cheapens the panel summary surfaces. (The crate-side cache is FFI-CACHE; do them together.)
  *verify:* tower resolve produces identical geometry; a microbench shows the per-key full-table rebuild gone.

---

## Phase 3 — Correctness & robustness

- [ ] **T-C1 — `instance_storage::persist` writes instance TOML non-atomically → a torn write drops the whole printer (incl. saved credentials)** *(MEDIUM)*
  `core/printer/instance_storage.rs:106-112` (`std::fs::write`), load path `:90-100` (parse error → "skipping malformed instance file", drops it), routed from `instance_registry.rs:198,586`.
  A crash/power-loss mid-write leaves truncated TOML; next launch silently drops the entire `PrinterInstance` — slot bindings, installed nozzles, and persisted `ConnectionInfo` host + access_code. The codebase already has the tmp-then-rename idiom for exactly this (`config.rs:166-173`, `snapmaker/snap_token.rs:113`, `orchestrator.rs:431`); `persist()` is the outlier. (Distinct from the accepted "registry mutex across disk write" tradeoff, which is about lock hold time, not atomicity.)
  *Action:* write to `<id>.toml.tmp` then `std::fs::rename` over the final path (atomic same-fs), cleaning up tmp on failure. Small shared helper.
  *verify:* kill the process mid-persist (or unit-test the helper); the old file survives intact, no instance lost.

- [ ] **T-C2 — Renderer `GpuMesh` cache grows unbounded within a session (GPU buffer leak)** *(MEDIUM)*
  `viewport_render.rs:894` (`meshes: HashMap<MeshId, GpuMesh>`), inserts `:1408,1480,1984`, only removal is wholesale `clear_meshes()` (`:1342-1349`, called only on project replace, `project_io.rs:31`).
  The Project side GCs meshes on delete/cut (`prune_orphan_meshes`, `geometry.rs:1195-1218`) but the renderer cache is never reconciled. Every deleted object, removed primitive, and cut (fresh MeshIds per piece + connector volumes, and re-cut mints more) leaves its vertex+index GPU buffers resident for the session — a heavy editing session accumulates hundreds of orphaned buffers while the visible scene stays small.
  *Action:* in `frame`, while the Project lock is held and objects+modifiers are already iterated, collect live MeshIds and `self.meshes.retain(|id,_| live.contains(id))` once per frame (or prune against `project.meshes` keyset). Same reconciliation covers `tower_meshes` on plate delete.
  *verify:* cut/delete repeatedly in one session; GPU memory (or the map len) stays bounded, not monotonically growing.

- [ ] **T-C3 — `read_rgba` panics on device loss / map failure and poisons `ViewportState`, bricking the viewport for the session** *(LOW; arch)*
  `viewport_gpu.rs:89-102` (discards `map_async` + `poll` results, unconditionally `get_mapped_range()`), `viewport_render.rs:2123-2125` (holds the `ViewportState` guard across `r.frame(...)`).
  On mapping failure / device lost / GPU reset / Windows TDR, `get_mapped_range` panics on the synchronous IPC path (`viewport_frame`/`thumbnail`/`toolpath_frame`); because the guard is held across the call, the panic poisons the managed-state Mutex, so every later `state.0.lock().unwrap()` in frame/thumbnail/tower_grab/move_tower panics too. One transient GPU hiccup permanently disables the whole viewport + gizmo + tower surface.
  *Action:* check the `map_async` callback result and `device.poll` return; on error return an empty/last-good frame instead of `get_mapped_range`. Return `Result` from `frame` so the command surfaces an error rather than poisoning the Mutex.
  *verify:* inject a map failure (or code-review); a bad frame degrades to one blank frame, subsequent commands still work.

- [ ] **T-C4 — `printer_instance_delete_with_reassign` commits plate rebinds but drops their events when `delete_instance` errors** *(LOW; arch)*
  `core/printer/mod.rs:211-258`.
  The rebind loop mutates Project under the lock (released at `:253`); `delete_instance` runs next with `?` (`:254`); `emit_all` only fires after it succeeds (`:255`). On `UnknownInstance` (stale/duplicate id, or a concurrent delete) the plates are already rebound in the backing Project but no events emit — the frontend mirror shows the old binding until the next `scene_snapshot`. The doc claims this closes the partial-commit window; the mutate→maybe-fail→emit ordering reopens one.
  *Action:* emit `all_events` regardless of the delete outcome (or do the delete inside the same locked mutation and only rebind if it succeeds).
  *verify:* a test that deletes a nonexistent instance after a rebind still emits the rebind events.

- [ ] **T-C5 — Bambu status worker clobbers the event loop's `Reconnecting` state with a stale `Connected`** *(LOW; arch)*
  `core/driver/bambu/status.rs:697,710-724`, `connection.rs:691-700`.
  Two tasks write the same `watch::Sender<PrinterStatus>`: the event loop sets `Reconnecting` on a Disconnect packet via `send_modify`; `run_worker` holds a long-lived `pending` snapshot (connection force-set to `Connected` on every report, never to Reconnecting/Disconnected) and flushes the *whole* value each ~1s. Sequence: report at T=0 → link drops at T=0.5, event loop writes `Reconnecting` → T=1s worker flush overwrites with `Connected`; the event loop then sleeps the backoff (up to 60s) without another write, so the panel shows "Connected" for the whole outage.
  *Action:* don't let the worker own the `connection` field — merge job/temps/extra via `send_modify` leaving `connection` to the event loop (or re-read the current connection into `pending` before each flush).
  *verify:* drop a connected Bambu link; the panel shows Reconnecting within a tick, not Connected-through-backoff.

- [ ] **T-C6 — `SendCancelRegistry` assumes the driver lock serializes sends, but `send()` takes a shared read lock** *(LOW; arch)*
  `core/driver/commands.rs:62-64` (doc claims one-in-flight), `:472-473` (`sends.arm(id)` then `handle.read().await`), `:72-75` (arm overwrites by key).
  The read lock is shared, so two `driver_send_plate` calls for the same `DriverId` can both hold it and run `send()` concurrently; the second `arm` overwrites the first's token, so `driver_send_cancel` fires only the later send and the earlier upload becomes uncancellable (for Bambu, two FTPS uploads + two `project_file` publishes race to the printer). Only the frontend header→one-active-plate binding prevents it today; the stated invariant is false.
  *Action:* enforce single-in-flight explicitly (return busy/Err if a token is already armed for the id), or track tokens per-send so cancel maps to the intended upload. At minimum fix the comment.
  *verify:* fire two concurrent sends to one driver in a test; the second is rejected or independently cancellable.

---

## Decision points — need a call before executing

- **DP-T1 (`plate_cascade_trace`, T-X1).** Part-1 C-1 deliberately shipped the trace *seam* (command + `Trace` type) as "the consumable seam; no popover ships here." T-X1 is the release-cleanup argument to delete the unwired seam. Keep it dormant for the imminent "why is X=Y" popover, or delete now and re-add with the UI? The reviewers recommend delete (no-dormant-remnants standard); confirm the popover isn't next-sprint.

---

## Accepted tradeoffs — no action (re-confirmed intentional)

Verified against the prior review's settled list and re-checked this pass; not
re-flagged: the two global-state conventions, composer libslic3r vector-quirk
assembly, Bambu pre-spawn connect probe, macOS Bambu cert handling, the
binding-coherence gate living in the command, the plugin-host Mutex across the
fold, the registry mutex hold across disk write (lock time — distinct from
T-C1's atomicity gap), FE/BE connection field-validation duplication, vendored
suppaftp, R-15's documented `printer ↔ profile_library` cycle, and C-14
(DriverId↔instance_id in the frontend). Refuted-and-dropped this pass: a
global-cancel slice race, a camera-restart teardown race, per-frame gizmo
re-upload, an `emit_sink` panic claim, and "validate.rs is dead" (it is live on
the C-1 path).
