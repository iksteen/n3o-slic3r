# PR-7a-3 — Bambu status parser → `PrinterStatus` + mounted plate

Status: ❌ open.

**Scope.** Take the raw MQTT report messages PR-7a-2 hands off
and turn them into typed `PrinterStatus` updates that flow into
the `watch::Sender` the trait surface in PR-7a-1 exposes.
Includes the rate-limiter that keeps the firehose from spamming
the frontend.

Reference: `bambu-overlay/src/bambu/{models.rs,report.rs}`.

**Acceptance criteria.**

- New module `core/driver/bambu/status.rs`:
  - `BambuMessage` — serde mirror of Bambu's `device/<ID>/report`
    payload shape (a `print` object with nested fields). Mirror
    bambu-overlay's `BambuReport` shape from `models.rs:1-179`.
  - `BambuExtra` — the `DriverExtra::Bambu` variant data:
    `mounted_plate: Option<String>`, `current_stage`,
    `print_error_code`, `fan_speed`. (AMS lives in PR-7a-4's
    own field on this struct.)
  - `parse_message(bytes: &[u8]) -> Result<BambuMessage, ParseError>`.
  - `merge_into(snapshot: &mut PrinterStatus, msg: BambuMessage)`
    — Bambu sends delta updates after the initial pushall;
    merge only the fields present in this message, leave the
    rest. Mirror `bambu-overlay/src/bambu/models.rs:148-179`'s
    merge semantics.

- **Field mapping** (Bambu MQTT payload → `PrinterStatus`):
  - `print.gcode_state` → `JobProgress.state`:
    `IDLE / RUNNING / PAUSE / FINISH / FAILED`.
  - `print.layer_num` → `JobProgress.current_layer`.
  - `print.total_layer_num` → `JobProgress.total_layers`.
  - `print.mc_percent` → `JobProgress.percent`.
  - `print.mc_remaining_time` → `JobProgress.eta_seconds`.
  - `print.gcode_file` → `JobProgress.file_name`.
  - `print.nozzle_temper` + `print.nozzle_target_temper` →
    `Temps.nozzles[0]`.
  - `print.bed_temper` + `print.bed_target_temper` → `Temps.bed`.
  - `print.chamber_temper` → `Temps.chamber`.
  - `print.bed_type` → `BambuExtra.mounted_plate` — feeds the
    BuildPlate cascade layer (see PR-1-7's BuildPlate context).
  - `print.stg_cur` → `BambuExtra.current_stage`.
  - `print.print_error` → `BambuExtra.print_error_code`.
  - `print.cooling_fan_speed` → `BambuExtra.fan_speed`.

- **Status worker task** (in PR-7a-2's connection module):
  - Owns the `mpsc::Receiver<BambuMessage>` from the rumqttc
    forwarder + the `watch::Sender<PrinterStatus>`.
  - On each message, `merge_into` the current snapshot.
  - **Rate-limit** UI updates to ≤1 Hz (configurable):
    accumulate merges, fire `watch.send_replace` on a Tokio
    `interval(Duration::from_millis(1000))` tick. Log every
    incoming raw message at `trace!` regardless, for
    diagnostics.

- **Mounted plate handoff**:
  - When `mounted_plate` differs from the previous snapshot,
    emit a Tauri event `driver:bed_changed` with payload
    `{ driver_id, bed: String }`. The cascade resolver
    consumes this to re-resolve `build_plate.identity` for
    plates bound to this printer (see PR-7c-6's persistence
    hook for the policy choice between "auto-update plates"
    and "warn the user").

- **`BambuMessage` is forward-compatible**: unknown keys are
  ignored, not failed. Bambu firmware updates routinely add
  fields; we don't want a firmware bump to break the driver.
  Use `#[serde(default, flatten)] extras: serde_json::Value`
  to capture-but-ignore.

- Tests:
  - **`parse_message_handles_pushall_snapshot`** — fixture: a
    captured pushall payload (full snapshot). Assert every
    field maps correctly.
  - **`parse_message_handles_delta_update`** — fixture: a
    captured delta. Assert merge into a baseline snapshot
    leaves untouched fields unchanged.
  - **`parse_message_ignores_unknown_fields`** — synthetic
    payload with a future-firmware key, no error.
  - **`rate_limiter_emits_at_most_one_hz`** — drive 100
    messages/sec into the worker, count `watch::Sender` emits,
    assert ≤ 1.1 per second.
  - **Capture fixtures** in `src-tauri/tests/fixtures/bambu-mqtt/`:
    `pushall_idle.json`, `pushall_printing.json`,
    `delta_layer_advance.json`. Sourced from a live A1 mini by
    the implementer (mqtt subscribe + dump).

**Effort.** ~2 days. Most of it is fixture capture from a real
printer + writing the field mapping faithfully against the
Bambu-overlay reference.

**Dependencies.** PR-7a-1 (`PrinterStatus`, `DriverExtra`),
PR-7a-2 (raw message channel).

**Out of scope.**

- AMS data — PR-7a-4 (the `print.ams` sub-payload is parsed
  separately and slotted into `BambuExtra.ams`).
- UI rendering — PR-7a-7 consumes the status stream.
- Persistent status log — diagnostic-only; not stored.
