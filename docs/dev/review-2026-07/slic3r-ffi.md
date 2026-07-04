# Review — `crates/slic3r-ffi` (libslic3r FFI)

Fresh architectural + over-engineering ("ponytail") review, 2026-07, ahead of
public release. Two lenses per dimension: real defects (boundary safety, leaks,
UB, build correctness) and code to delete/simplify. Five reviewers (boundary
safety, C++ shim, build orchestration, API shape, scraped metadata); every
finding was adversarially re-verified against the actual code and the sole
consumer (`src-tauri`).

**18 findings after merging cross-dimension duplicates: 1 high, 8 medium, 9
low.** One real memory-safety bug (crafted-file → OOB/UB), a cluster of
build-reproducibility gaps that can silently ship a stale engine, and ~450 LOC
of dead FFI surface plus two scraped tables that should move C++-side like the
prior review's `bucket`/`scope` migration (X-3). The boundary design itself
(string-keyed config, single-threaded handles, hand-rolled C ABI + bindgen) is
sound and not in question.

Item format: `ID — title` · files · action · *verify:*. Checkboxes unchecked;
this is a plan, not applied work. Nothing here is committed.

---

## Decision points — need a call before executing

- **DP-A (log sink).** `set_log_sink`/`CallbackLogSink` (lib.rs + shim) has zero
  production consumers, AND the sink is installed unconditionally at init
  (shim `:688-691`), displacing boost.log's default console sink — so with no
  callback registered, libslic3r's own warning/error records go to a no-op sink
  and are **lost in production**. Two clean options: (a) wire `set_log_sink`
  into `tracing` at app startup (recovers the swallowed engine diagnostics), or
  (b) delete the whole machinery in all three layers and let libslic3r log to
  stderr. Do NOT leave it as-is. Feeds F-DEAD.
- **DP-B (`Model::load` path).** `slic3r_model_load[_with_config]` + the
  `set_temporary_dir` init workaround exist only for `examples/slice.rs` (the
  standalone slice-a-3mf harness used for libslic3r-vs-invocation debugging).
  Production never loads a file through the engine. Keep the harness (and its
  ~100 LOC) or delete it? Feeds F-DEAD.
- **DP-C (cstyle vector escaping).** Fix in place vs expose over FFI — see F-CSTYLE.

---

## Phase 0 — The one real bug

- [ ] **FFI-B1 — Malformed MMU paint strings cross the boundary unvalidated → OOB read / unbounded recursion (crafted `.n3o`/3MF)** *(HIGH)*
  `src/lib.rs:1151,1224` (add_object/add_volume apply paint with no validation), `ffi/slic3r_ffi.cpp:605` (apply_paint → set_triangle_from_string, no structural check), `ffi/slic3r_ffi.cpp:354-397` (remap_paint_walk reads `in[c]`/`in[c+1]` on `vector<bool>` unchecked, recurses per attacker-controlled split tree), `external/OrcaSlicer/.../TriangleSelector.cpp:1804-1812` (deserialize `next_nibble` debug-asserts only); input path `core/threemf/core_spec.rs:259-263` → `slice/input.rs:452` → `orchestrator.rs:449-451,766`.
  The crate bounds-checks triangle **indices** (`validate_indices`, lib.rs:1330, explicit "libslic3r indexes unchecked" rationale) but applies **zero** validation to paint hex strings on the same safe APIs. A crafted paint string like `"1"` or `"3"` (declares a split, provides no child bits) walks past the bitstream end — heap OOB read, and a long hostile run of split-declaring nibbles recurses ~2 bits/level → stack overflow. Reached at slice time for any painted model (deserialize) and on toolchanger re-map (remap_paint_walk). Paint arrives verbatim from shared project/3MF files. Note the Rust-side decoder of the *same* format (`core/threemf/paint.rs`) already bounds-checks every read — the C++ walk does not.
  *Action:* validate at the boundary like indices — reject paint arrays whose length ≠ triangle count and strings with non-hex chars (in `apply_paint` C++-side); add `c + needed <= in.size()` guards + a depth cap (or explicit-stack rewrite) to `remap_paint_walk`; bail with `SLIC3R_ERR_INVALID_ARG` instead of reading past the vector. Optionally validate structure once at ingest and reject the file.
  *verify:* craft a `.n3o` with a truncated paint string, load + slice on a toolchanger plate; must return a clean error, not crash. Add a unit test feeding malformed hex to the boundary.

