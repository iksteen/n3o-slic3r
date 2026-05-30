# PR-8-7 — platecycler plugin (post-slice macro append)

Status: ❌ open.

**Scope.** The flagship plugin and the architecture's proof point — in
its **redefined, simplified form** (see `phase-8.md` decision 2):
instead of porting the Python tool's multi-plate concatenation, a
post-slice plugin **appends the Chitu PlateCycler eject/swap macro to
the tail of a single plate's G-code**. When that print finishes, the
PlateCycler ejects the finished plate and loads a fresh one — ready for
the next print. No cross-plate composition, no `.gcode.3mf` re-wrap, no
Python/Pillow dependency.

Owns the MVP remainder of **FR-PL-5** (the compose-hook part is
deferred).

**Acceptance criteria.**

- A bundled plugin `examples/plugins/platecycler/` (`plugin.toml` +
  `main.lua`):
  - `hooks = ["post_slice"]`, `printer_compatibility = ["Bambu Lab A1
    mini"]` (the lead's PlateCycler rig; widen later as validated).
  - `on_post_slice(gcode, plate)` appends the swap macro via
    `gcode:append(...)` at the very end of the body (after the slice's
    own end-G-code).
  - Declares a `swap_gcode` string setting (default = the
    `DEFAULT_SWAP_GCODE` ejector+reset macro characterized in
    `docs/spikes/spike-5-platecycler.md`), so a user can tune the macro
    for their hardware. Until PR-8-9 wires plugin settings through the
    cascade, the default is read from the manifest default directly.

- The appended macro is idempotent-safe: re-running post-slice on
  already-appended G-code must not double-append (guard on a sentinel
  comment the plugin writes, e.g. `; n3o:platecycler`).

- The plugin is a pure post-slice transform — it uses **only** the
  G-code bindings from PR-8-4 and the dispatch from PR-8-5. It pulls in
  no new host capability; if it needs one, that's a signal the scope
  drifted back toward the deferred compose hook.

- **Real-hardware smoke** (the exit-criteria proof): slice a print with
  the platecycler plugin active, send it to the project lead's A1 mini
  via the existing send path, and confirm on hardware that the
  PlateCycler **auto-ejects the finished plate** at print end. Captured
  in `docs/phase-8-platecycler-smoke.md` (assumption, method, result —
  same shape as the spike docs).

- Tests (software, pre-hardware):
  - The plugin appends the macro to a sliced fixture's tail; the
    sentinel + macro lines are present; the body above is unchanged.
  - Idempotency: a second post-slice pass does not double-append.
  - Verify-via-G-code: grep the re-sliced output for the macro's
    characteristic commands at the tail.

**Effort.** ~1.5 days software + a hardware-test window. The simplified
scope makes this small; most of the value is the hardware proof.

**Dependencies.** PR-8-4 (G-code bindings), PR-8-5 (post-slice wired),
**Phase 7a** (A1 mini send path for the hardware smoke).
`docs/spikes/spike-5-platecycler.md` supplies the macro.

**Out of scope.**

- Multi-plate concatenation, 3MF metadata rewriting, plate-count
  transforms — all the deferred compose-hook behavior. This plugin
  appends a macro; it does not merge plates.
- Shelling out to the Python `platecycler` tool (the spike's
  alternative) — not needed for the append behavior, and it would
  reintroduce a Python/Pillow runtime dep.
- Per-plate `cycle_count` driving N repeats — that's the
  multi-plate/compose story; deferred.
