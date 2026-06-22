//! Spike: pack a `GtkGLArea` into Tauri's Linux GTK window, alongside the
//! WebKitWebView, and clear it ORANGE with GL. If we see orange GL content +
//! the green webview, both stable, then GTK-composited GPU content coexists
//! with the webview — the thing the raw wgpu swapchain (Strategy B) could not
//! do — which means a wgpu→GL-texture present path avoids BOTH the Strategy-B
//! surface crash AND the Strategy-A ~330 MB/s webview transport.

use tauri::Manager;

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let win = app
                .get_webview_window("main")
                .expect("default window 'main' from tauri.conf.json");

            #[cfg(target_os = "linux")]
            install_glarea(&win);

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error running tauri app");
}

#[cfg(target_os = "linux")]
fn install_glarea(win: &tauri::WebviewWindow) {
    use gtk::prelude::*;

    // Load GL entry points through libepoxy (what GtkGLArea uses internally).
    // Resolving epoxy's dispatch symbols needs only the library, not a current
    // context (epoxy binds the real GL call per-invocation).
    static LOAD: std::sync::Once = std::sync::Once::new();
    LOAD.call_once(|| {
        let lib = unsafe { libloading::Library::new("libepoxy.so.0") }
            .expect("libepoxy.so.0");
        epoxy::load_with(|name| unsafe {
            lib.get::<unsafe extern "C" fn()>(name.as_bytes())
                .map(|s| *s as *const std::ffi::c_void)
                .unwrap_or(std::ptr::null())
        });
        // Intentionally leak the handle so the function pointers stay valid.
        std::mem::forget(lib);
    });

    // The GtkBox Tauri packs the webview into. Add the GLArea as a sibling so
    // GTK lays out + composites both — no reparenting of the webview needed.
    let vbox: gtk::Box = win.default_vbox().expect("default_vbox (Linux)");

    let gl = gtk::GLArea::new();
    gl.set_size_request(-1, 320); // force a visible band
    gl.set_has_depth_buffer(false);
    gl.set_has_stencil_buffer(false);

    gl.connect_render(|_area, _ctx| {
        unsafe {
            epoxy::ClearColor(1.0, 0.5, 0.0, 1.0); // orange
            epoxy::Clear(epoxy::COLOR_BUFFER_BIT);
        }
        eprintln!("[poc] GLArea render fired (orange clear)");
        gtk::glib::Propagation::Stop
    });

    // Put the GL band on TOP of the webview in the vertical box.
    vbox.pack_start(&gl, false, false, 0);
    vbox.reorder_child(&gl, 0);
    gl.show();
    eprintln!("[poc] GtkGLArea packed into Tauri's default_vbox");
}
