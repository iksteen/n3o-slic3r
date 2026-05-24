# PR-7b-4 — Plain `.gcode` upload (multipart) + start-print

Status: ❌ open.

**Scope.** `Driver::send()` impl for U1. Two-step Moonraker
flow: multipart POST to `/server/files/upload`, then JSON-RPC
`printer.print.start`. Bambu-overlay doesn't cover write-side;
follow Moonraker docs.

**Acceptance criteria.**

- New `core/driver/u1/upload.rs`:
  - `upload_file(host, port, file_name, body) -> Result<(), _>`
    — `multipart/form-data` POST to
    `http://<host>:<port>/server/files/upload` with one field
    `file` containing the bytes and `Content-Disposition`
    filename = `file_name`. Use `reqwest::multipart`.
  - Server returns `{ result: { item: { path: "..." } } }`;
    return the path so the start-print step can reference it.

- `U1Driver::send()`:
  - Match `SendPayload::Gcode { bytes, file_name }`.
  - Pick a unique remote name: `n3o-{plate_id}-{nanos}.gcode`.
  - Upload via the helper above.
  - Send JSON-RPC over the existing WebSocket:
    ```json
    {"jsonrpc":"2.0","method":"printer.print.start",
     "params":{"filename":"<uploaded name>"}, "id":<n>}
    ```
  - Await the JSON-RPC response via the correlator from
    PR-7b-2. Success → return `SendHandle { id, file_name }`.

- **Error mapping**:
  - HTTP upload non-2xx → `DriverError::Network(_)`.
  - HTTP upload connection failure → `DriverError::Network(_)`.
  - JSON-RPC response error → `DriverError::Protocol(_)` with
    the Moonraker error.message.
  - No response in 30s → `DriverError::Protocol("no start-print ack")`.

- Tests:
  - **`upload_uses_multipart_form_data`** — stub HTTP server,
    assert the Content-Type starts with `multipart/form-data`
    + the body contains the file bytes.
  - **`upload_returns_path_from_response`** — stub returns a
    canned response with `item.path`, assert it round-trips.
  - **`start_print_uses_uploaded_path`** — stub correlator,
    assert the JSON-RPC params contain the uploaded path.

**Effort.** ~1.5 days.

**Dependencies.** PR-7b-2 (JSON-RPC correlator + WebSocket).

**Out of scope.**

- Upload progress reporting — see PR-7a-5 rationale.
- `.gcode.3mf` wrapping for U1 — explicitly skipped per the
  PRD; U1 takes plain G-code.
