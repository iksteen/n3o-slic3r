# PR-8-6 — pre-slice + pre-send hooks

Status: ❌ open.

**Scope.** The remaining two hook points. **pre-slice** lets a plugin
read and modify the resolved settings before the cascade adapter hands
config to libslic3r. **pre-send** lets a plugin transform the bytes a
driver is about to send, per printer. Both reuse the host's dispatch
fold; this ticket adds their payload bindings and wires them into the
slice-input and driver-send paths.

Owns the pre-slice and pre-send parts of **FR-PL-3**.

**Acceptance criteria.**

- **pre-slice** binding + wiring:
  - Hook point: in the slice-input build path, after the cascade
    resolves but before `cascade_adapter` emits the
    `DynamicPrintConfig`. The plugin sees a `settings` Lua table
    (resolved logical key → value, as strings) plus a read-only
    `context` table (printer model, plate type, slot count).
  - `on_pre_slice(settings, context)` returns a (possibly mutated)
    settings table; the host folds across plugins; the result feeds
    the adapter. A plugin setting an unknown key or an out-of-schema
    value is rejected at the adapter's existing validation boundary
    (the plugin can't smuggle past schema validation) — surfaced as a
    plugin error, slice proceeds with the pre-hook settings.
  - Read-only `context`; only `settings` is writable.

- **pre-send** binding + wiring:
  - Hook point: in the driver send path, before `Driver::send`, the
    host dispatches `on_pre_send(payload, target)` where `payload`
    exposes the send bytes + `kind` (`"gcode"` for U1 / `"gcode_3mf"`
    for Bambu — mirrors the driver `SendPayload` enum) and `target`
    is a read-only table (printer model, driver kind). The plugin may
    return replacement bytes (for the `gcode` kind it can edit the
    G-code text; for `gcode_3mf` it gets the raw bundle bytes —
    editing those is opaque/advanced and may be a no-op for most
    plugins).
  - Fold across plugins; the final bytes go to the driver. Isolation
    as ever: a pre-send plugin error leaves the payload untouched and
    the send proceeds.

- Example plugin **rewrite-bed-temp-by-range** (pre-slice): clamps /
  rewrites the resolved bed-temperature setting when it falls outside a
  configured band. Real `plugin.toml` + `main.lua` under
  `examples/plugins/`.

- Verification: a pre-slice integration test runs a slice with the
  bed-temp plugin active and **greps the emitted G-code's config block
  / `M140`** to confirm the rewritten temperature reached libslic3r
  (not just that the Lua ran).

- Tests:
  - pre-slice fold mutates a setting; the adapter receives the mutated
    value; an out-of-schema mutation is rejected without failing the
    slice.
  - pre-send fold rewrites `gcode`-kind bytes; the driver receives the
    transformed payload (drive `Driver::send` against a stub driver).
  - Both hooks: erroring plugin is isolated, pipeline proceeds.

**Effort.** ~2 days.

**Dependencies.** PR-8-3 (host + dispatch), `core/cascade` +
`core/cascade_adapter` (pre-slice point), `core/driver` (pre-send
point + `SendPayload`). pre-send's most useful case (G-code text edit)
is the U1 raw-G-code path; the Bambu `.gcode.3mf` bundle is opaque
bytes for MVP.

**Out of scope.**

- A typed-G-code view for the `gcode_3mf` Bambu bundle (would need
  unpacking the 3MF inside the hook) — pre-send gets raw bytes; plugins
  that want structured editing use **post-slice** instead, which is the
  right layer for G-code transforms.
- Plugin-declared settings driving the example's band → PR-8-9
  (hardcode a default until then).
- Filament-state access inside the hooks → PR-8-8.
