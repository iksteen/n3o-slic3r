//! App-shell project open / new commands.
//!
//! A wholesale project replace must ALSO drop the wgpu renderer's GPU mesh
//! cache — `MeshId`s restart at 1 in each project, so a stale entry would draw
//! the previous project's geometry under a reused id. That can't live in
//! `core::project`: `core` is renderer-unaware by design (AD-8 — the renderer
//! is a one-way consumer of scene state, never the reverse). This module is the
//! app-shell seam that owns both the `Session` and the `ViewportState`, so it
//! performs the replace + the cache clear **atomically under both locks**,
//! taken ViewportState-then-Session to match `viewport_frame`'s lock order — so
//! no in-flight render can observe a cleared cache against the old project (or
//! vice versa). `core::project` keeps the pure load/build logic.

use std::path::Path;
use std::sync::{Arc, Mutex};

use tauri::{State, Window};

use crate::core::project::commands::{emit_all, fresh_project, load_or_import, Loaded};
use crate::core::project::Session;
use crate::core::scene::events::SceneEvent;
use crate::viewport_render::ViewportState;

/// Drop the renderer's GPU mesh cache and swap in `next`, atomically. Locks
/// ViewportState then Session (the render's order) so a concurrent
/// `viewport_frame` can't interleave between the clear and the swap.
fn replace_session(viewport: &ViewportState, session: &Mutex<Session>, next: Session) {
    let mut vp = viewport.0.lock().unwrap();
    let mut s = session.lock().unwrap();
    if let Some(r) = vp.as_mut() {
        r.clear_meshes();
    }
    *s = next;
}

/// Wrap a freshly-loaded project in a `Session`, seeding the runtime from the
/// load result: its save target (`source_path`) and the crash-recovery Save-As
/// hint. `Session::new` reconciles the per-plate runtime (beds derive from
/// each plate's binding).
fn session_from(loaded: Loaded) -> Session {
    let mut session = Session::new(loaded.project);
    session.runtime.source_path = loaded.source_path;
    session.runtime.recovery_origin = loaded.recovery_origin;
    session
}

/// Load a project file from `path`, **replacing** the in-memory project
/// wholesale (and dropping the renderer cache). Transparently imports a foreign
/// OrcaSlicer / Bambu Studio project. Emits `project:loaded` (+ `project:imported`
/// for a foreign import, carrying the summary); the frontend re-syncs off the
/// event.
#[tauri::command]
#[tracing::instrument(skip(window, session, viewport))]
pub fn project_load(
    path: String,
    window: Window,
    session: State<'_, Arc<Mutex<Session>>>,
    viewport: State<'_, ViewportState>,
) -> Result<(), String> {
    let loaded = load_or_import(Path::new(&path))?;
    let report = loaded.report.clone();
    replace_session(viewport.inner(), session.inner(), session_from(loaded));
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

/// Load a crash-recovery autosave from `autosave_path`, **replacing** the
/// in-memory project wholesale (dropping the renderer cache). Unlike
/// [`project_load`], the recovered project is left Untitled
/// (`source_path = None`); its `recovery_origin` — from the recovery file's
/// envelope, pointing at wherever it was saved before the crash — becomes the
/// Save-As default, so Save writes back over the original rather than the
/// stale on-disk copy or the recovery file. Emits `project:loaded`.
#[tauri::command]
#[tracing::instrument(skip(window, session, viewport))]
pub fn project_recover(
    autosave_path: String,
    window: Window,
    session: State<'_, Arc<Mutex<Session>>>,
    viewport: State<'_, ViewportState>,
) -> Result<(), String> {
    let loaded = load_or_import(Path::new(&autosave_path))?;
    let mut next = Session::new(loaded.project);
    // Untitled: no source_path. The envelope's recovery_origin is the Save-As hint.
    next.runtime.recovery_origin = loaded.recovery_origin;
    replace_session(viewport.inner(), session.inner(), next);
    // Empty path → the UI shows Untitled (recovery isn't a saved file).
    emit_all(&window, &[SceneEvent::ProjectLoaded { path: String::new() }]);
    Ok(())
}

/// Reset the in-memory project to a fresh default, **replacing** the current
/// one wholesale (and dropping the renderer cache). Emits `project:loaded` with
/// an empty path; the new project is "Untitled" (`source_path = None`).
#[tauri::command]
#[tracing::instrument(skip(window, session, viewport))]
pub fn project_new(
    window: Window,
    session: State<'_, Arc<Mutex<Session>>>,
    viewport: State<'_, ViewportState>,
) -> Result<(), String> {
    replace_session(
        viewport.inner(),
        session.inner(),
        Session::new(fresh_project()),
    );
    emit_all(&window, &[SceneEvent::ProjectLoaded { path: String::new() }]);
    Ok(())
}
