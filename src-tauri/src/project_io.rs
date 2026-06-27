//! App-shell project open / new commands.
//!
//! A wholesale project replace must ALSO drop the wgpu renderer's GPU mesh
//! cache — `MeshId`s restart at 1 in each project, so a stale entry would draw
//! the previous project's geometry under a reused id. That can't live in
//! `core::project`: `core` is renderer-unaware by design (AD-8 — the renderer
//! is a one-way consumer of scene state, never the reverse). This module is the
//! app-shell seam that owns both the `Project` and the `ViewportState`, so it
//! performs the replace + the cache clear **atomically under both locks**,
//! taken ViewportState-then-Project to match `viewport_frame`'s lock order — so
//! no in-flight render can observe a cleared cache against the old project (or
//! vice versa). `core::project` keeps the pure load/build logic.

use std::path::Path;
use std::sync::{Arc, Mutex};

use tauri::{State, Window};

use crate::core::project::commands::{emit_all, fresh_project, load_or_import};
use crate::core::project::Project;
use crate::core::scene::events::SceneEvent;
use crate::viewport_render::ViewportState;

/// Drop the renderer's GPU mesh cache and swap in `next`, atomically. Locks
/// ViewportState then Project (the render's order) so a concurrent
/// `viewport_frame` can't interleave between the clear and the swap.
fn replace_project(viewport: &ViewportState, project: &Mutex<Project>, next: Project) {
    let mut vp = viewport.0.lock().unwrap();
    let mut p = project.lock().unwrap();
    if let Some(r) = vp.as_mut() {
        r.clear_meshes();
    }
    *p = next;
}

/// Load a project file from `path`, **replacing** the in-memory project
/// wholesale (and dropping the renderer cache). Transparently imports a foreign
/// OrcaSlicer / Bambu Studio project. Emits `project:loaded` (+ `project:imported`
/// for a foreign import, carrying the summary); the frontend re-syncs off the
/// event.
#[tauri::command]
#[tracing::instrument(skip(window, project, viewport))]
pub fn project_load(
    path: String,
    window: Window,
    project: State<'_, Arc<Mutex<Project>>>,
    viewport: State<'_, ViewportState>,
) -> Result<(), String> {
    let (loaded, report) = load_or_import(Path::new(&path))?;
    replace_project(viewport.inner(), project.inner(), loaded);
    // ProjectLoaded first (scene re-syncs), then the import report if any.
    let mut events = vec![SceneEvent::ProjectLoaded { path: path.clone() }];
    if let Some(report) = report {
        events.push(SceneEvent::ProjectImported {
            path: path.clone(),
            report,
        });
    }
    emit_all(&window, &events);
    Ok(())
}

/// Reset the in-memory project to a fresh default, **replacing** the current
/// one wholesale (and dropping the renderer cache). Emits `project:loaded` with
/// an empty path; the new project is "Untitled" (`source_path = None`).
#[tauri::command]
#[tracing::instrument(skip(window, project, viewport))]
pub fn project_new(
    window: Window,
    project: State<'_, Arc<Mutex<Project>>>,
    viewport: State<'_, ViewportState>,
) -> Result<(), String> {
    replace_project(viewport.inner(), project.inner(), fresh_project());
    emit_all(&window, &[SceneEvent::ProjectLoaded { path: String::new() }]);
    Ok(())
}
