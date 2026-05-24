# PR-7a-6 — Pause / resume / stop commands for Bambu

Status: ❌ open.

**Scope.** `Driver::command()` impl for the A1 mini. Three
MQTT commands, structurally identical, one per `PrinterCommand`
variant. Ack via the status stream.

bambu-overlay does not implement these — source from
BambuStudio's `src/slic3r/Utils/PrintHost.cpp`.

**Acceptance criteria.**

- Extend `core/driver/bambu/connection.rs`:
  - `BambuDriver::command(cmd)` matches `PrinterCommand` and
    publishes to `device/<DEVICE_ID>/request` with the right
    JSON shape:
    - `Pause` → `{"print":{"sequence_id":"<n>","command":"pause"}}`
    - `Resume` → `{"print":{"sequence_id":"<n>","command":"resume"}}`
    - `Stop`  → `{"print":{"sequence_id":"<n>","command":"stop"}}`
  - Reuses the sequence-id counter from PR-7a-5.

- **Ack semantics**: after publishing, wait for the next status
  update where `print.command` echoes the published command +
  the new `gcode_state` is consistent with the request (PAUSE
  after pause, RUNNING after resume, FAILED/FINISH after stop).
  Timeout 10s → `DriverError::Protocol("no command ack")`.

- **State guards** (return `DriverError::Other` without
  publishing):
  - Pause from non-RUNNING state.
  - Resume from non-PAUSE state.
  - Stop from IDLE state.

- Tests:
  - **`pause_publishes_correct_command`** — unit, stub MQTT.
  - **`resume_publishes_correct_command`** — unit.
  - **`stop_publishes_correct_command`** — unit.
  - **`command_returns_error_on_no_ack`** — stub status stream
    that doesn't reflect the command, assert
    `DriverError::Protocol`.
  - **`command_guards_block_invalid_transitions`** — pause
    from IDLE returns error without publishing.

**Effort.** ~0.5 days. Mechanical once the connection +
sequence-id plumbing from PR-7a-5 is in place.

**Dependencies.** PR-7a-2 (MQTT publish), PR-7a-3 (status
stream for state checks), PR-7a-5 (sequence-id counter).

**Out of scope.**

- Mid-print cancel cleanup (some printers ask "save current
  layer to resume from?") — A1 mini's stop is unconditional.
- Multi-command queuing — one in-flight command at a time per
  driver; trait method takes `&mut self` so this is enforced
  by the borrow checker.
