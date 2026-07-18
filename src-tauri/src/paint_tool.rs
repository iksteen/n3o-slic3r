//! Support-paint viewport tool (manual tree/normal supports).
//!
//! Interactive enforcer/blocker painting over one object's mesh, backed by a
//! libslic3r `TriangleSelector` inside [`slic3r_ffi::PaintSession`] (sub-triangle
//! splitting, exact Orca semantics — no brush geometry reimplemented here).
//!
//! Unlike the cut tool (fully frontend-state + stateless commands), painting is
//! inherently stateful: a live selector accumulates strokes. So the session +
//! its mesh live in [`PaintToolState`] for the tool's lifetime; each command
//! raycasts the pointer against the active object, drives one selector op,
//! commits the serialized paint onto the mesh live, and rebuilds the renderer's
//! world-space facet overlay.

use std::sync::{Arc, Mutex};

use glam::{Mat4, Vec3};
use serde::{Deserialize, Serialize};
use slic3r_ffi::{BrushKind, PaintSession, PaintState};
use tauri::{State, Window};

use crate::core::project::Session;
use crate::core::scene::commands::emit_all;
use crate::core::scene::events::SceneEvent;
use crate::core::scene::state::{MeshId, ObjectId};
use crate::viewport_gpu::{cam_eye, cursor_ray};
use crate::viewport_render::{clear_paint_overlay, ray_tri, store_paint_overlay, ViewportState};

/// The open paint session over one object. Holds the mesh buffers (`Arc`-shared,
/// so strokes don't re-lock the project for geometry) + the object→world matrix
/// and its inverse, so world raycasts map to the mesh-local coordinates the
/// selector speaks.
pub struct ActivePaint {
    object: u64,
    /// The object's mesh. Painting mutates this mesh's `support_paint` in place
    /// (same id, geometry untouched), so the slice path sees each stroke live
    /// and the session stays valid across the whole edit.
    mesh_id: MeshId,
    vertices: Arc<Vec<f32>>,
    indices: Arc<Vec<u32>>,
    /// Object→world (mesh-local → world). `trafo16` is the same, column-major f64
    /// for the FFI.
    model: Mat4,
    inv_model: Mat4,
    trafo16: [f64; 16],
    session: PaintSession,
}

/// Tauri-managed paint tool state — `Some` while the tool is open.
#[derive(Default)]
pub struct PaintToolState(pub Mutex<Option<ActivePaint>>);

fn brush_from_u32(b: u32) -> BrushKind {
    if b == 1 {
        BrushKind::Sphere
    } else {
        BrushKind::Circle
    }
}

fn state_from_u32(s: u32) -> PaintState {
    match s {
        1 => PaintState::Enforcer,
        2 => PaintState::Blocker,
        _ => PaintState::None,
    }
}

/// Camera + cursor for a paint raycast (mirrors the cut requests' camera block).
#[derive(Deserialize)]
pub struct PaintStrokeRequest {
    pub width: u32,
    pub height: u32,
    pub x: f32,
    pub y: f32,
    pub az: f32,
    pub el: f32,
    pub dist: f32,
    pub center: [f32; 3],
    /// Brush radius, world mm.
    pub radius: f32,
    /// 0 = circle (screen-projected), 1 = sphere.
    pub brush: u32,
    /// 0 = erase, 1 = enforce, 2 = block.
    pub state: u32,
    /// `true` on the first sample of a drag → one undo step per drag.
    pub new_stroke: bool,
}

#[derive(Deserialize)]
pub struct PaintFillRequest {
    pub width: u32,
    pub height: u32,
    pub x: f32,
    pub y: f32,
    pub az: f32,
    pub el: f32,
    pub dist: f32,
    pub center: [f32; 3],
    /// Smart-fill angle bound, degrees.
    pub angle: f32,
    pub state: u32,
}

/// Whether the object currently carries any enforcer / blocker paint. Drives the
/// panel's "enable manual/auto support" prompts.
#[derive(Serialize)]
pub struct PaintFlags {
    pub enforce: bool,
    pub block: bool,
}

/// A brush/fill result: whether the cursor hit the mesh (`false` = missed, so the
/// frontend drives the camera) plus the post-stroke paint flags.
#[derive(Serialize)]
pub struct PaintOutcome {
    pub hit: bool,
    pub enforce: bool,
    pub block: bool,
}

/// Raycast `(ro, rd)` against the active mesh (world space). Returns the nearest
/// triangle's index and the world hit point.
fn raycast(ap: &ActivePaint, ro: Vec3, rd: Vec3) -> Option<(i32, Vec3)> {
    let v = &ap.vertices;
    let wp = |i: u32| {
        let o = i as usize * 3;
        ap.model
            .transform_point3(Vec3::new(v[o], v[o + 1], v[o + 2]))
    };
    let mut best: Option<(f32, i32)> = None;
    for (ti, t3) in ap.indices.chunks_exact(3).enumerate() {
        let (a, b, c) = (wp(t3[0]), wp(t3[1]), wp(t3[2]));
        if let Some(t) = ray_tri(ro, rd, a, b, c) {
            if best.map_or(true, |(bt, _)| t < bt) {
                best = Some((t, ti as i32));
            }
        }
    }
    best.map(|(t, ti)| (ti, ro + rd * t))
}

