// Tauri commands bridging the renderer to libslic3r via orca-slicer-ffi.
//
// init runs once at app startup; the FFI crate's Once guard means re-calls are
// no-ops. For v0 we expose introspection + one-shot slicing; persistent
// Config/Model handles can come later as a state-owning struct in
// tauri::State.

use serde::Serialize;
use slic3r_ffi::{init, option_defs, slice, version, Config, Model};
use std::path::PathBuf;

#[derive(Serialize)]
struct SlicerInfo {
    version: String,
    option_count: usize,
}

#[tauri::command]
fn slicer_info() -> SlicerInfo {
    SlicerInfo {
        version: version(),
        option_count: option_defs().len(),
    }
}

#[derive(Serialize)]
struct OptionSummary {
    key: String,
    ty: String,
    label: Option<String>,
    category: Option<String>,
    default_value: Option<String>,
}

#[tauri::command]
fn slicer_options(filter: Option<String>) -> Vec<OptionSummary> {
    let needle = filter.unwrap_or_default().to_lowercase();
    option_defs()
        .into_iter()
        .filter(|d| {
            if needle.is_empty() {
                true
            } else {
                d.key.to_lowercase().contains(&needle)
                    || d.label.as_deref().map_or(false, |s| s.to_lowercase().contains(&needle))
            }
        })
        .map(|d| OptionSummary {
            key: d.key,
            ty: format!("{:?}", d.ty),
            label: d.label,
            category: d.category,
            default_value: d.default_serialized,
        })
        .collect()
}

#[derive(Serialize)]
struct SliceResult {
    ok: bool,
    out_path: String,
    error: Option<String>,
}

#[tauri::command]
fn slicer_slice(model_path: String, out_path: String) -> SliceResult {
    // Construct model + default config inline; v0 doesn't persist them.
    let do_it = || -> Result<(), slic3r_ffi::Error> {
        let mut model = Model::new()?;
        model.load(PathBuf::from(&model_path))?;
        let config = Config::new()?;
        slice(&model, &config, PathBuf::from(&out_path))?;
        Ok(())
    };
    match do_it() {
        Ok(()) => SliceResult { ok: true, out_path, error: None },
        Err(e) => SliceResult { ok: false, out_path, error: Some(format!("{e}")) },
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|_app| {
            // Resources dir is only needed for STEP / font embossing; STL & 3MF
            // load without it. Log level 3 = warning (matches OrcaSlicer's CLI
            // default).
            init(None, 3).expect("libslic3r init failed");
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            slicer_info,
            slicer_options,
            slicer_slice,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
