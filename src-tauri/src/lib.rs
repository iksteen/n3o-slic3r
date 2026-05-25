//! Tauri application entry point.
//!
//! Wires the backend's module tree (`core::*`) into the Tauri runtime,
//! initializes the libslic3r FFI once at startup, and registers the
//! command surface the frontend talks to. Business logic lives in
//! `core::*` — this file is just the seam between Tauri's runtime and
//! our module tree.

pub mod core;

use std::sync::{Arc, Mutex};

use slic3r_ffi::init;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Initialize the tracing subscriber before anything that might emit
    // events. Tauri's own init can hit info-level events during setup.
    core::logging::init();
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "n3o-slic3r starting");

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(std::sync::Mutex::new(core::cascade::CascadeRegistry::new()))
        .manage(core::slice::JobRegistry::new())
        .manage(Arc::new(core::preview::PreviewRegistry::new()))
        .manage(Arc::new(core::driver::DriverRegistry::new()))
        .setup(|app| {
            use tauri::Manager;

            // Resources dir is only needed for STEP / font embossing; STL
            // and 3MF load without it. Log level 3 = warning, matching
            // OrcaSlicer's CLI default.
            init(None, 3).expect("libslic3r init failed");
            tracing::info!("libslic3r initialized");

            // Load the bundled profile tree from the Tauri resource dir
            // (configured via tauri.conf.json::bundle.resources). The
            // ProfileLibrary lazy fallback (workspace path baked in at
            // compile time) is only meaningful for tests; a packaged
            // binary needs this explicit init or it'd panic on the
            // first cascade compose.
            let resource_root = app
                .path()
                .resource_dir()
                .expect("resource_dir")
                .join("profiles/vendor");
            core::profile_library::init_from(resource_root);
            tracing::info!("profile library loaded");

            // User-owned printer instance library. Seeded from the
            // bundled fixtures on first launch; subsequent launches
            // load whatever's on disk. Mutations (slot filament/color
            // edits, future printer add/remove) persist back here.
            //
            // `config_dir()` is the platform base (`~/.config/` on
            // Linux, `~/Library/Application Support/` on macOS,
            // `%APPDATA%/` on Windows); we append our own name rather
            // than reusing `app_config_dir` because that suffixes
            // with Tauri's reverse-DNS identifier
            // (`org.thegraveyard.n3o-slic3r`), which is the right
            // thing for the bundle but reads ugly when a user goes
            // looking through their config files.
            let printers_root = app
                .path()
                .config_dir()
                .expect("config_dir")
                .join("n3o-slic3r/printers");
            core::printer::instance_storage::init_root(printers_root);
            tracing::info!("printer instance library initialized");

            // Project state is constructed AFTER the storage roots are
            // wired so its `Project::default()` (which auto-binds a
            // printer instance, eagerly touching the registry) can
            // load from the on-disk library instead of seeding the
            // OnceLock with the in-memory bundled fixtures. Same
            // reasoning for cascade — once both roots are live, the
            // rest of the app's state managers see a fully initialized
            // backend.
            let project: Arc<Mutex<core::project::Project>> =
                Arc::new(Mutex::new(core::project::Project::default()));
            app.manage(project);
            app.manage(core::project::autosave::AutosaveHandle::new());

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            core::cascade::slicer_info,
            core::cascade::slicer_options,
            core::cascade::slicer_options_for_printer,
            core::cascade::commands::cascade_load,
            core::cascade::commands::cascade_resolve,
            core::cascade::commands::cascade_trace,
            core::cascade::commands::cascade_context_dimensions,
            core::scene::commands::scene_snapshot,
            core::scene::commands::scene_mesh_buffers,
            core::scene::commands::scene_select,
            core::scene::commands::scene_deselect,
            core::scene::commands::scene_load_mesh_from_path,
            core::scene::commands::scene_load_3mf,
            core::scene::commands::scene_object_translate,
            core::scene::commands::scene_object_rotate,
            core::scene::commands::scene_object_scale,
            core::scene::commands::scene_object_set_transform,
            core::scene::commands::scene_object_delete,
            core::scene::commands::scene_object_duplicate,
            core::scene::commands::scene_object_mirror,
            core::scene::commands::scene_object_lay_flat,
            core::scene::commands::scene_gizmo_set,
            core::scene::commands::scene_camera_set,
            core::scene::commands::scene_set_active_printer,
            core::scene::commands::scene_load_default_printer,
            core::scene::commands::scene_add_plate,
            core::scene::commands::scene_remove_plate,
            core::scene::commands::scene_set_active_plate,
            core::scene::commands::scene_rename_plate,
            core::scene::commands::scene_object_override_set,
            core::scene::commands::scene_object_override_clear,
            core::scene::commands::scene_object_override_clear_all,
            core::scene::commands::scene_project_override_set,
            core::scene::commands::scene_project_override_clear,
            core::scene::commands::scene_project_override_clear_all,
            core::scene::commands::scene_move_object,
            core::scene::commands::scene_set_plate_printer,
            core::scene::commands::scene_rebind_plate_printer,
            core::scene::commands::printer_catalog,
            core::printer::printer_instance_list,
            core::printer::printer_instance_get,
            core::printer::printer_instance_set_slot_filament,
            core::printer::printer_instance_set_slot_color,
            core::printer::printer_instance_set_bed,
            core::printer::filament_catalog_list,
            core::printer::filament_profile_list,
            core::project::commands::project_set_plate_composition_order,
            core::project::commands::project_set_material_slot,
            core::project::commands::project_clear_material_slot,
            core::project::commands::project_save,
            core::project::commands::project_save_as,
            core::project::commands::project_load,
            core::project::commands::project_autosave_enable,
            core::project::commands::project_autosave_disable,
            core::project::commands::project_autosave_list,
            core::project::commands::project_autosave_drop,
            core::scene::commands::library_primitives,
            core::scene::commands::library_calibration,
            core::scene::commands::library_imported,
            core::scene::commands::scene_object_add_from_primitive,
            core::scene::commands::scene_auto_arrange,
            core::slice::slicer_slice,
            core::slice::commands::slice_start_job,
            core::slice::commands::slice_active_plate,
            core::slice::commands::slice_cancel,
            core::slice::commands::slice_status,
            core::preview::commands::preview_load,
            core::preview::commands::preview_load_gcode_3mf,
            core::preview::commands::preview_buffers,
            core::preview::commands::preview_layer_stats,
            core::preview::commands::preview_segment_detail,
            core::preview::commands::preview_drop,
            core::driver::commands::driver_register,
            core::driver::commands::driver_unregister,
            core::driver::commands::driver_list,
            core::driver::commands::driver_connect,
            core::driver::commands::driver_disconnect,
            core::driver::commands::driver_status,
            core::driver::commands::driver_send,
            core::driver::commands::driver_dry_send,
            core::driver::commands::driver_send_plate,
            core::driver::commands::driver_dry_send_plate,
            core::driver::commands::driver_export_plate,
            core::driver::commands::driver_command,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
