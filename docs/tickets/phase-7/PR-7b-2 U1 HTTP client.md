# PR-7b-2 — Moonraker WebSocket client + JSON-RPC + status subscribe

Status: ❌ open.

**Scope.** First half of the U1 driver: connect to Moonraker's
WebSocket, run the JSON-RPC handshake, subscribe to the printer
object firehose, hand incoming `notify_status_update` messages
off for parsing. Parallel to PR-7a-2 for the Bambu side.

The U1 is standard Klipper-via-Moonraker, **not** a Snapmaker-
specific HTTP wrapper. The PRD's wording (§10.B.6) is incorrect;
the user's `bambu-overlay/src/snapmaker/moonraker.rs:49-58`
confirms it.

**Acceptance criteria.**

- New module `core/driver/u1/`:
  - `mod.rs` re-exports `U1Driver`.
  - `probe.rs` — `GET /machine/system_info` to discover the
    serial number.
  - `websocket.rs` — WebSocket connection + JSON-RPC plumbing.
  - `U1Driver` struct implementing the `Driver` trait
    (PR-7a-1).

- **Add deps** to `src-tauri/Cargo.toml`:
  - `tokio-tungstenite = "0.21"` (WebSocket client).
  - `reqwest` (already present via Tauri; HTTP probe step
    reuses).

- **Probe step** (`probe.rs`), mirroring
  `bambu-overlay/src/snapmaker/probe.rs:21-56`:
  - `GET http://<host>:<port>/machine/system_info`.
  - Parse `.result.system_info.product_info.serial_number`.
  - Cache as the driver's `device_id`. Runs once per
    `connect()`.

- **WebSocket lifecycle** (`websocket.rs`):
  - Connect to `ws://<host>:<port>/websocket`.
  - On open, send the JSON-RPC subscribe request:
    ```json
    {
      "jsonrpc": "2.0",
      "method": "printer.objects.subscribe",
      "params": {
        "objects": {
          "print_stats": null,
          "display_status": null,
          "extruder": null,
          "extruder1": null,
          "extruder2": null,
          "extruder3": null,
          "heater_bed": null,
          "fan": null,
          "virtual_sdcard": null,
          "print_task_config": null,
          "gcode_move": null,
          "toolhead": null
        }
      },
      "id": 1
    }
    ```
    The object list comes verbatim from
    `bambu-overlay/src/snapmaker/moonraker.rs:25-39`.
  - Pull the initial status snapshot out of the response's
    `.result.status` and seed the cache.
  - Forward incoming `notify_status_update` messages to a
    `mpsc::Sender<U1Message>` channel for PR-7b-3's parser.

- **JSON-RPC request/response correlator**: a `HashMap<u64,
  oneshot::Sender<serde_json::Value>>` keyed by request id; used
  by PR-7b-4 (send-print) and PR-7b-6 (commands) to await
  individual responses. Background tasks resolve by stripping
  matching ids off incoming messages.

- **Reconnect**: same backoff sequence as PR-7a-2 (1, 2, 4, 8,
  16s capped at 60s). On reconnect, re-issue the subscribe
  request + re-seed the snapshot.

- **Driver trait impl**:
  - `connect()`: probe → WebSocket connect → subscribe →
    return Ok. Spawn the event loop task.
  - `disconnect()`: close the WebSocket cleanly.
  - `subscribe_status()` / `status()`: same shape as Bambu.
  - `send()` / `command()`: stub `Err(DriverError::NotConnected)`
    — implemented in PR-7b-4 / PR-7b-6.

- **Configuration**:
  ```rust
  pub struct U1Config {
      pub host: String,
      pub port: u16,           // default 80
      pub serial: Option<String>,  // probe overrides
  }
  ```
  No auth fields — the status surface is unauthenticated on
  the LAN. (The mTLS pairing flow is camera-only and out of
  scope for MVP send + status.)

- Tests:
  - **`probe_extracts_serial_from_system_info`** — stub HTTP
    server, assert the returned id matches the fixture's
    `serial_number`.
  - **`subscribe_payload_matches_reference`** — assert the
    JSON-RPC subscribe request matches bambu-overlay's shape
    byte-for-byte (we expect this to be invariant; deviation
    likely means a regression).
  - **`reconnect_backoff_caps_at_60s`** — same shape as
    PR-7a-2's test.

**Effort.** ~2 days. WebSocket plumbing + JSON-RPC correlator
is the bulk.

**Dependencies.** PR-7a-1.

**Out of scope.**

- mTLS pairing flow (`bambu-overlay/src/snapmaker/pair.rs`) —
  that's the camera control plane only; not needed for send /
  status / commands. Phase 9+ if we want webcam support.
- Status interpretation — PR-7b-3.
- Send + commands — PR-7b-4, PR-7b-6.
- Multi-printer connection management — same as Bambu, single
  driver per printer, registry handles multiplexing.
