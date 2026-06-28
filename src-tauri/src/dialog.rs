//! In-process file open/save dialogs.
//!
//! The frontend used to call `@tauri-apps/plugin-dialog`'s open/save,
//! which on Linux routes through `xdg-desktop-portal`. In the flatpak
//! sandbox that portal is unreliable — when logind marks the graphical
//! session inactive it refuses to start and every file dialog silently
//! fails to appear. Bambu Studio works on the same host because it drives
//! GTK's in-process `FileChooserDialog`, which browses the sandbox's
//! `--filesystem=home` mount directly with no portal involved. We do the
//! same here. macOS/Windows have no such indirection, so they keep rfd's
//! native dialog.
//!
//! Both commands run the blocking dialog on the GTK/AppKit main thread via
//! `run_on_main_thread` and hand the result back over a oneshot channel,
//! so the async command itself never blocks the Tokio runtime.
//!
//! Message dialogs still go through `tauri-plugin-dialog` — rfd-style
//! message boxes are in-process on every platform, so the portal problem
//! doesn't touch them.

use tauri::AppHandle;
// `Manager::webview_windows()` is used only on the Linux dialog-parent path.
#[cfg(target_os = "linux")]
use tauri::Manager;

/// A named set of extensions, matching the plugin-dialog filter shape the
/// frontend already builds (`{ name, extensions }`).
#[derive(Debug, Clone, serde::Deserialize)]
pub struct DialogFilter {
    pub name: String,
    pub extensions: Vec<String>,
}

/// Pick an existing file to open. Returns the chosen path, or `None` if
/// the user cancelled.
#[tauri::command]
pub async fn dialog_open_file(
    app: AppHandle,
    title: Option<String>,
    filters: Vec<DialogFilter>,
) -> Result<Option<String>, String> {
    pick(app, false, title, None, filters).await
}

/// Pick a destination file to save to. `default_path` seeds the filename
/// field. Returns the chosen path, or `None` if cancelled.
#[tauri::command]
pub async fn dialog_save_file(
    app: AppHandle,
    title: Option<String>,
    default_path: Option<String>,
    filters: Vec<DialogFilter>,
) -> Result<Option<String>, String> {
    pick(app, true, title, default_path, filters).await
}

async fn pick(
    app: AppHandle,
    save: bool,
    title: Option<String>,
    default_path: Option<String>,
    filters: Vec<DialogFilter>,
) -> Result<Option<String>, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.clone()
        .run_on_main_thread(move || {
            let _ = tx.send(show(&app, save, title, default_path, filters));
        })
        .map_err(|e| e.to_string())?;
    rx.await.map_err(|e| e.to_string())
}

#[cfg(target_os = "linux")]
fn show(
    app: &AppHandle,
    save: bool,
    title: Option<String>,
    default_path: Option<String>,
    filters: Vec<DialogFilter>,
) -> Option<String> {
    use gtk::prelude::*;
    use gtk::{FileChooserAction, FileChooserDialog, FileFilter, ResponseType};

    // Parent on the app's GTK window so the dialog is modal to it.
    let parent = app
        .webview_windows()
        .values()
        .next()
        .and_then(|w| w.gtk_window().ok());

    let action = if save {
        FileChooserAction::Save
    } else {
        FileChooserAction::Open
    };
    let dialog = FileChooserDialog::new(title.as_deref(), parent.as_ref(), action);
    dialog.add_button("_Cancel", ResponseType::Cancel);
    dialog.add_button(if save { "_Save" } else { "_Open" }, ResponseType::Accept);
    dialog.set_modal(true);

    for f in &filters {
        let filter = FileFilter::new();
        filter.set_name(Some(&f.name));
        for ext in &f.extensions {
            filter.add_pattern(&format!("*.{ext}"));
        }
        dialog.add_filter(filter);
    }

    if save {
        dialog.set_do_overwrite_confirmation(true);
        if let Some(name) = default_path.as_deref() {
            dialog.set_current_name(name);
        }
    }

    let chosen = if dialog.run() == ResponseType::Accept {
        dialog.filename().map(|mut path| {
            // Save dialogs don't force an extension; append the first
            // filter's first one if the typed name has none of the
            // offered extensions (so "myproject" → "myproject.n3o").
            if save {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                let lower = name.to_lowercase();
                let has_ext = filters
                    .iter()
                    .flat_map(|f| &f.extensions)
                    .any(|e| lower.ends_with(&format!(".{}", e.to_lowercase())));
                if !has_ext {
                    if let Some(ext) = filters.first().and_then(|f| f.extensions.first()) {
                        path.set_file_name(format!("{name}.{ext}"));
                    }
                }
            }
            path.to_string_lossy().into_owned()
        })
    } else {
        None
    };

    dialog.close();
    chosen
}

#[cfg(not(target_os = "linux"))]
fn show(
    _app: &AppHandle,
    save: bool,
    title: Option<String>,
    default_path: Option<String>,
    filters: Vec<DialogFilter>,
) -> Option<String> {
    let mut dialog = rfd::FileDialog::new();
    if let Some(t) = &title {
        dialog = dialog.set_title(t);
    }
    for f in &filters {
        let exts: Vec<&str> = f.extensions.iter().map(String::as_str).collect();
        dialog = dialog.add_filter(&f.name, &exts);
    }
    if let Some(name) = &default_path {
        dialog = dialog.set_file_name(name);
    }
    let path = if save {
        dialog.save_file()
    } else {
        dialog.pick_file()
    };
    path.map(|p| p.to_string_lossy().into_owned())
}
