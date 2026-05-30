# PR-8-8 — read-only filament-loadout Lua bindings

Status: ✅ done (software).

## Scope decision (2026-05-30) — slice-time mapping, not live state

The original ticket scoped this as a *live* filament view: per-slot
`loaded` flag and `mismatch` state sourced from the driver report + the
Phase 7c mismatch detector. Reframed at implementation time: **expose
the slice-time material→slot mapping** — what each physical slot is
*bound* to, as resolved from the `PrinterInstance` — and drop the live
dimension. Two reasons:

1. **The mismatch detector (PR-7c-4) isn't built.** There's no
   `core/filament/mismatch.rs`; 7c landed only the driver→slot *sync*
   (7c-2) and the Materials UI (7c-3). A `mismatch` field would have
   nothing to compute from.
2. **The live driver loadout isn't reachable at the slice hooks.** The
   pre/post-slice dispatch carries `SlicingContext` (vendor profile +
   resolved filament profiles), not the live `PrinterStatus` (AMS
   trays / U1 toolheads). Surfacing `loaded` there is separate
   plumbing.

The bound loadout, by contrast, **is** in scope: `prepare_job` already
resolves the `PrinterInstance`, so its `extruders[].slots[]` bindings
snapshot cleanly with no new plumbing. That's the material→slot mapping
plugins actually need to be material-aware at slice time.

Owns the MVP slice of **FR-PL-7**. The live `loaded`/`mismatch`
surface moves to a follow-up gated on PR-7c-4 + a driver-status thread
into the dispatch (and the pre-send context).

## What shipped

- A read-only `filament` binding, handed to **pre-slice** and
  **post-slice** hooks as the **third positional Lua arg**
  (`on_post_slice(gcode, plate, filament)` /
  `on_pre_slice(settings, ctx, filament)`). Backward-compatible — the
  existing plugins that take two args ignore it.
  - `filament:slots()` → array (1-based) of read-only slot tables:
    `{ index, extruder, slot, feed, identity, type, color, vendor,
    bound }`. `index` is the 1-based flat filament ordinal (material
    `index` emits `T<index-1>`); `extruder`/`slot` are 1-based coords;
    `feed` is `"direct"`/`"ams"`; unbound slots have `bound=false` and
    `nil` identity fields. `type`/`vendor`/`color` resolve from the
    bundled filament catalog (color prefers the per-slot binding color).
  - `filament:slot(i)` → the 1-based i-th slot, or `nil`.
  - `filament:count()` → physical slot count.
  - `filament:printer()` → `{ model, toolhead_count }`.
- **One-way read into host state:** the `FilamentLoadout` lives
  Rust-side behind an `Arc` and is never handed to Lua, so a plugin
  cannot write filament state back into the slice (the FR-PL-7
  guarantee). The handle is immutable userdata (assignment raises). The
  per-slot/printer tables are fresh per-call snapshots whose `=`
  assignment path raises via `__newindex` (`__metatable = false` hides
  the read-through table). Not bulletproof against `rawset` — which by
  design bypasses `__newindex` — but a `rawset` only shadows a key on
  the plugin's throwaway copy and never reaches host state; the guard
  catches honest mistakes, the sandbox owns hostile plugins.
- **Offline-safe:** when the instance can't be resolved the loadout is
  empty — `slots()` returns `{}`, `count()` is 0, no error. (Today the
  instance always resolves, since `resolve_cascade` already proved it;
  the empty path is the defensive fallback.)
- Assembled host-side (`FilamentLoadout::from_instance`) and snapshotted
  into `ResolvedJob.filament` at job prep — it does **not** reach back
  into the instance registry from Lua. Snapshot-at-dispatch: a
  long-running hook sees a consistent view.

## Implementation

- `core/plugin/bindings/filament.rs` — `FilamentLoadout` / `SlotInfo`
  data + the `FilamentHandle` userdata (read-only proxy helper).
- `core/plugin/hooks.rs` — `filament` field on `PreSliceHook` /
  `PostSliceHook`, passed as the third Lua arg.
- `core/slice/orchestrator.rs` — builds the snapshot in `prepare_job`,
  stores it on `ResolvedJob`, threads it into the two dispatch helpers.
- `examples/plugins/filament-summary/` — a read-only example that
  prepends a per-slot loadout header (template for material-aware
  plugins).

## Tests (no real printer)

- `slots()` reflects a stubbed loadout (type/color/vendor/feed, bound vs
  unbound); `slot(i)` range + `0`/out-of-range → `nil`; empty loadout →
  no slots, no error.
- Assigning a slot field **or** a handle field raises, and clean-by-copy
  discards the whole edit (G-code untouched).
- `from_instance` over a bundled fixture instance: slot count + 1-based
  ordinals.
- `filament-summary` example loads and prepends its header end-to-end.

## Not done / follow-ups

- **Live `loaded` + `mismatch`** — needs PR-7c-4 (mismatch detector)
  and a live `PrinterStatus` thread into the slice dispatch. Tracked as
  a post-7c-4 extension of this binding.
- **pre-send** doesn't get `filament`. Its dispatch site
  (`core/driver/commands.rs`) has only `plate_id` + `driver_kind`, no
  instance context; threading the loadout there is separate work.

**Dependencies (met).** PR-8-3 (host/dispatch context), PR-8-5
(post-slice wired), `core/printer` instance topology, `core/filament`
catalog.

## Out of scope (unchanged)

- Writing/binding filament from a plugin (read-only by FR-PL-7).
- Live/streaming updates into a running hook — snapshot is the contract.
- Raw driver protocol details (AMS humidity, etc.).
