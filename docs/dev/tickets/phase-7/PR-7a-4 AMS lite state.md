# PR-7a-4 — AMS lite state read

Status: ❌ open.

**Scope.** Parse the `print.ams.ams[].tray[]` sub-payload of the
Bambu MQTT status into a typed per-slot filament identity model,
slot it onto `BambuExtra`. Live state only; user-side override +
mismatch detection lives in Phase 7c.

Reference: `bambu-overlay/src/bambu/{models.rs:181-200,report.rs:38-141}`.

**Acceptance criteria.**

- Extend `core/driver/bambu/status.rs`:
  - `AmsState { units: Vec<AmsUnit>, active_slot: Option<u8> }`
  - `AmsUnit { id: u8, trays: Vec<AmsTray> }` — A1 mini has one
    unit with 4 trays; the model permits N units for forward
    compat with full AMS.
  - `AmsTray { id: u8, identity: Option<AmsFilament> }` — `None`
    when no spool is loaded.
  - `AmsFilament { tray_type, color, sub_brand, multi_colors }`
    — type is `"PLA"` / `"PETG"` / `"ABS"` / etc; color is an
    RGBA hex string (`RRGGBBAA`); sub_brand is Bambu's
    spool-specific descriptor when reported; multi_colors is
    `Vec<String>` for variegated spools.

- Field mapping (from
  `bambu-overlay/src/bambu/report.rs:38-141`):
  - `print.ams.ams[]` → `AmsState.units[]`.
  - For each unit, `unit.tray[]` → `AmsUnit.trays[]`.
  - Per tray: `tray.tray_type`, `tray.tray_color`,
    `tray.tray_sub_brand` (optional), `tray.cols[]` (optional,
    for multi-color spools).
  - Active slot: `print.ams.tray_now` encoded as
    `unit_id * 4 + tray_id` — decode into `(unit_id, tray_id)`
    + flatten to a single-byte slot index for the simple case
    (1 unit, 4 trays: slot index 0..3). Document the encoding
    in a code comment so the implementer doesn't trip on the
    inversion.

- **Color normalization** (mirror `report.rs:127-141`):
  - A tray with no spool reports `tray_color = "00000000"`
    (fully transparent black). Surface as
    `AmsTray.identity = None`, not as a "loaded black spool."
  - Sub-brand of empty string → `None`.

- **`BambuExtra.ams`** field added, populated from `AmsState`.

- Worker integration: the same status worker that PR-7a-3
  spawned merges AMS deltas. Bambu sends AMS state in the same
  `print.ams` sub-object; the `merge_into` from PR-7a-3
  recurses naturally.

- Tests:
  - **`parse_full_ams_4_loaded`** — fixture: pushall snapshot
    with all 4 trays loaded with distinct PLA colors. Assert
    `units[0].trays.len() == 4`, each `identity` populated.
  - **`parse_ams_3_loaded_1_empty`** — assert the empty tray's
    `identity` is `None`, not a phantom black spool.
  - **`parse_ams_multicolor`** — fixture with a variegated
    spool, assert `multi_colors` populated + first color is
    the primary.
  - **`active_slot_encoding_decodes`** — assert `tray_now = 5`
    decodes to `unit=1, tray=1` (and our flat-slot index = 5
    for unit 0–N concatenation).
  - **Capture fixtures** in `tests/fixtures/bambu-mqtt/`:
    `ams_4loaded.json`, `ams_3loaded.json`, `ams_multicolor.json`.

**Effort.** ~1 day. The parsing is rote — most of the time is
in fixture capture.

**Dependencies.** PR-7a-3 (`BambuMessage`, status worker).

**Out of scope.**

- AMS write (loading / unloading a slot from the app) — out of
  scope for Phase 7. The user does this on the printer.
- Full AMS detection (16-slot variant) — single-unit AMS lite
  only. The model permits multi-unit but the differentiator is
  "is this an A1 mini" which isn't a Phase 7 question.
- Cross-AMS-unit slot binding UI — single-unit only.
- Filament profile resolution from `tray_type` — that's
  PR-7c-2's job (FilamentState ties printer-reported identity
  to a cascade-defined FilamentProfile).