---

## Phase 1 — Documentation truth-up (zero code risk)

- [ ] **FFI-D1 — `libslic3r-workarounds.md` §8 is stale + two shim comment paths dead-end**
  `docs/dev/libslic3r-workarounds.md:373-377`, `ffi/slic3r_ffi.cpp:569,1151`.
  §8 says the validation warning sink is "discard[ed] (we have no warning UI)" — stale: `slic3r_slice` now returns it via `out_warning` (shim `:1158-1161`, header `:340-344`) and src-tauri renders it as a `PlateWarning` (`orchestrator.rs:835`). A future bump reading §8 could "simplify" away live `out_warning` plumbing. Also both shim comments cite `docs/libslic3r-workarounds.md` while the file is at `docs/dev/libslic3r-workarounds.md`. (Upstream premises re-verified at the current pin — no workaround has been upstreamed.)
  *Action:* rewrite §8 to present tense (sink absorbs the null-deref writes; non-empty warning is returned and shown); fix the two comment paths.
  *verify:* grep confirms no `docs/libslic3r-workarounds.md` (no `/dev/`) remains.

---

## Phase 2 — Delete / simplify (release cleanup)

- [ ] **FFI-DEAD — ~450 LOC of dead production FFI surface** *(MEDIUM; ponytail)* — merges the cpp-shim + api-shape "dead surface" findings.
  Zero src-tauri consumers (grep-verified; only tests/examples touch them):
  - plain `cut_mesh` + shim `slic3r_cut_mesh` (`src/lib.rs:213`, `ffi/slic3r_ffi.cpp:1332`) — fully subsumed by `cut_mesh_deferred` with 0 connectors (header says so; split tool uses only the deferred path, `core/scene/commands.rs:883`). ~160 LOC. `cut_connectors_smoke.rs:46` already proves equivalence.
  - `Model::load`/`load_with_config` + shim `do_load` + `set_temporary_dir` init workaround (`src/lib.rs:1093,1111`, `ffi/slic3r_ffi.cpp:839,700`) — **DP-B**.
  - `Config::validate` (slice path already validates in-shim), `version()`, the `slice()` convenience wrapper — crate-test/example-only.
  - log-sink machinery (`src/lib.rs:1611-1705` + shim `CallbackLogSink`/`slic3r_set_log_sink`) — **DP-A**.
  *Action:* delete `cut_mesh` outright, routing `cut_smoke.rs` through the deferred path; resolve DP-A/DP-B; rewrite `tests/api.rs` + examples onto the production `add_object`/`add_group`/`add_volume` entry points (release standard: rewrite tests off dead prod paths, don't keep dormant).
  *verify:* `cargo test -p slic3r-ffi` green after rewrite; `grep -r 'slic3r_ffi::' src-tauri/src` shows no reference to the deleted symbols.

- [ ] **FFI-EXT — `PER_EXTRUDER` scraped table → compute in the shim (the X-3 precedent)** *(MEDIUM; ponytail/arch)* — merges the api-shape + scraped-metadata findings.
  `src/option_printer_pages.rs:439-523` (~85-key table + binary search + sortedness test), `scripts/scrape_option_printer_pages.py:45,139-148`, consumer `core/printer/options.rs:418`.
  Unlike the display-order/page tables (which come from GUI `Tab.cpp`, unlinkable), this set is `PrintConfigDef::extruder_option_keys()` — a **public libslic3r accessor** (`PrintConfig.hpp:593`) reachable from `DefCache::build`, which already computes buckets there (shim `:247-300`, comment at `:239` records the X-3 move). A bump adding a per-extruder key silently desyncs `is_extruder_visible` (`options.rs:408-419`) until re-scrape; the scraper's non-greedy `\{(.*?)\}` regex truncates on a nested brace.
  *Action:* add `per_extruder` to `slic3r_option_def_t`/`DefCache::Entry`, set from `extruder_option_keys()`, surface on `OptionDef`; switch `options.rs:418` to it; delete `PER_EXTRUDER` + `parse_extruder_keys`/`EXTRUDER_KEYS_BLOCK`. Port the sortedness assertions to `tests/api.rs` against the FFI field. Leave display-order/page tables (GUI-sourced, not computable).
  *verify:* FFI `per_extruder` matches the old table for all keys, both printers; extruder panel unchanged.

- [ ] **FFI-SEV — Single-variant `Severity` enum is dead flexibility** *(LOW; ponytail)*
  `src/lib.rs:1443,1455`. `Severity` has one variant (`Warning`); the C boundary carries one `out_warning` string (header `:340-344`), so no second severity can arrive. Every consumer destructures a tuple whose first element is constant.
  *Action:* delete the enum; `SliceOutcome.diagnostics` → `warnings: Vec<String>` (or `Option<String>`); drop the tuple mapping and the src-tauri match ceremony.
  *verify:* `cargo test --workspace` green; slice still surfaces the warning.

- [ ] **FFI-CACHE — `engine_default_serialized` marshals the ~900-entry table per key; the single-key lookup sits unused** *(LOW; ponytail)*
  `core/printer/options.rs:314-319`, `core/project/resolve.rs:342-355` (up to 5×/tower-cache invalidation), `src/lib.rs:986` (`option_def(key)`, backed by the shim by-key hash, zero prod consumers), plus 3 consumer-side caches of the same immutable table (`core/schema/mod.rs`, `bucket_of`, `orca_import` baseline).
  *Action:* cache the decoded table once crate-side (`OnceLock<Vec<OptionDef>>`, populated only after successful init → `&'static [OptionDef]`); switch `engine_default_serialized` to `option_def(key)` or the cached slice; delete the duplicate consumer caches. Fixes FFI-C1's poisoning by construction (populate only when non-empty).
  *verify:* tower resolve produces identical config; a microbench shows the per-key full-marshal gone.

- [ ] **FFI-PATCH3 — Idempotent patch-apply logic hand-rolled 3× with divergent failure semantics** *(LOW; ponytail)*
  `build.rs:350-393` (reverse-check, hard-panic on failed apply), `patches/wave-overhangs/apply.sh:12-24` (forward-check, hard-error; header comment falsely claims partial-apply is skipped), `packaging/windows-cross/build-deps.sh:348-357` (treats *any* failed apply as "already applied", silently continues — weakest, on the deps-bootstrap path a fresh contributor least watches).
  *Action:* generalize `apply.sh` to take a patches dir (all build hosts are unix; Windows cross-builds from Linux), have `build.rs` and `build-deps.sh` shell out to it; fix the false comment.
  *verify:* clean-from-scratch build applies both patch sets; re-run is a no-op; a deliberately broken patch fails loudly on all three entry points.

---

## Phase 3 — Correctness & robustness

- [ ] **FFI-C1 — Pre-init `option_defs()` returns empty and permanently poisons the `bucket_of` cache** *(MEDIUM)* — merges the boundary-safety + api-shape findings.
  `src/lib.rs:866-878` (`bucket_of` OnceLock), `:971-983` (`option_defs`), `ffi/slic3r_ffi.cpp:733-736` (`slic3r_option_def_count` returns 0, not an error, when the cache is null); live trap noted at `core/project/commands.rs:621`, consumers `orca_import/mod.rs:34`, `printer/mod.rs:137`, `commands.rs:158`.
  One `bucket_of` call before `init()` (a plausible startup-ordering or multi-threaded-test race) caches an empty map for the process lifetime — every key thereafter classifies as bucketless with **no error**. The `NOT_INIT` state is representable (`slic3r_option_def_at`) but the count-based path discards it.
  *Action:* distinguish not-initialized from empty — `option_defs()` returns `Result`/panics on `NOT_INIT`; populate `bucket_of`'s OnceLock only from a non-empty table. Combined with FFI-CACHE, fixed by construction.
  *verify:* a test calling `bucket_of` before init then after init returns correct buckets, not `None`.

- [ ] **FFI-C2 — `catch(...)` missing on the highest-third-party-code entry points → non-`std::exception` unwinds the C ABI (UB/terminate)** *(LOW)*
  `ffi/slic3r_ffi.cpp` — `slic3r_slice` (`:1238-1255`), `do_load` (`:863-866`), `slic3r_init` (`:722-724`), `config_set/get/validate` (`:784-798,817-820`), `remap_paint_filaments` (`:912-914`) catch only `std::exception`; but `add_object`/`add_volume`/`orient`/`cut`/`arrange` already carry the trailing `catch(...)`. `Print::process` fans out through TBB/boost/Clipper/CGAL; a non-`std::exception` throw (e.g. `boost::thread_interrupted`, or a raw throw TBB rethrows) unwinds through `extern "C"` — in practice `std::terminate`, bypassing all error plumbing and the Rust `catch_unwind` guards (which cover only Rust panics).
  *Action:* add the same trailing `catch(...) { set_err(...); return SLIC3R_ERR_*; }` arm to those entry points — one mechanical pass mirroring the seven that have it.
  *verify:* code inspection that every `extern "C"` entry point ends with `catch(...)`.

- [ ] **FFI-C3 — `init()`'s `Once` caches a failed first init as permanent success; NUL path panic poisons it** *(LOW)*
  `src/lib.rs:93,99-109`, `ffi/slic3r_ffi.cpp:677-679`. `slic3r_init` can genuinely fail (e.g. `temp_directory_path()` throws on a bad `TMPDIR`), but the `Once` is consumed by that run, so every later `init()` returns the default `Ok(())` while `Config::new()` then null-fails with the misleading "did you call init()?" — even though the C++ side is fully retriable (mutex-guarded re-check). Separately, a NUL in `resources_dir` hits `.expect(...)` inside `call_once`, poisoning the `Once` so all later calls panic. The Rust `Once` duplicates the C++ guard and is the sole source of both.
  *Action:* delete `INIT_GUARD`, call `sys::slic3r_init` unconditionally (shim is idempotent + mutex-guarded); convert the CString `expect` to the crate's standard `InvalidArg` Err.
  *verify:* a test that init fails once (bad `TMPDIR`) then succeeds on retry; NUL path returns `Err`, not panic.

- [ ] **FFI-C4 — Cut/tower marshalling swallows OOM as `SLIC3R_OK` with geometry missing** *(LOW)* — merges the two OOM findings.
  `ffi/slic3r_ffi.cpp:1388-1392` (`cut_mesh` marshal lambda returns silently on malloc failure → "empty half", which the API defines as "mesh entirely on the other side" — split tool silently discards real geometry), `:1717,1728-1731,1748-1750` (deferred cut ignores `conn_marshal` returns + outer-array alloc failure → `SLIC3R_OK` with every peg/hole volume dropped, so halves print without connector holes). The halves path was already fixed (`:1692-1698`); connectors weren't. That error path may also leak the already-marshalled upper half (Rust early-returns without freeing, `src/lib.rs:548`).
  *Action:* make OOM fatal + uniform — check `conn_marshal` returns in the mod/dowel loops, treat outer-array alloc failure as `SLIC3R_ERR_INTERNAL` with an `out_err`; free/`take_cut_half` the already-written buffers on the error path. (If `cut_mesh` survives FFI-DEAD, give its lambda the same treatment.)
  *verify:* fault-inject malloc failure (or code-review the paths); no `SLIC3R_OK` with a null half / zero connector count.

- [ ] **FFI-C5 — `cut_mesh_deferred` silently discards paint on length mismatch / NUL** *(LOW)*
  `src/lib.rs:478-480`. `paint.filter(|p| p.len() == triangle_count)` drops mismatched paint silently (exactly the stale-paint-after-edit case this param exists to carry), and `CString::new(...).unwrap_or_default()` maps interior-NUL strings to unpainted. Every other malformed input here is an `InvalidArg`; paint is the one masked. (If FFI-B1 adds boundary validation, fold this in.)
  *Action:* return `Err(InvalidArg, "paint length N != triangle count M")`; propagate the NUL error via the existing `cstring()` helper.
  *verify:* a re-cut with mismatched paint length returns an error, not a silently-unpainted result.

- [ ] **FFI-C6 — One `enum_value_count` governs two independently sized C arrays → latent OOB on a future upstream def** *(LOW)*
  `ffi/slic3r_ffi.cpp:274-284,321-323`, `src/lib.rs:940-945`. Shim copies `enum_values` and `enum_labels` verbatim but exposes one count (from values); `from_raw` reads `labels[0..count]` whenever labels is non-null. libslic3r doesn't guarantee equal lengths; a def with `0 < labels < values` reads past the label array (UB). No mismatch in the current pin, but nothing enforces it across a bump.
  *Action:* one-line guard in `DefCache::build` — pad/truncate labels to `values.size()` (GUI fallback = value-as-label), or expose a separate `enum_label_count` and honor it.
  *verify:* code inspection; add an assertion in the def-build test.

- [ ] **FFI-C7 — `build.rs` never watches the OrcaSlicer submodule → pin bump / engine edit links a stale libslic3r and false-greens** *(MEDIUM)*
  `build.rs:261-279`, `scripts/sync_orcaslicer.sh:93-99`. `rerun-if-changed` lists only shim/patch files — nothing under `external/OrcaSlicer`. `git checkout <older-commit> && git submodule update` (or any manual pin move, or editing a libslic3r `.cpp`) leaves watched files untouched, so `build.rs` is skipped, `cmake --build` never runs, and cargo links/tests the *previous* engine. `sync_orcaslicer.sh` already admits this and papers over it with a `touch` — helping only people who bump via that one script.
  *Action:* emit `cargo:rerun-if-changed` on the submodule gitdir HEAD (`git -C external/OrcaSlicer rev-parse --absolute-git-dir` → watch `<gitdir>/HEAD`, graceful fallback outside a checkout); optionally watch `external/OrcaSlicer/src/libslic3r` for local edits. Then delete the `touch` hack.
  *verify:* move the pin via plain git, `cargo build` triggers a real libslic3r rebuild; `slic3r_version()` SHA matches the checkout.

- [ ] **FFI-C8 — Cross-build inputs missing from `rerun-if-changed`** *(MEDIUM)*
  `build.rs:162-169,204-217,275-279`, `CMakeLists.txt:90-92`. Only the wave-overhangs dir is watched; `packaging/windows-cross/patches/*.patch`, the three windows-cross `*.cmake`, `packaging/macos-cross/toolchain.cmake`, and `ffi/macos_availability_shim.mm` are not. Editing a windows-cross patch then rerunning `cargo xwin build` skips `build.rs`, keeps the old patch applied, and ships the DLL without the fix — and a later unrelated rerun then panics with the misleading "tree not at the pinned commit" (see FFI-C9).
  *Action:* add `rerun-if-changed` for those paths (unconditional `println!`s; the paths exist on every host).
  *verify:* edit a windows-cross patch, `cargo xwin build` reruns `build.rs`.

- [ ] **FFI-C9 — `apply_submodule_patches`: stranding panic message on the common edited-patch case + silent skip when the patches dir is unreadable** *(LOW)*
  `build.rs:353-359,381-392`. The usual way to hit the hard-fail panic (post-C-2) is iterating on a carried patch — the old version is still applied, so the new one fails both reverse-check and forward apply; the message blames "tree not at the pinned commit" with no recovery step (a fresh contributor regenerating the wave carry hits this every edit). And `Err(_) => return` at `:358` silently drops the whole carry if `read_dir` fails (renamed/sparse checkout) — shipping an engine without the wave module while the scraped tables still advertise `wave_overhang_*` options.
  *Action:* on the panic, check `git -C external/OrcaSlicer diff --quiet` and, if dirty, print the reset command (`git -C external/OrcaSlicer checkout -- . && cargo build`); replace `Err(_) => return` with a panic (dir is committed, must exist) or at minimum a `cargo:warning`.
  *verify:* edit a wave patch → build fails with an actionable message; rename the patches dir → build fails loudly.

- [ ] **FFI-CSTYLE — Hand-ported cstyle vector escape/unescape in the backend diverges from libslic3r's** *(MEDIUM)* — **DP-C**.
  `core/profile_library/composer.rs:775-885` (~110 LOC port), `core/cascade_adapter/adapter.rs:230` (uses split counts to size per-filament vectors), vs `external/OrcaSlicer/.../Config.cpp:72,146`. Three verified divergences: (1) `[""]` — libslic3r quotes it so it round-trips to 1 element; the port drops it (an imported one-filament profile with an empty `filament_start_gcode`/`filament_notes` loses its only vector element — the same vector-length-mismatch class as the cold-start first-layer-temp incident); (2) libslic3r skips whitespace after `;`, the port keeps `" b"`; (3) libslic3r reports malformed input (unterminated quote), the port silently truncates. Same missing-FFI-signal pattern as X-3.
  *Action:* add two shim entry points wrapping `escape_strings_cstyle`/`unescape_strings_cstyle` (plain exported functions, no handle) and delete the Rust port; **or** fix the three divergences and add round-trip tests against engine output.
  *verify:* round-trip `[""]`, `"a; b"`, and an unterminated quote through the chosen path; results match libslic3r.

- [ ] **FFI-C10 — Display-order scrape misses every multi-option-line key: 61 panel-visible options sort to the end** *(MEDIUM)*
  `scripts/scrape_option_display_order.py:43` (matches only `append_single_option_line("KEY"`), vs `scrape_option_printer_pages.py:67-71` (catches all three forms); `src/option_display_order.rs`, consumer `core/printer/options.rs:331,449,549` (`display_order_of(key).unwrap_or(u32::MAX)`).
  Orca lays out its most-edited options via `line.append_option(get_option("KEY"))` / `create_line_with_widget`. Verified set difference: 61 panel-visible keys have no display-order entry — `nozzle_temperature`, the whole bed-temp family, `fan_min/max_speed`, `chamber_temperature`, all machine G-code keys, `filament_start/end_gcode`. So the temperature and cooling rows users touch most render at the **end** of the filament panel, and the machine panel's entire "Machine G-code" page too — silently defeating the table's purpose. The in-file canary only checks Support keys (all single-option form), so it never catches this.
  *Action:* add the other two key forms to the display-order scraper's regex (position = first encounter of any form), regenerate `option_display_order.rs`, extend the canary with a multi-option-line key (assert `nozzle_temperature` has an order and precedes Cooling-page keys).
  *verify:* filament panel renders temps/fans in Orca's order; canary covers a multi-option key.

---

## Accepted tradeoffs — no action (verified intentional)

- String-keyed config over the boundary (libslic3r's own (de)serializer) — deliberate boundary design.
- Single-threaded opaque-handle discipline — documented, deliberate.
- The wave-overhangs patch carry — intentional feature carry; FFI-C8/C9 address only its reproducibility mechanics, not its existence.
- Bindgen + hand-rolled C header (no cxx) — appropriate for this flat ABI.
- Display-order / page / subgroup tables from GUI `Tab.cpp` — genuinely unreachable at runtime (GUI layer not compiled); their scrape is justified. Only `PER_EXTRUDER` (FFI-EXT) is engine-reachable.