/// Expand a mesh-local indexed facet set into a world-space non-indexed triangle
/// soup (6 floats/vertex: pos + flat normal). Left exactly coplanar with the
/// mesh; the renderer's depth-biased overlay pipeline keeps it from z-fighting
/// (a geometric normal-offset can't, since imported meshes have no winding
/// guarantee and the flat normal may point inward).
fn world_soup(verts: &[f32], indices: &[u32], model: Mat4) -> Vec<f32> {
    let mut out = Vec::with_capacity(indices.len() * 6);
    let wp = |i: u32| {
        let o = i as usize * 3;
        model.transform_point3(Vec3::new(verts[o], verts[o + 1], verts[o + 2]))
    };
    for t in indices.chunks_exact(3) {
        let (a, b, c) = (wp(t[0]), wp(t[1]), wp(t[2]));
        let n = (b - a).cross(c - a).normalize_or_zero();
        for p in [a, b, c] {
            out.extend_from_slice(&[p.x, p.y, p.z, n.x, n.y, n.z]);
        }
    }
    out
}

/// Write the session's current paint onto the object's mesh in place (same
/// `MeshId`, geometry untouched) so the slice path picks it up immediately — no
/// separate apply step. Stores `None` when nothing is painted.
fn commit_live(ap: &ActivePaint, session: &Mutex<Session>) -> Result<(), String> {
    let hex = ap.session.serialize().map_err(|e| e.to_string())?;
    let support = (!hex.iter().all(String::is_empty)).then(|| Arc::new(hex));
    let mut s = session.lock().map_err(|e| format!("session lock: {e}"))?;
    if let Some(m) = s.project.meshes.get_mut(&ap.mesh_id) {
        m.support_paint = support;
    }
    Ok(())
}

/// Rebuild the renderer overlay from the session's current enforcer/blocker
/// facets, returning whether each is non-empty. O(mesh) — the frontend throttles
/// strokes with an in-flight flag.
fn rebuild_overlay(ap: &ActivePaint, viewport: &ViewportState) -> Result<(bool, bool), String> {
    let (ev, ei) = ap.session.facets(PaintState::Enforcer).map_err(|e| e.to_string())?;
    let (bv, bi) = ap.session.facets(PaintState::Blocker).map_err(|e| e.to_string())?;
    let (has_enf, has_blk) = (!ei.is_empty(), !bi.is_empty());
    let enf = world_soup(&ev, &ei, ap.model);
    let blk = world_soup(&bv, &bi, ap.model);
    store_paint_overlay(viewport, &enf, &blk);
    Ok((has_enf, has_blk))
}

/// Open a paint session over `object_id` on the active plate, seeding it with the
/// object's existing support paint. Shows the current paint in the overlay.
#[tauri::command]
pub fn paint_open(
    object_id: u64,
    session: State<'_, Arc<Mutex<Session>>>,
    tool: State<'_, PaintToolState>,
    viewport: State<'_, ViewportState>,
) -> Result<PaintFlags, String> {
    let s = session.lock().map_err(|e| format!("session lock: {e}"))?;
    let obj = s
        .project
        .active_plate()
        .scene
        .objects
        .get(&ObjectId(object_id))
        .ok_or("paint_open: unknown object")?;
    let mesh = s
        .project
        .meshes
        .get(&obj.mesh)
        .ok_or("paint_open: object has no mesh")?;
    let seed: &[String] = mesh
        .support_paint
        .as_deref()
        .map(|v| v.as_slice())
        .unwrap_or(&[]);
    let session =
        PaintSession::new(&mesh.vertices, &mesh.indices, seed).map_err(|e| e.to_string())?;
    let model = obj.transform.to_mat4();
    let trafo16 = model.to_cols_array().map(f64::from);
    let ap = ActivePaint {
        object: object_id,
        mesh_id: obj.mesh,
        vertices: Arc::clone(&mesh.vertices),
        indices: Arc::clone(&mesh.indices),
        model,
        inv_model: model.inverse(),
        trafo16,
        session,
    };
    drop(s);
    let (enforce, block) = rebuild_overlay(&ap, &viewport)?;
    *tool.0.lock().map_err(|e| format!("paint lock: {e}"))? = Some(ap);
    Ok(PaintFlags { enforce, block })
}

