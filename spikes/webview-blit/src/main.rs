//! Strategy A — webview half. Measures, on the WebKitGTK software path
//! (WEBKIT_DISABLE_DMABUF_RENDERER=1), the cost of getting a rendered frame
//! from Rust into the webview and on screen:
//!   - `frame_bytes(n)` returns n bytes as a raw IPC Response → measures the
//!     Rust→JS per-frame transfer cost (the same channel scene_mesh_buffers uses).
//!   - JS measures `putImageData` sync cost + rAF present cadence.
//!   - JS reports every result back via `report`, which prints to stdout and
//!     exits the app on "DONE" — reliable capture (no console-forwarding games).

#[tauri::command]
fn report(line: String, app: tauri::AppHandle) {
    println!("[bench] {line}");
    use std::io::Write;
    let _ = std::io::stdout().flush();
    // Also append to a fixed file — when the app is launched into a GUI session
    // (macOS `launchctl asuser`) stdout detaches, so the file is how we capture.
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/blit-bench.log")
    {
        let _ = writeln!(f, "{line}");
    }
    if line == "DONE" {
        app.exit(0);
    }
}

/// Raw binary IPC payload of `n` bytes (resolves to an ArrayBuffer in JS).
#[tauri::command]
fn frame_bytes(n: usize) -> tauri::ipc::Response {
    tauri::ipc::Response::new(vec![0u8; n])
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![report, frame_bytes])
        // The realistic per-frame transport: a custom URI scheme (Tauri's
        // asset-serving path), fetched from JS. `frame://localhost/<n>` returns
        // a slice of a pre-allocated static buffer — so the measured time is the
        // WebKit transfer, NOT a per-call alloc+zero.
        .register_uri_scheme_protocol("frame", |_app, request| {
            static BUF: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
            let buf = BUF.get_or_init(|| vec![7u8; 64 * 1024 * 1024]);
            let n: usize = request
                .uri()
                .path()
                .trim_start_matches('/')
                .parse()
                .unwrap_or(0)
                .min(buf.len());
            tauri::http::Response::builder()
                .status(200)
                .header("Access-Control-Allow-Origin", "*")
                .header("Content-Type", "application/octet-stream")
                .body(std::borrow::Cow::Borrowed(&buf[..n]))
                .unwrap()
        })
        .run(tauri::generate_context!())
        .expect("error running tauri app");
}
