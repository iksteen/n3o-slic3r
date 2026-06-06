# PR-7a-5 — `.gcode.3mf` FTPS upload + send-print MQTT command

Status: ❌ open.

**Scope.** Implement the `Driver::send()` path for Bambu:
upload the `.gcode.3mf` bundle via FTPS, then publish the MQTT
command that tells the printer to start the print. Two halves
of one operation; both gated behind one `SendPayload::Gcode3mf`
match in the driver.

bambu-overlay covers the FTPS connection shape (`thumbnail/local.rs:132-160`)
but as a download. **The upload + print-trigger MQTT command
shape is not in bambu-overlay** — source it from BambuStudio's
`src/slic3r/Utils/PrintHost.cpp`.

**Acceptance criteria.**

- New module `core/driver/bambu/ftps.rs`:
  - `connect_ftps(host, access_code) -> Result<FtpStream, _>`
    mirroring bambu-overlay's `connect_local_ftps`:
    - `suppaftp::NativeTlsFtpStream::connect_secure_implicit(host:990, …)`
    - login `"bblp"` / access code
    - `set_passive_nat_workaround(true)`
    - `transfer_type(FileType::Binary)`
  - `upload(stream, remote_path: &str, body: &[u8]) -> Result<(), _>`
    — `PUT` against `Metadata/<name>.gcode.3mf` (Bambu's
    expected directory).

- New `BambuDriver::send()` impl:
  - Match `SendPayload::Gcode3mf { bytes, plate_id }`.
  - Pick a remote file name: `n3o-{plate_id}-{nanos}.gcode.3mf`
    so concurrent sends don't collide.
  - FTPS connect → upload → close.
  - Publish to `device/<DEVICE_ID>/request`:
    ```json
    {
      "print": {
        "sequence_id": "<incrementing>",
        "command": "project_file",
        "param": "Metadata/plate_<plate_id>.gcode",
        "subtask_name": "<file_name without extension>",
        "url": "ftp://Metadata/<remote_file_name>",
        "bed_type": "<auto|cool|textured_pei|engineering>",
        "use_ams": <true|false>,
        "timelapse": false,
        "flow_cali": false,
        "bed_leveling": true,
        "vibration_cali": false,
        "layer_inspect": false
      }
    }
    ```
    Field mapping comes from the BambuStudio source (cite the
    file path in a code comment). The `bed_type` value matches
    whatever the user selected in the plate's BuildPlate
    binding; default `auto` lets the printer choose.
  - Return a `SendHandle { id, file_name }` so the UI can
    correlate the running job in subsequent status updates.

- **Sequence-id tracking**: maintain a monotonic counter inside
  `BambuDriver` for `sequence_id`. Print commands echo it back
  in status messages; future tickets (PR-7a-6 commands) reuse
  the counter.

- **Error mapping**:
  - FTPS connect/login fails → `DriverError::Auth(_)`.
  - FTPS upload fails mid-stream → `DriverError::Network(_)`.
  - MQTT publish times out (5s) → `DriverError::Network(_)`.
  - Status doesn't report job start within 30s of the MQTT
    publish → `DriverError::Protocol("printer did not acknowledge print start")`.

- Tests:
  - **`upload_path_uses_metadata_prefix`** — unit, stub FTPS
    client, assert PUT was called against
    `Metadata/n3o-1-…gcode.3mf`.
  - **`send_publishes_correct_mqtt_command`** — unit, stub MQTT
    client, assert published payload matches the expected JSON
    shape (modulo `sequence_id`).
  - **`send_returns_error_on_no_print_ack`** — drive with a
    status stream that stays IDLE, assert
    `DriverError::Protocol`.
  - **No real-printer test here** — PR-7a-8's smoke covers it
    end-to-end.

**Effort.** ~2 days. The FTPS half is mostly a copy of
bambu-overlay's connection setup; the MQTT command shape needs
careful sourcing from BambuStudio.

**Dependencies.** PR-7a-1 (`SendPayload`, `SendHandle`,
`DriverError`), PR-7a-2 (MQTT publish channel + connection),
PR-7a-3 (status stream for ack detection), PR-3-10 (the
`.gcode.3mf` writer the bytes come from).

**Out of scope.**

- AMS bindings inside the `.gcode.3mf` — populated by PR-7c-7
  (sync-on-send). This ticket sends whatever bytes the caller
  hands over, including the bindings PR-3-10 + PR-7c-7 will
  pack.
- Upload progress reporting — the FTPS upload is fast enough
  (<10MB / few seconds) that a single "uploading" status edge
  is sufficient. Streaming progress is Phase 9 polish.
- Cancel-during-upload — single-shot send for MVP.