/// Apply one brush sample: raycast the pointer, drive the selector, commit + rebuild
/// the overlay, and report whether the mesh was hit + the resulting paint flags.
#[tauri::command]
pub fn paint_stroke(
    req: PaintStrokeRequest,
    session: State<'_, Arc<Mutex<Session>>>,
    tool: State<'_, PaintToolState>,
    viewport: State<'_, ViewportState>,
) -> Result<PaintOutcome, String> {
    let mut guard = tool.0.lock().map_err(|e| format!("paint lock: {e}"))?;
    let ap = guard.as_mut().ok_or("paint_stroke: no open session")?;
    let (w, h) = (req.width.max(1) as f32, req.height.max(1) as f32);
    let (ro, rd) = cursor_ray(w, h, req.x, req.y, req.az, req.el, req.dist, Vec3::from(req.center));
    let Some((facet, world_hit)) = raycast(ap, ro, rd) else {
        return Ok(PaintOutcome { hit: false, enforce: false, block: false });
    };
    let hit_local = ap.inv_model.transform_point3(world_hit).to_array();
    let eye = cam_eye(req.az, req.el, req.dist, Vec3::from(req.center));
    let cam_local = ap.inv_model.transform_point3(eye).to_array();
    ap.session
        .stroke(
            facet,
            hit_local,
            cam_local,
            &ap.trafo16,
            req.radius,
            brush_from_u32(req.brush),
            state_from_u32(req.state),
            req.new_stroke,
        )
        .map_err(|e| e.to_string())?;
    commit_live(ap, session.inner())?;
    let (enforce, block) = rebuild_overlay(ap, &viewport)?;
    Ok(PaintOutcome { hit: true, enforce, block })
}

/// Smart-fill from the pointer: flood the angle-bounded connected region and
/// paint it `state`. Reports whether the mesh was hit + the resulting paint flags.
#[tauri::command]
pub fn paint_fill(
    req: PaintFillRequest,
    session: State<'_, Arc<Mutex<Session>>>,
    tool: State<'_, PaintToolState>,
    viewport: State<'_, ViewportState>,
) -> Result<PaintOutcome, String> {
    let mut guard = tool.0.lock().map_err(|e| format!("paint lock: {e}"))?;
    let ap = guard.as_mut().ok_or("paint_fill: no open session")?;
    let (w, h) = (req.width.max(1) as f32, req.height.max(1) as f32);
    let (ro, rd) = cursor_ray(w, h, req.x, req.y, req.az, req.el, req.dist, Vec3::from(req.center));
    let Some((facet, world_hit)) = raycast(ap, ro, rd) else {
        return Ok(PaintOutcome { hit: false, enforce: false, block: false });
    };
    let hit_local = ap.inv_model.transform_point3(world_hit).to_array();
    ap.session
        .fill(
            facet,
            hit_local,
            &ap.trafo16,
            req.angle,
            state_from_u32(req.state),
            true,
        )
        .map_err(|e| e.to_string())?;
    commit_live(ap, session.inner())?;
    let (enforce, block) = rebuild_overlay(ap, &viewport)?;
    Ok(PaintOutcome { hit: true, enforce, block })
}

/// Undo the last stroke/fill, returning the resulting paint flags.
#[tauri::command]
pub fn paint_undo(
    session: State<'_, Arc<Mutex<Session>>>,
    tool: State<'_, PaintToolState>,
    viewport: State<'_, ViewportState>,
) -> Result<PaintFlags, String> {
    let mut guard = tool.0.lock().map_err(|e| format!("paint lock: {e}"))?;
    let ap = guard.as_mut().ok_or("paint_undo: no open session")?;
    if ap.session.undo() {
        commit_live(ap, session.inner())?;
    }
    let (enforce, block) = rebuild_overlay(ap, &viewport)?;
    Ok(PaintFlags { enforce, block })
}

/// Erase all paint on the object (keeps the tool open) — rebuilds an empty
/// session over the same mesh and clears the committed support paint.
#[tauri::command]
pub fn paint_clear(
    session: State<'_, Arc<Mutex<Session>>>,
    tool: State<'_, PaintToolState>,
    viewport: State<'_, ViewportState>,
) -> Result<PaintFlags, String> {
    let mut guard = tool.0.lock().map_err(|e| format!("paint lock: {e}"))?;
    let ap = guard.as_mut().ok_or("paint_clear: no open session")?;
    let verts = Arc::clone(&ap.vertices);
    let idx = Arc::clone(&ap.indices);
    ap.session = PaintSession::new(&verts, &idx, &[]).map_err(|e| e.to_string())?;
    commit_live(ap, session.inner())?;
    let (enforce, block) = rebuild_overlay(ap, &viewport)?;
    Ok(PaintFlags { enforce, block })
}

/// Close the tool. The paint is already live on the mesh (applied per stroke), so
/// this just drops the session + overlay and emits one `ObjectUpdated` — folding
/// the whole edit into a single undo step and marking the project dirty for save.
#[tauri::command]
pub fn paint_close(
    session: State<'_, Arc<Mutex<Session>>>,
    tool: State<'_, PaintToolState>,
    viewport: State<'_, ViewportState>,
    window: Window,
) -> Result<(), String> {
    let ap = tool
        .0
        .lock()
        .map_err(|e| format!("paint lock: {e}"))?
        .take();
    clear_paint_overlay(&viewport);
    let Some(ap) = ap else {
        return Ok(());
    };
    let s = session.lock().map_err(|e| format!("session lock: {e}"))?;
    let plate = s.project.active_plate();
    if let Some(obj) = plate.scene.objects.get(&ObjectId(ap.object)) {
        let event = SceneEvent::ObjectUpdated {
            plate_id: plate.id,
            object: obj.clone(),
        };
        drop(s);
        emit_all(&window, &[event]);
    }
    Ok(())
}
