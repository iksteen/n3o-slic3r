# PR-7b-3 — U1 status parser → mounted toolhead + per-toolhead state

Status: ❌ open.

**Scope.** Take Moonraker's `notify_status_update` messages and
merge into the typed `PrinterStatus` the trait surface exposes.
Covers per-toolhead temperatures, the currently-mounted
toolhead, per-toolhead loaded-filament state, and the standard
job-progress fields.

Reference: `bambu-overlay/src/snapmaker/{report.rs,moonraker.rs:162-176}`.

**Acceptance criteria.**

- New module `core/driver/u1/status.rs`:
  - `U1Message` — serde mirror of Moonraker's
    `notify_status_update` payload (a `params` array whose
    first element is a `HashMap<String, serde_json::Value>`).
    Parse into a typed `StatusObjects` struct with one field
    per subscribed object (`print_stats`, `extruder`, etc.).
  - `U1Extra` — `DriverExtra::U1` variant data:
    `mounted_toolhead: u8` (0..3), `toolhead_filaments: [Option<U1Filament>; 4]`,
    `current_stage: String`, `fan_speed: f32`.
  - `U1Filament { material_type: String, color: String }` —
    no brand/SKU (Moonraker doesn't expose it for U1).
  - `merge_into(snapshot: &mut PrinterStatus, msg: U1Message)`
    — same delta-merge semantics as PR-7a-3.

- **Field mapping** (Moonraker object → `PrinterStatus`):
  - `print_stats.state` → `JobProgress.state`:
    `standby → IDLE`, `printing → RUNNING`, `paused → PAUSED`,
    `complete → FINISH`, `cancelled|error → FAILED`. Mirror
    `bambu-overlay/src/snapmaker/report.rs:57-69`.
  - `print_stats.filename` → `JobProgress.file_name`.
  - `display_status.progress` → `JobProgress.percent`
    (Moonraker reports as 0.0..1.0; multiply by 100).
  - `virtual_sdcard.progress` → fallback for `percent` if
    `display_status` is missing.
  - `print_stats.print_duration` + `print_stats.total_duration`
    → derive `JobProgress.eta_seconds` (rough: total - print).
  - `toolhead.position` → not surfaced in `PrinterStatus` for
    MVP (could feed Phase 9 viewport indicator).
  - `toolhead.extruder` → `U1Extra.mounted_toolhead`. Decode
    `"extruder"` → 0, `"extruder1"` → 1, etc. Mirror
    `bambu-overlay/src/snapmaker/report.rs:76-86`.
  - `extruder.temperature` + `extruder.target` → `Temps.nozzles[0]`.
  - `extruder1.{temperature,target}` → `Temps.nozzles[1]`. (etc)
  - `heater_bed.temperature` + `heater_bed.target` → `Temps.bed`.
  - `fan.speed` → `U1Extra.fan_speed`.
  - `print_task_config.filament_color_rgba[i]` →
    `U1Extra.toolhead_filaments[i].color`. (Hex RGBA, no `#`.)
  - `print_task_config.filament_type[i]` →
    `U1Extra.toolhead_filaments[i].material_type`. Mirror
    `bambu-overlay/src/snapmaker/report.rs:108-145`.
  - `print_task_config` absence (no print in progress) →
    `toolhead_filaments` all `None`.

- **Layer count**: Klipper / Moonraker doesn't natively expose
  layer count. Use the slicer-emitted G-code comment scan if
  the printer reports `print_stats.filename` — for MVP, leave
  `current_layer`/`total_layers` as `None` and surface a "no
  layer count" placeholder in the UI. Document this as a known
  gap in the panel ticket (PR-7b-8).

- **Status worker task** in `core/driver/u1/websocket.rs`:
  same shape as PR-7a-3's: mpsc → merge → 1Hz throttle →
  `watch::Sender`.

- **`U1Extra.mounted_toolhead`** change emits a Tauri event
  `driver:toolhead_changed` so the UI can light up the new
  toolhead in the per-toolhead state strip.

- Tests:
  - **`merge_full_snapshot`** — fixture: a captured
    subscribe-response `result.status`. Assert all fields
    map.
  - **`merge_status_delta`** — fixture: a captured
    `notify_status_update` with only `extruder1.temperature`.
    Assert other fields untouched.
  - **`state_mapping_covers_all_klipper_strings`** — pin the
    `standby|printing|paused|complete|cancelled|error` →
    `JobState` mapping in a table-driven test.
  - **`active_toolhead_decodes_extruder_string`** — assert
    `"extruder"` → 0, `"extruder2"` → 2.
  - **Capture fixtures** in `tests/fixtures/u1-moonraker/`:
    `subscribe_response_idle.json`, `notify_layer_advance.json`,
    `notify_toolchange.json`. Sourced from a live U1 by the
    implementer.

**Effort.** ~1.5 days. Most of it is fixture capture + the
table-driven state-mapping test.

**Dependencies.** PR-7b-2 (raw message channel), PR-7a-1
(`PrinterStatus` shape).

**Out of scope.**

- Layer count from G-code comment scan — explicit MVP gap.
- Mounted build plate — see PR-7b-3-followup discussion in
  the index doc; if Snapmaker reports it as a custom Klipper
  object, add it here in a follow-up.
- Stall/error recovery beyond surfacing FAILED state.
