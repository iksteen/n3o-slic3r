# U1 Moonraker fixtures

Captured from a live Snapmaker U1 over the Moonraker WebSocket
(`ws://<host>:<port>/websocket`) during PR-7b-3 fixture capture.

Consumed by `src/core/driver/snapmaker/status.rs` fixture tests
(search `FIXTURES_DIR`).

## Files

| File | What it is | What it tests |
|---|---|---|
| `subscribe_response.json` | Full `printer.objects.subscribe` reply | Decoder handles a complete real-world snapshot — every object the U1 reports, including ones the inline `json!` tests skip (`gcode_move`, `virtual_sdcard`, full `print_task_config` key set). |
| `notify_layer_advance.json` | `notify_status_update` mid-print at layer 33/100 | Layer count surfaces through a delta merge against the subscribe baseline (delta carries only `current_layer`, baseline supplies `state` + `total_layer`). |
| `notify_toolchange.json` | `notify_status_update` with `toolhead.extruder = "extruder1"` | Mounted-toolhead decode after a delta merge that touches the `toolhead` object. |

## Capture state

The `subscribe_response.json` was captured immediately after a
finished print, so `print_stats.state = "complete"`. The
`subscribe_response_idle.json` name from the original ticket spec
was renamed for honesty — a "complete"-state snapshot is in fact a
*better* fixture than a fresh-boot idle one because it exercises
the populated-`print_task_config` path (filament_color_rgba,
filament_type arrays) the decoder cares about most.

## Re-capturing

If the Snapmaker firmware adds new fields and a fixture needs
refreshing: temporarily re-add the `dump_frame()` call in
`src/core/driver/snapmaker/moonraker.rs`'s receive loops (it
appends every WS text frame to `/tmp/n3o-u1-moonraker-raw.jsonl`),
run a print, then cherry-pick frames with the
`tmp/n3o-make-2cube.py`-adjacent extractor script. The original
captures used a 2-material 2-cube print to force per-layer
toolchanges.
