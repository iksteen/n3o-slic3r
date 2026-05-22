//! Tauri application entry point.
//!
//! Wires the backend's module tree (`core::*`) into the Tauri runtime,
//! initializes the libslic3r FFI once at startup, and registers the
//! command surface the frontend talks to. Business logic lives in
//! `core::*` — this file is just the seam between Tauri's runtime and
//! our module tree.

pub mod core;

use slic3r_ffi::init;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Initialize the tracing subscriber before anything that might emit
    // events. Tauri's own init can hit info-level events during setup.
    core::logging::init();
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "n3o-slic3r starting");

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(std::sync::Mutex::new(core::cascade::CascadeRegistry::new()))
        .manage(std::sync::Mutex::new(core::scene::SceneState::new()))
        .setup(|_app| {
            // Resources dir is only needed for STEP / font embossing; STL
            // and 3MF load without it. Log level 3 = warning, matching
            // OrcaSlicer's CLI default.
            init(None, 3).expect("libslic3r init failed");
            tracing::info!("libslic3r initialized");
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            core::cascade::slicer_info,
            core::cascade::slicer_options,
            core::cascade::commands::cascade_load,
            core::cascade::commands::cascade_resolve,
            core::cascade::commands::cascade_trace,
            core::cascade::commands::cascade_context_dimensions,
            core::scene::commands::scene_snapshot,
            core::scene::commands::scene_select,
            core::scene::commands::scene_deselect,
            core::scene::commands::scene_load_mesh,
            core::scene::commands::scene_object_translate,
            core::scene::commands::scene_object_rotate,
            core::scene::commands::scene_object_scale,
            core::scene::commands::scene_object_set_transform,
            core::scene::commands::scene_object_delete,
            core::scene::commands::scene_object_duplicate,
            core::scene::commands::scene_gizmo_set,
            core::scene::commands::scene_camera_set,
            core::slice::slicer_slice,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
