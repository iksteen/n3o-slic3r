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

/// Resolve the bundled-resources root — the directory the shipped
/// trees (`profiles/`, `plugins/`, …) live directly under. For a
/// packaged build this is Tauri's `resource_dir`; dev runs override it
/// via `$N3O_SLIC3R_RESOURCES_ROOT`.
///
/// The override exists because `tauri-build`'s copy of `bundle.resources`
/// into `target/<profile>/` runs only on tracked-file changes and never
/// prunes, so a source-tree restructure can leave stale files shadowing
/// the right ones. Dev runs pass `N3O_SLIC3R_RESOURCES_ROOT=./resources`
/// (via the npm `tauri` script) and read straight from source; production
/// never sets it and gets the bundled `resource_dir` path.
///
/// A relative override resolves against the workspace root baked in at
/// compile time (one dir above `src-tauri`), not the runtime CWD —
/// Tauri's dev mode may set CWD to `src-tauri/`, so a naive `./resources`
/// against process CWD would land in the wrong place.
fn resources_root<R: tauri::Runtime, M: tauri::Manager<R>>(mgr: &M) -> std::path::PathBuf {
    if let Some(path_os) = std::env::var_os("N3O_SLIC3R_RESOURCES_ROOT") {
        let path = std::path::PathBuf::from(&path_os);
        let resolved = if path.is_absolute() {
            path
        } else {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .expect("workspace root above src-tauri")
                .join(path)
        };
        tracing::info!(path = %resolved.display(), "using resources-root override");
        resolved
    } else {
        mgr.path().resource_dir().expect("resource_dir")
    }
}

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

            // Resolve the bundled-resources root once: the shipped trees
            // (`profiles/`, `plugins/`) live directly under it. For a
            // packaged build this is the Tauri resource dir (configured
            // via tauri.conf.json::bundle.resources); dev runs override
            // it with `N3O_SLIC3R_RESOURCES_ROOT=./resources`.
            //
            // Why the override exists: `tauri-build`'s copy of
            // `bundle.resources` into `target/<profile>/` runs only on
            // tracked-file changes and never prunes, so a source-tree
            // restructure can leave stale fragments shadowing the right
            // ones (see profile_library's same-slug collision warning
            // for the failure mode). Dev passes
            // `N3O_SLIC3R_RESOURCES_ROOT=./resources` through the npm
            // `tauri` script (`cross-env` in package.json) and reads
            // straight from source; production never sets it and gets the
            // bundled `resource_dir` path. The override is explicit so an
            // unset env in prod doesn't silently fall back to a baked-in
            // source path that won't exist on the user's machine.
            //
            // Relative override paths resolve against the workspace root
            // baked in at compile time (one dir above `src-tauri`), not
            // the binary's runtime CWD — Tauri's dev mode may set CWD to
            // `src-tauri/`, so a naive `./resources` resolved against
            // process CWD would land at the wrong place.
            let resources = resources_root(app);

            // Load the bundled profile tree.
            core::profile_library::init_from(resources.join("profiles"));
            tracing::info!("profile library loaded");

            // User-owned printer instance library. First launch
            // starts empty — the frontend shows the onboarding
            // empty-state, the add-printer wizard writes the first
            // instance. Subsequent launches load whatever's on disk.
            // Mutations (slot filament/color edits, printer add /
            // remove) persist back here.
            //
            // There is no first-launch seeding from bundled
            // fixtures. The `bundled_instances()` set in
            // `core::printer::instance_library` is a test-only
            // fallback that fires when `init_root` is never called
            // (i.e. in unit tests that don't spin up Tauri).
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

            // Project state is constructed AFTER the storage roots
            // are wired so its `Project::default()` (which eagerly
            // touches the registry to look up the default printer)
            // sees the real on-disk library, not a registry pinned
            // to whatever happened to be in the OnceLock first.
            // Same reasoning for cascade — once both roots are
            // live, the rest of the app's state managers see a
            // fully initialized backend.
            let project: Arc<Mutex<core::project::Project>> =
                Arc::new(Mutex::new(core::project::Project::default()));
            app.manage(project);
            app.manage(core::project::autosave::AutosaveHandle::new());

            // Plugin host. Two roots, bundled first then user, so a
            // user plugin overrides a bundled one of the same name:
            //   - bundled: the `plugins/` dir under the resources root
            //     (alongside `profiles/`).
            //   - user: `~/.local/share/n3o-slic3r/plugins`.
            // Loading runs each plugin's Lua top level in its sandbox;
            // a load failure keeps the plugin in the host as errored.
            let mut plugin_host = core::plugin::PluginHost::load(&[
                resources.join("plugins"),
                core::plugin::user_plugins_dir(),
            ]);
            // Seed the global tier (enable/disable + setting values) from
            // config.toml so a user's choices survive restart.
            let cfg = core::config::load();
            let settings = cfg.plugins.settings_as_strings();
            plugin_host.apply_global(cfg.plugins.enabled, settings);
            app.manage(Arc::new(Mutex::new(plugin_host)));
            tracing::info!("plugin host loaded");

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
            core::scene::commands::scene_user_override_set,
            core::scene::commands::scene_user_override_clear,
            core::scene::commands::scene_move_object,
            core::scene::commands::scene_set_plate_printer,
            core::scene::commands::scene_rebind_plate_printer,
            core::scene::commands::scene_unbind_plate_printer,
            core::scene::commands::printer_catalog,
            core::printer::printer_instance_list,
            core::printer::printer_instance_get,
            core::printer::printer_instance_set_slot_filament,
            core::printer::printer_instance_set_slot_color,
            core::printer::printer_instance_set_extruder_nozzle_diameter,
            core::printer::printer_instance_set_bed,
            core::printer::printer_instance_create,
            core::printer::printer_instance_delete,
            core::printer::printer_instance_delete_with_reassign,
            core::printer::filament_catalog_list,
            core::printer::filament_profile_list,
            core::printer::process_fragment_list,
            core::printer::printer_instance_set_quality_profile,
            core::printer::printer_instance_set_display_name,
            core::printer::printer_instance_set_ams_units,
            core::printer::printer_instance_set_connection,
            core::printer::printer_instance_update,
            core::printer::printer_instance_sync_from_driver,
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
            core::driver::commands::driver_test_connection,
            core::driver::commands::driver_unregister,
            core::driver::commands::driver_list,
            core::driver::commands::driver_connect,
            core::driver::commands::driver_disconnect,
            core::driver::commands::driver_status,
            core::driver::commands::driver_send,
            core::driver::commands::driver_send_plate,
            core::driver::commands::driver_export_plate,
            core::driver::commands::driver_command,
            core::plugin::commands::plugin_list,
            core::plugin::commands::plugin_set_enabled,
            core::plugin::commands::plugin_set_global_enabled,
            core::plugin::commands::plugin_set_global_setting,
            core::plugin::commands::plugin_reload,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
