# PR-9-1 — slice-path / cascade-resolver correctness gate

Status: ✅ done (2026-05-31). **Confirmed clean** — the live slice path
routes through the resolver + adapter; no fix needed. Verified via
G-code, not just the trace. **Release blocker** cleared for the
independence audit (PR-9-8) / PRD §3.3 #1.

> **Result.** `core/slice/orchestrator.rs::resolve_cascade` composes a
> fresh cascade from the bound `PrinterInstance`; `cascade::resolve` +
> `cascade_adapter::adapt` produce the `DynamicPrintConfig`; the
> `.3mf`/STL is loaded for **geometry only** (`model.load`). Proven at
> the G-code boundary by
> `tests/slice_orchestrator.rs::resolved_bed_temp_reaches_the_engine_for_both_printers`:
> slicing a raw STL (no embedded config to leak), the engine body's
> `M140`/`M190` carry the cascade-resolved `textured_plate_temp`,
> `curr_bed_type` is the context's plate type, and the two MVP printers
> resolve to different temps from their own fragments (A1 mini + bambu-pla
> → 65; U1 + generic-pla → 60). The old "`hot_plate_temp=60` not `55`"
> concern was the wrong key (active plate is Textured PEI →
> `textured_plate_temp`) and pre-dated the compose-context fix (310f7b6);
> the U1 + snapmaker-pla `55` rule fires at compose time, guarded by
> `composer::tests::u1_filament_fragment_printer_rule_fires_at_compose_time`.
> CLAUDE.md's OPEN bullet updated to CONFIRMED.

**Scope.** Confirm — and fix if needed — that the live *slice* path
applies our resolved cascade, not the input `.3mf`'s embedded config
plus the shim's pre-`apply` normalization. CLAUDE.md flags this OPEN:

> A 2026-05-30 U1 slice (`plate-1.gcode.3mf`) emitted baseline
> `hot_plate_temp=60` (not the `Snapmaker U1` rule's `55`), which
> suggests the slice did **not** apply our consolidated filament
> fragments.

The cascade resolver (`core/cascade/`) + adapter (`core/cascade_adapter/`)
are built and tested end-to-end (`tests/reference_profiles.rs`). The
open question is whether `core/slice/input.rs` → `orchestrator.rs`
actually *feed* the resolver's output to libslic3r, or slice from the
embedded project config. This ticket closes that question.

**Acceptance criteria.**

- **Trace the path.** Read `core/slice/input.rs` and
  `core/slice/orchestrator.rs` and document, in the ticket or a smoke
  doc, exactly where the `DynamicPrintConfig` handed to libslic3r comes
  from: the cascade adapter, the embedded `.3mf` config, or a merge.
- **Verify via G-code, not tests** (memory: `verify_via_gcode`). Slice
  a U1 plate bound to the `Snapmaker U1` rule and grep the output:
  the per-plate bed temp must be the **cascade-resolved** value
  (the `55` the rule sets), and per-extruder / filament values must
  reflect the resolved fragments — not libslic3r defaults or the
  embedded config.
- **If it routes correctly:** record the proof (sliced G-code excerpt)
  and update the CLAUDE.md OPEN bullet to confirmed. Done.
- **If it does not:** route the live slice path through the
  resolver + adapter so the resolved cascade is the config of record.
  Frame the fix as "find the pre-`apply` setup we're missing" — Orca
  on the same input produces correct output, so the engine is rarely
  the variable (memory: `libslic3r_vs_our_invocation`). Re-verify via
  G-code.
- Update CLAUDE.md's "OPEN / unverified" bullet under "The cascade
  resolver *is* built" to reflect the finding either way.

**Effort.** ~1–2 days (mostly tracing + a verify slice; the fix, if
needed, is a wiring change, not new resolver work).

**Dependencies.** None — the resolver, adapter, and slice path all
exist. Pull this first; everything downstream assumes correct slices.

**Out of scope.**

- New cascade rules or fragments — this is about *applying* what's
  already resolved, not authoring more.
- Re-architecting the adapter — the dimensional-expansion +
  `curr_bed_type` work is done and tested.
