# PR-7b-6 — Pause / resume / stop commands for U1

Status: ❌ open.

**Scope.** `Driver::command()` impl for the U1. Three Moonraker
JSON-RPC commands, plus state guards + ack-via-status, parallel
to PR-7a-6.

**Acceptance criteria.**

- Extend `core/driver/u1/websocket.rs`:
  - `U1Driver::command(cmd)` matches `PrinterCommand` and sends
    JSON-RPC:
    - `Pause` → `method = "printer.print.pause"`, no params.
    - `Resume` → `method = "printer.print.resume"`, no params.
    - `Stop` → `method = "printer.print.cancel"`, no params.
  - Uses the request/response correlator from PR-7b-2.

- **Ack via status**: after the JSON-RPC response succeeds,
  wait for `notify_status_update` with `print_stats.state`
  matching the expected new state. Timeout 10s →
  `DriverError::Protocol("no command ack")`.

- **State guards** (same shape as PR-7a-6):
  - Pause from non-RUNNING → `DriverError::Other(...)` before
    publishing.
  - Resume from non-PAUSED → same.
  - Stop from IDLE → same.

- Tests:
  - **`pause_sends_correct_jsonrpc`** — stub WebSocket, assert
    request payload.
  - **`resume_sends_correct_jsonrpc`** — ditto.
  - **`stop_sends_correct_jsonrpc`** — ditto.
  - **`command_returns_error_on_no_ack`** — drive a status
    stream that doesn't reflect the new state.
  - **`command_guards_block_invalid_transitions`** — pause
    from IDLE returns error without sending.

**Effort.** ~0.5 days. Mechanical.

**Dependencies.** PR-7b-2 (correlator), PR-7b-3 (state stream
for ack check).

**Out of scope.**

- Klipper-specific `EMERGENCY_STOP` (a separate gcode-emit
  command, not the same as `cancel`). Out of MVP scope; a
  pure print-cancel covers the typical user case.
