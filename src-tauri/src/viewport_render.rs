//! Strategy-A wgpu viewport: render the live scene to an offscreen texture and
//! read it back as RGBA8 so the frontend can blit it into an opaque `<canvas>`.
//!
//! WebKitGTK can't composite a transparent webview over native GPU content
//! (dynamic DOM smears — see docs/dev/wgpu-renderer.md), so the renderer lives in
//! Rust and hands finished frames to the webview. The GPU-resident scene mirror
//! is here too (per the decision doc): meshes are uploaded once, keyed by
//! `MeshId`, and drawn each frame with their object transforms. Camera stays
//! frontend-owned (passed in per frame).

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use glam::camera::rh::{proj::directx::perspective as perspective_rh, view::look_at_mat4 as look_at_rh};
use glam::{Mat4, Quat, Vec3};
use wgpu::util::DeviceExt;

use crate::core::printer::{instance_registry, PrinterInstance, SlotRef};
use crate::core::project::resolve::{tower_geometry_for_plate, TowerGeometry};
use crate::core::project::{PlateId, Project};
use crate::core::scene::state::{mesh_bb_corners, MeshId, ModifierKind, ObjectId};
use crate::viewport_gizmo::{
    compute_pre, pick_gizmo, pick_move_at, ray_plane, selection_basis, selection_gizmo,
    selection_world_aabb, GizmoGrab, GizmoMode, GrabKind, GIZMO_SCREEN_K,
};
use crate::viewport_gpu::{
    cam_eye, cursor_ray, make_targets, read_rgba, view_proj, COLOR_FMT, SAMPLES,
};

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    pos: [f32; 3],
    nrm: [f32; 3],
}

const SHADER: &str = r#"
struct U {
  mvp: mat4x4<f32>,
  color: vec4<f32>,
  model: mat4x4<f32>,     // world matrix, for the split tool's per-side tint
  plane_o: vec4<f32>,     // xyz = cut-plane origin, w = cut active (0/1)
  plane_n: vec4<f32>,     // xyz = cut-plane normal, w = keep code (bit0 pos, bit1 neg)
};
@group(0) @binding(0) var<uniform> u: U;
struct VO { @builtin(position) p: vec4<f32>, @location(0) n: vec3<f32>, @location(1) wpos: vec3<f32> };
@vertex fn vs(@location(0) pos: vec3<f32>, @location(1) nrm: vec3<f32>) -> VO {
  var o: VO; o.p = u.mvp * vec4<f32>(pos, 1.0); o.n = nrm;
  o.wpos = (u.model * vec4<f32>(pos, 1.0)).xyz; return o;
}
@fragment fn fs(i: VO) -> @location(0) vec4<f32> {
  if (dot(i.n, i.n) < 0.01) { return vec4<f32>(u.color.rgb, 1.0); } // unlit line (grid / bbox bracket)
  let n = normalize(i.n);
  // Matte, two-sided lighting matching the previous viewport (ambient 0.55 +
  // key + fill). High ambient + abs() compresses contrast so the source model's
  // inconsistent/flipped normals don't carve stark facets (the "missing shapes"
  // artifact); a shinier, one-sided model exposed them.
  let key = normalize(vec3<f32>(0.5, -0.5, 0.85));
  let fill = normalize(vec3<f32>(-0.5, 0.5, 0.33));
  let d = 0.55 + abs(dot(n, key)) * 0.4 + abs(dot(n, fill)) * 0.1;
  // color = the face group's resolved color (object base / per-filament paint,
  // already selection-tinted CPU-side).
  var rgb = u.color.rgb;
  // Split-tool preview: tint this fragment red or blue by which side of the cut
  // plane it falls on, and dim the side the user is discarding. Only set for
  // selected objects while the split tool is active (plane_o.w == 1).
  if (u.plane_o.w > 0.5) {
    let pos_side = dot(i.wpos - u.plane_o.xyz, u.plane_n.xyz) >= 0.0;
    let tint = select(vec3<f32>(0.85, 0.25, 0.25), vec3<f32>(0.25, 0.45, 0.95), pos_side); // RED neg / BLUE pos
    rgb = mix(rgb, tint, 0.6);
    let keep = u32(u.plane_n.w + 0.5);
    let kept = select((keep & 2u) != 0u, (keep & 1u) != 0u, pos_side);
    if (!kept) { rgb = rgb * 0.4; } // ghost the discard side
  }
  return vec4<f32>(rgb * min(d, 1.0), 1.0);
}
"#;

/// Gizmo vertex: position, normal (for shading the rods/balls), per-vertex color
/// (axes/planes are multi-colored in one draw).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GizmoVertex {
    pos: [f32; 3],
    nrm: [f32; 3],
    color: [f32; 3],
}

const GIZMO_SHADER: &str = r#"
struct U { mvp: mat4x4<f32>, color: vec4<f32> };
@group(0) @binding(0) var<uniform> u: U;
struct VO { @builtin(position) p: vec4<f32>, @location(0) n: vec3<f32>, @location(1) c: vec3<f32> };
@vertex fn vs(@location(0) pos: vec3<f32>, @location(1) nrm: vec3<f32>, @location(2) col: vec3<f32>) -> VO {
  var o: VO; o.p = u.mvp * vec4<f32>(pos, 1.0); o.n = nrm; o.c = col; return o;
}
@fragment fn fs(i: VO) -> @location(0) vec4<f32> {
  let l = normalize(vec3<f32>(0.4, 0.5, 0.9));
  let d = abs(dot(normalize(i.n), l)) * 0.45 + 0.55; // soft, stays bright
  return vec4<f32>(i.c * d, 1.0);
}
"#;

// Translucent priming tower: lit, alpha from the uniform (drawn with blending).
const TOWER_SHADER: &str = r#"
struct U { mvp: mat4x4<f32>, color: vec4<f32> };
@group(0) @binding(0) var<uniform> u: U;
struct VO { @builtin(position) p: vec4<f32>, @location(0) n: vec3<f32> };
@vertex fn vs(@location(0) pos: vec3<f32>, @location(1) nrm: vec3<f32>) -> VO {
  var o: VO; o.p = u.mvp * vec4<f32>(pos, 1.0); o.n = nrm; return o;
}
@fragment fn fs(i: VO) -> @location(0) vec4<f32> {
  let n = normalize(i.n);
  let d = 0.6 + abs(n.z) * 0.4;
  return vec4<f32>(u.color.rgb * d, u.color.a);
}
"#;
const TOWER_RGB: [f32; 3] = [0.231, 0.510, 0.965]; // #3b82f6, matches the previous overlay
const TOWER_BOX_H: f32 = 50.0; // indicative box height (the real tower is as tall as the print)

/// Shared move/scale handle proportions (fractions of the arm length `l`) so the
/// two gizmos read as one family: identical rods + matched endpoint footprint.
const GIZMO_ROD_R: f32 = 0.012; // rod radius (also the rotate ring tube)
const GIZMO_TIP: f32 = 0.12; // endpoint size: move cone base Ø == scale cube width
const GIZMO_CONE_H: f32 = 0.2; // move arrowhead length, beyond the full-length rod
/// Per-object uniform: `mat4 mvp` (64) + `vec4 color` (16) + `mat4 model` (64)
/// + `vec4 plane_o` (16, xyz origin + w = cut-active flag) + `vec4 plane_n`
/// (16, xyz normal + w = keep code). The trailing 96 bytes drive the split
/// tool's per-side red/blue tint; they're zero (inert) for every normal frame
/// and for the gizmo/tower/line shaders, whose smaller `U` ignores the tail.
const UNIFORM_BYTES: u64 = 176;
// Bed extents are f64 (BoundingBox); converted to f32 for the GPU.
const DEFAULT_BED: ([f64; 3], [f64; 3]) = ([-110.0, -110.0, 0.0], [110.0, 110.0, 200.0]);
const SELECTED_RGB: [f32; 3] = [0.231, 0.510, 0.965]; // tailwind blue-500 (#3b82f6)
const DEFAULT_RGB: [f32; 3] = [0.694, 0.694, 0.694]; // #b1b1b1, matches the previous fallback
const GRID_LINE: [f32; 4] = [0.34, 0.36, 0.40, 1.0];
const BRACKET: [f32; 4] = [0.85, 0.88, 0.97, 1.0]; // selection bbox corner brackets
// Origin axis-marker colors (mirror the previous viewport + corner legend).
const AXIS_X: [f32; 4] = [1.0, 0.267, 0.267, 1.0]; // #ff4444
const AXIS_Y: [f32; 4] = [0.267, 0.867, 0.267, 1.0]; // #44dd44
const AXIS_Z: [f32; 4] = [0.267, 0.533, 1.0, 1.0]; // #4488ff

/// Corner-bracket line segments for selected objects' bounding boxes: at each of
/// the 8 corners, three short segments (25% of the edge) toward the adjacent
/// corners. Built in world space (corners transformed by the object's model).
fn bbox_brackets(boxes: &[(Mat4, [f32; 3], [f32; 3])]) -> Vec<Vertex> {
    let mut v = Vec::new();
    for (model, min, max) in boxes {
        let corner = |sx: bool, sy: bool, sz: bool| {
            Vec3::new(
                if sx { max[0] } else { min[0] },
                if sy { max[1] } else { min[1] },
                if sz { max[2] } else { min[2] },
            )
        };
        for &sx in &[false, true] {
            for &sy in &[false, true] {
                for &sz in &[false, true] {
                    let c = corner(sx, sy, sz);
                    let cw = model.transform_point3(c);
                    for n in [corner(!sx, sy, sz), corner(sx, !sy, sz), corner(sx, sy, !sz)] {
                        let ew = model.transform_point3(c + (n - c) * 0.25);
                        v.push(Vertex { pos: cw.to_array(), nrm: [0.0; 3] });
                        v.push(Vertex { pos: ew.to_array(), nrm: [0.0; 3] });
                    }
                }
            }
        }
    }
    v
}

/// Parse a CSS hex color (`#rrggbb`[`aa`]) to linear-ish 0..1 rgb.
fn parse_hex(s: &str) -> Option<[f32; 3]> {
    let s = s.trim_start_matches('#');
    if s.len() < 6 {
        return None;
    }
    let ch = |i: usize| u8::from_str_radix(&s[i..i + 2], 16).ok().map(|v| v as f32 / 255.0);
    Some([ch(0)?, ch(2)?, ch(4)?])
}

/// Resolve an object's spool color: extruder_id (default 1) → `material_to_slot`
/// → the bound instance's slot color, falling back to neutral gray. Mirrors the
/// frontend's `colorForObject`.
fn spool_color(
    material_to_slot: &BTreeMap<u8, SlotRef>,
    instance: Option<&PrinterInstance>,
    extruder_id: Option<u8>,
) -> [f32; 3] {
    let material = extruder_id.unwrap_or(1);
    material_to_slot
        .get(&material)
        .and_then(|sr| {
            let slot = instance?
                .extruders
                .get(sr.extruder as usize)?
                .slots
                .get(sr.slot as usize)?;
            parse_hex(slot.color.as_deref()?)
        })
        .unwrap_or(DEFAULT_RGB)
}

/// Camera + target the frontend passes per frame (camera is frontend-owned).
/// `gizmo_drag`, when present, is a transient drag preview: the active gizmo
/// handle + cursor are resolved Rust-side into a world pre-multiply applied to
/// the selection for this frame only — no scene-state change — so dragging stays
/// smooth; the real transforms commit once on release (`viewport_gizmo_commit`).
#[derive(serde::Deserialize)]
pub struct FrameRequest {
    pub width: u32,
    pub height: u32,
    pub az: f32,
    pub el: f32,
    pub dist: f32,
    pub center: [f32; 3],
    /// Which gizmo to show at the selection center.
    #[serde(default)]
    pub gizmo: GizmoMode,
    /// An in-progress handle drag: the grabbed handle + current cursor. The
    /// preview transform is derived from it Rust-side (see `compute_pre`).
    #[serde(default)]
    pub gizmo_drag: Option<GizmoDrag>,
    /// Hovered gizmo handle to highlight: for Move 0/1/2 = X/Y/Z axis, 3/4/5 =
    /// XY/YZ/XZ plane; for Rotate 0/1/2 = X/Y/Z ring; -1 = none.
    #[serde(default = "neg_one")]
    pub gizmo_hover: i32,
    /// Active split-tool cutting plane. When present the renderer draws the
    /// translucent plane quad, tints the selected mesh red/blue per side, and
    /// puts the move gizmo on the plane instead of the selection.
    #[serde(default)]
    pub cut: Option<CutPreview>,
}

/// The split tool's cutting plane for one frame (world space). `keep_pos` /
/// `keep_neg` choose which sides stay solid vs. ghosted in the preview;
/// `connectors` are drawn as translucent peg previews on the plane.
#[derive(serde::Deserialize, Clone)]
pub struct CutPreview {
    pub origin: [f32; 3],
    pub normal: [f32; 3],
    #[serde(default)]
    pub keep_pos: bool,
    #[serde(default)]
    pub keep_neg: bool,
    #[serde(default)]
    pub connectors: Vec<CutConnectorPreview>,
}

/// One connector's peg preview (world position on the plane + size). The peg
/// axis is the plane normal; `selected` brightens it.
#[derive(serde::Deserialize, Clone, Copy)]
pub struct CutConnectorPreview {
    pub pos: [f32; 3],
    pub radius: f32,
    pub height: f32,
    #[serde(default)]
    pub selected: bool,
}

impl CutPreview {
    /// `plane_n.w` keep code consumed by the mesh shader (bit0 = keep positive
    /// side, bit1 = keep negative side).
    fn keep_code(&self) -> f32 {
        (self.keep_pos as u32 | ((self.keep_neg as u32) << 1)) as f32
    }
}

/// A live handle drag: the handle captured at grab time plus the current cursor
/// (canvas pixels) and whether Shift (freehand, no snap) is held.
#[derive(serde::Deserialize)]
pub struct GizmoDrag {
    pub grab: GizmoGrab,
    pub sx: f32,
    pub sy: f32,
    #[serde(default)]
    pub shift: bool,
}

fn neg_one() -> i32 {
    -1
}

/// One paint-state index group within a mesh: the triangles painted with
/// filament `state` (0 = unpainted → object's base color), as an index buffer
/// into the shared vertex buffer.
struct MeshGroup {
    state: u8,
    ib: wgpu::Buffer,
    n_indices: u32,
}

/// Priming tower the frontend pushes (a slice/overlay artifact, not part of the
/// Rust scene). Placement is `x`/`y` corner + `width`/`brim`/`rotation` (bed mm);
/// `mesh` is the exact sliced shape when one's valid (else the predicted box).
/// The XY bounding box `[min_x, min_y, max_x, max_y]` of flat `xyz` vertex
/// positions. `None` for an empty/degenerate vertex list.
fn tower_footprint(verts: &[f32]) -> Option<[f32; 4]> {
    let mut min = [f32::INFINITY; 2];
    let mut max = [f32::NEG_INFINITY; 2];
    for p in verts.chunks_exact(3) {
        min[0] = min[0].min(p[0]);
        min[1] = min[1].min(p[1]);
        max[0] = max[0].max(p[0]);
        max[1] = max[1].max(p[1]);
    }
    (min[0] <= max[0]).then_some([min[0], min[1], max[0], max[1]])
}

/// The tower's on-bed footprint (+brim) in tower-local coords: the sliced mesh's
/// XY bbox when known, else the square box `-brim..width+brim` on both axes.
fn tower_local_footprint(footprint: Option<[f32; 4]>, width: f32, brim: f32) -> [f32; 4] {
    footprint.unwrap_or([-brim, -brim, width + brim, width + brim])
}

/// Clamp a requested corner so the footprint stays within `[bed_min, bed_max]`.
/// `fp` is the resolved local footprint from [`tower_local_footprint`]. When the
/// footprint is wider than the bed the low edge wins (matches the frontend's
/// former `clampTowerCorner`).
fn clamp_tower_corner(fp: [f32; 4], bed_min: [f32; 2], bed_max: [f32; 2], x: f32, y: f32) -> (f32, f32) {
    let lo_x = bed_min[0] - fp[0];
    let hi_x = bed_max[0] - fp[2];
    let lo_y = bed_min[1] - fp[1];
    let hi_y = bed_max[1] - fp[3];
    (
        x.max(lo_x).min(hi_x.max(lo_x)),
        y.max(lo_y).min(hi_y.max(lo_y)),
    )
}

/// Whether bed point `(bx, by)` lands on the tower's footprint at corner
/// `(corner_x, corner_y)`. `fp` is the resolved local footprint.
fn tower_corner_hit(fp: [f32; 4], corner_x: f32, corner_y: f32, bx: f32, by: f32) -> bool {
    bx >= corner_x + fp[0]
        && bx <= corner_x + fp[2]
        && by >= corner_y + fp[1]
        && by <= corner_y + fp[3]
}

struct TowerGpu {
    vb: wgpu::Buffer,
    ib: wgpu::Buffer,
    n_indices: u32,
}

/// A plate's sliced tower mesh on the GPU plus its on-bed XY footprint
/// `[min_x, min_y, max_x, max_y]` (tower-local, relative to the placement
/// corner) for the drag clamp + hit-test. Fed by the slice event sink, keyed by
/// plate, so switching plates never re-uploads the mesh.
struct TowerMeshEntry {
    gpu: TowerGpu,
    footprint: Option<[f32; 4]>,
    /// The material count + bound printer this was sliced at. The tower reshapes
    /// on either, so the mesh is shown only while these still match the resolved
    /// geometry — else it's stale and `frame` falls back to the predicted box.
    material_count: usize,
    printer_instance_id: Option<String>,
}

struct GpuMesh {
    vb: wgpu::Buffer,
    // One group for an unpainted mesh (state 0); one per filament for a painted
    // mesh. The vertices stay shared/indexed — only the triangles are partitioned,
    // so painting never multiplies the vertex count.
    groups: Vec<MeshGroup>,
}

/// Interleave positions with **recomputed** area-weighted smooth vertex normals
/// (not the source `normals`): imported models often carry bad/flat per-vertex
/// normals that shade as faceted "missing shapes" even though the geometry is
/// fine (it slices smooth). The mesh is welded (shared vertices), so accumulating
/// face normals per vertex yields the smooth shading the slice shows.
fn smooth_verts(vertices: &[f32], indices: &[u32]) -> Vec<Vertex> {
    let vcount = vertices.len() / 3;
    let pos: Vec<Vec3> = (0..vcount)
        .map(|i| Vec3::new(vertices[3 * i], vertices[3 * i + 1], vertices[3 * i + 2]))
        .collect();
    let mut nrm = vec![Vec3::ZERO; vcount];
    for tri in indices.chunks_exact(3) {
        let (a, b, c) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
        if a < vcount && b < vcount && c < vcount {
            let face = (pos[b] - pos[a]).cross(pos[c] - pos[a]); // area-weighted
            nrm[a] += face;
            nrm[b] += face;
            nrm[c] += face;
        }
    }
    (0..vcount)
        .map(|i| Vertex { pos: pos[i].to_array(), nrm: nrm[i].normalize_or_zero().to_array() })
        .collect()
}

/// Build our interleaved vertex buffer for a `Mesh` (smooth normals) and upload
/// it, partitioning triangles into per-paint-state index groups.
fn upload_mesh(device: &wgpu::Device, m: &crate::core::scene::state::Mesh) -> GpuMesh {
    let verts = smooth_verts(&m.vertices, &m.indices);
    let vb = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("viewport.mesh.vb"),
        contents: bytemuck::cast_slice(&verts),
        usage: wgpu::BufferUsages::VERTEX,
    });
    let make_ib = |state: u8, indices: &[u32]| MeshGroup {
        state,
        ib: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("viewport.mesh.ib"),
            contents: bytemuck::cast_slice(indices),
            usage: wgpu::BufferUsages::INDEX,
        }),
        n_indices: indices.len() as u32,
    };
    // MMU paint: partition triangles by their dominant filament state into index
    // groups (vertices stay shared, so painting never multiplies vertex count).
    let states = m
        .paint_colors
        .as_deref()
        .map(Vec::as_slice)
        .and_then(crate::core::threemf::decode_dominant_states)
        .filter(|s| s.len() == m.indices.len() / 3);
    let groups = match states {
        Some(states) => {
            let mut by_state: BTreeMap<u8, Vec<u32>> = BTreeMap::new();
            for (t, tri) in m.indices.chunks_exact(3).enumerate() {
                by_state.entry(states[t]).or_default().extend_from_slice(tri);
            }
            by_state.into_iter().map(|(s, idx)| make_ib(s, &idx)).collect()
        }
        None => vec![make_ib(0, &m.indices)],
    };
    GpuMesh { vb, groups }
}

/// Build-plate grid lines on the bed floor (`z = min.z`), ~10mm spacing.
fn grid_verts(min: [f32; 3], max: [f32; 3]) -> Vec<Vertex> {
    let z = min[2];
    let step = 10.0_f32;
    let mut v = Vec::new();
    let nx = ((max[0] - min[0]) / step).ceil() as i32;
    let ny = ((max[1] - min[1]) / step).ceil() as i32;
    for i in 0..=nx {
        let x = min[0] + i as f32 * step;
        v.push(Vertex { pos: [x, min[1], z], nrm: [0.0; 3] });
        v.push(Vertex { pos: [x, max[1], z], nrm: [0.0; 3] });
    }
    for j in 0..=ny {
        let y = min[1] + j as f32 * step;
        v.push(Vertex { pos: [min[0], y, z], nrm: [0.0; 3] });
        v.push(Vertex { pos: [max[0], y, z], nrm: [0.0; 3] });
    }
    v
}

/// Unit cube centred at the origin (±0.5), per-face normals — the priming-tower
/// box, scaled/placed by its model matrix.
fn unit_cube_verts() -> Vec<Vertex> {
    let mut v = Vec::new();
    let faces = [
        ([1.0, 0.0, 0.0], [[0.5, -0.5, -0.5], [0.5, 0.5, -0.5], [0.5, 0.5, 0.5], [0.5, -0.5, 0.5]]),
        ([-1.0, 0.0, 0.0], [[-0.5, 0.5, -0.5], [-0.5, -0.5, -0.5], [-0.5, -0.5, 0.5], [-0.5, 0.5, 0.5]]),
        ([0.0, 1.0, 0.0], [[0.5, 0.5, -0.5], [-0.5, 0.5, -0.5], [-0.5, 0.5, 0.5], [0.5, 0.5, 0.5]]),
        ([0.0, -1.0, 0.0], [[-0.5, -0.5, -0.5], [0.5, -0.5, -0.5], [0.5, -0.5, 0.5], [-0.5, -0.5, 0.5]]),
        ([0.0, 0.0, 1.0], [[-0.5, -0.5, 0.5], [0.5, -0.5, 0.5], [0.5, 0.5, 0.5], [-0.5, 0.5, 0.5]]),
        ([0.0, 0.0, -1.0], [[-0.5, 0.5, -0.5], [0.5, 0.5, -0.5], [0.5, -0.5, -0.5], [-0.5, -0.5, -0.5]]),
    ];
    for (n, q) in faces {
        for &i in &[0usize, 1, 2, 0, 2, 3] {
            v.push(Vertex { pos: q[i], nrm: n });
        }
    }
    v
}

/// A unit cylinder along Z: radius 1, height 1 centered at z=0 (z ∈ [-0.5, 0.5]),
/// as a triangle list (side + both caps). Used for the split tool's connector
/// peg previews (scaled + oriented to the plane normal per connector).
const CYL_SEG: usize = 20;
fn unit_cylinder_verts() -> Vec<Vertex> {
    let mut v = Vec::new();
    let ring = |i: usize| -> (f32, f32) {
        let t = i as f32 / CYL_SEG as f32 * std::f32::consts::TAU;
        (t.cos(), t.sin())
    };
    for i in 0..CYL_SEG {
        let (c0, s0) = ring(i);
        let (c1, s1) = ring(i + 1);
        let (n0, n1) = ([c0, s0, 0.0], [c1, s1, 0.0]);
        let (b0, t0) = ([c0, s0, -0.5], [c0, s0, 0.5]);
        let (b1, t1) = ([c1, s1, -0.5], [c1, s1, 0.5]);
        // side
        v.push(Vertex { pos: b0, nrm: n0 });
        v.push(Vertex { pos: b1, nrm: n1 });
        v.push(Vertex { pos: t1, nrm: n1 });
        v.push(Vertex { pos: b0, nrm: n0 });
        v.push(Vertex { pos: t1, nrm: n1 });
        v.push(Vertex { pos: t0, nrm: n0 });
        // top cap (+Z)
        v.push(Vertex { pos: [0.0, 0.0, 0.5], nrm: [0.0, 0.0, 1.0] });
        v.push(Vertex { pos: t0, nrm: [0.0, 0.0, 1.0] });
        v.push(Vertex { pos: t1, nrm: [0.0, 0.0, 1.0] });
        // bottom cap (-Z)
        v.push(Vertex { pos: [0.0, 0.0, -0.5], nrm: [0.0, 0.0, -1.0] });
        v.push(Vertex { pos: b1, nrm: [0.0, 0.0, -1.0] });
        v.push(Vertex { pos: b0, nrm: [0.0, 0.0, -1.0] });
    }
    v
}

/// The 12 edges of the unit cube as line segments (24 verts) — the box outline.
fn unit_cube_edges() -> Vec<Vertex> {
    let c = |x: f32, y: f32, z: f32| Vertex { pos: [x, y, z], nrm: [0.0; 3] };
    let p = |s: f32| if s < 0.5 { -0.5 } else { 0.5 };
    let corner = |i: u32| c(p((i & 1) as f32), p(((i >> 1) & 1) as f32), p(((i >> 2) & 1) as f32));
    let mut v = Vec::new();
    for a in 0u32..8 {
        for bit in 0u32..3 {
            let b = a ^ (1 << bit);
            if b > a {
                v.push(corner(a));
                v.push(corner(b));
            }
        }
    }
    v
}

/// Origin axis markers: three short segments (+X, +Y, +Z) from the world origin,
/// lifted a hair off the grid. Two verts per axis; drawn with the per-axis color.
fn axes_verts(min: [f32; 3], max: [f32; 3]) -> [Vertex; 6] {
    let len = (max[0] - min[0]).min(max[1] - min[1]) * 0.18;
    let z = min[2] + 0.05;
    let o = Vertex { pos: [0.0, 0.0, z], nrm: [0.0; 3] };
    [
        o,
        Vertex { pos: [len, 0.0, z], nrm: [0.0; 3] },
        o,
        Vertex { pos: [0.0, len, z], nrm: [0.0; 3] },
        o,
        Vertex { pos: [0.0, 0.0, z + len], nrm: [0.0; 3] },
    ]
}

/// One flat quad (two triangles, single normal). Winding-agnostic — the gizmo
/// pipeline culls nothing and shades two-sided.
fn push_quad(v: &mut Vec<GizmoVertex>, a: Vec3, b: Vec3, c: Vec3, d: Vec3, n: Vec3, col: [f32; 3]) {
    let n = n.to_array();
    for p in [a, b, c, a, c, d] {
        v.push(GizmoVertex { pos: p.to_array(), nrm: n, color: col });
    }
}

/// Round rod (cylinder) of `radius` from `start` along `dir` for `len`, with end
/// caps and smooth radial normals — a solid bar instead of a 1px line. `dir` is
/// assumed unit-length (the gizmo passes axis vectors).
fn push_cylinder(v: &mut Vec<GizmoVertex>, start: Vec3, dir: Vec3, len: f32, radius: f32, col: [f32; 3]) {
    const SEG: usize = 16;
    let up = if dir.x.abs() > 0.9 { Vec3::Y } else { Vec3::X };
    let b1 = dir.cross(up).normalize();
    let b2 = dir.cross(b1).normalize();
    let end = start + dir * len;
    // Per-ring point i: radial offset (× radius) and its outward normal.
    let ring = |i: usize| -> (Vec3, Vec3) {
        let a = std::f32::consts::TAU * (i as f32) / (SEG as f32);
        let radial = b1 * a.cos() + b2 * a.sin();
        (radial * radius, radial)
    };
    let mut push = |p: Vec3, n: Vec3| v.push(GizmoVertex { pos: p.to_array(), nrm: n.to_array(), color: col });
    for i in 0..SEG {
        let (o0, n0) = ring(i);
        let (o1, n1) = ring(i + 1);
        // Side wall (two tris).
        push(start + o0, n0);
        push(start + o1, n1);
        push(end + o1, n1);
        push(start + o0, n0);
        push(end + o1, n1);
        push(end + o0, n0);
        // End cap (+dir) and start cap (-dir).
        push(end, dir);
        push(end + o0, dir);
        push(end + o1, dir);
        push(start, -dir);
        push(start + o1, -dir);
        push(start + o0, -dir);
    }
}

/// Cone (arrowhead) with apex at `tip`, pointing along `dir`, of `height` and
/// base `radius`. Slanted side normals + a base cap.
fn push_cone(v: &mut Vec<GizmoVertex>, tip: Vec3, dir: Vec3, height: f32, radius: f32, col: [f32; 3]) {
    const SEG: usize = 16;
    let base = tip - dir * height;
    let up = if dir.x.abs() > 0.9 { Vec3::Y } else { Vec3::X };
    let e1 = dir.cross(up).normalize();
    let e2 = dir.cross(e1).normalize();
    let rim = |i: usize| -> Vec3 {
        let a = std::f32::consts::TAU * (i as f32) / (SEG as f32);
        base + (e1 * a.cos() + e2 * a.sin()) * radius
    };
    // Slant normal at a rim point: radial outward tilted toward the apex.
    let slant = (radius / height.max(1e-6)).atan();
    let norm_at = |i: usize| -> Vec3 {
        let a = std::f32::consts::TAU * (i as f32) / (SEG as f32);
        let radial = e1 * a.cos() + e2 * a.sin();
        (radial * slant.cos() + dir * slant.sin()).normalize()
    };
    for i in 0..SEG {
        let (p0, p1) = (rim(i), rim(i + 1));
        let (n0, n1) = (norm_at(i), norm_at(i + 1));
        let napex = (n0 + n1).normalize_or_zero();
        v.push(GizmoVertex { pos: p0.to_array(), nrm: n0.to_array(), color: col });
        v.push(GizmoVertex { pos: p1.to_array(), nrm: n1.to_array(), color: col });
        v.push(GizmoVertex { pos: tip.to_array(), nrm: napex.to_array(), color: col });
        // Base cap (normal -dir).
        let nd = (-dir).to_array();
        v.push(GizmoVertex { pos: base.to_array(), nrm: nd, color: col });
        v.push(GizmoVertex { pos: p1.to_array(), nrm: nd, color: col });
        v.push(GizmoVertex { pos: p0.to_array(), nrm: nd, color: col });
    }
}

/// Lerp a handle color halfway to white when hovered.
fn hl(c: [f32; 3], on: bool) -> [f32; 3] {
    if on {
        [c[0] * 0.5 + 0.5, c[1] * 0.5 + 0.5, c[2] * 0.5 + 0.5]
    } else {
        c
    }
}

/// Move gizmo at `center`: one rod+arrowhead handle per axis (X/Y/Z) + a filled
/// square per plane (XY/YZ/XZ). `arm` is the rod length (object-sized, = the
/// rotate ring radius, so the gizmo scales with the part); `l` is the
/// screen-constant thickness scale (rod/tip radii hold a fixed on-screen size).
/// `hover` brightens one handle (0/1/2 = X/Y/Z axis, 3/4/5 = XY/YZ/XZ plane).
fn gizmo_geometry(center: Vec3, l: f32, arm: f32, hover: i32) -> Vec<GizmoVertex> {
    let mut v = Vec::new();
    let rod_r = l * GIZMO_ROD_R;
    let cone_h = l * GIZMO_CONE_H;
    let cone_r = l * GIZMO_TIP * 0.5; // base radius → base Ø == scale cube width
    let axes = [
        (Vec3::X, [0.90, 0.27, 0.27]),
        (Vec3::Y, [0.30, 0.80, 0.33]),
        (Vec3::Z, [0.36, 0.48, 0.96]),
    ];
    for (i, (dir, color)) in axes.into_iter().enumerate() {
        let color = hl(color, hover == i as i32);
        // Object-length rod, screen-constant arrowhead seated on its end.
        push_cylinder(&mut v, center, dir, arm, rod_r, color);
        push_cone(&mut v, center + dir * (arm + cone_h), dir, cone_h, cone_r, color);
    }
    // Filled planar handles, offset into each plane; normal is the third axis.
    let planes = [
        (Vec3::X, Vec3::Y, Vec3::Z, [0.92, 0.82, 0.28]),
        (Vec3::Y, Vec3::Z, Vec3::X, [0.28, 0.82, 0.86]),
        (Vec3::X, Vec3::Z, Vec3::Y, [0.86, 0.36, 0.86]),
    ];
    let (o, s) = (l * 0.28, l * 0.24);
    for (i, (a, b, n, color)) in planes.into_iter().enumerate() {
        let color = hl(color, hover == 3 + i as i32);
        let c0 = center + a * o + b * o;
        let c1 = center + a * (o + s) + b * o;
        let c2 = center + a * (o + s) + b * (o + s);
        let c3 = center + a * o + b * (o + s);
        push_quad(&mut v, c0, c1, c2, c3, n, color);
    }
    v
}

/// Torus around `axis` at `center`, major radius `r`, tube radius `tube`, smooth
/// outward normals. Winding-agnostic (the gizmo shades two-sided).
fn push_torus(v: &mut Vec<GizmoVertex>, center: Vec3, axis: Vec3, r: f32, tube: f32, col: [f32; 3]) {
    const MAJOR: usize = 48;
    const MINOR: usize = 10;
    let up = if axis.x.abs() > 0.9 { Vec3::Y } else { Vec3::X };
    let e1 = axis.cross(up).normalize();
    let e2 = axis.cross(e1).normalize();
    // Tube center + outward radial at major angle u; surface point at minor angle x.
    let pt = |i: usize, j: usize| -> (Vec3, Vec3) {
        let u = std::f32::consts::TAU * (i as f32) / (MAJOR as f32);
        let x = std::f32::consts::TAU * (j as f32) / (MINOR as f32);
        let radial = e1 * u.cos() + e2 * u.sin();
        let c = center + radial * r;
        let n = radial * x.cos() + axis * x.sin();
        (c + n * tube, n)
    };
    let quad = |a: (Vec3, Vec3), b: (Vec3, Vec3), c: (Vec3, Vec3), d: (Vec3, Vec3), v: &mut Vec<GizmoVertex>| {
        for (p, n) in [a, b, c, a, c, d] {
            v.push(GizmoVertex { pos: p.to_array(), nrm: n.to_array(), color: col });
        }
    };
    for i in 0..MAJOR {
        for j in 0..MINOR {
            quad(pt(i, j), pt(i + 1, j), pt(i + 1, j + 1), pt(i, j + 1), v);
        }
    }
}

/// Rotate gizmo at `center`: one ring per axis (X/Y/Z) of radius `radius` (sized
/// to the object so the ring wraps it), each a torus of `tube` thickness. The
/// radius is object-sized but `tube` is screen-constant (passed in), so the rings
/// don't get chunkier on bigger parts. `hover` brightens ring 0/1/2 (X/Y/Z).
fn gizmo_rotate_geometry(center: Vec3, radius: f32, tube: f32, hover: i32) -> Vec<GizmoVertex> {
    let mut v = Vec::new();
    let rings = [
        (Vec3::X, [0.90, 0.27, 0.27]),
        (Vec3::Y, [0.30, 0.80, 0.33]),
        (Vec3::Z, [0.36, 0.48, 0.96]),
    ];
    for (i, (axis, color)) in rings.into_iter().enumerate() {
        push_torus(&mut v, center, axis, radius, tube, hl(color, hover == i as i32));
    }
    v
}

/// Axis-aligned-in-`basis` cube centered at `c`, half-extent `h`, per-face normals.
fn push_cube(v: &mut Vec<GizmoVertex>, c: Vec3, h: f32, basis: [Vec3; 3], col: [f32; 3]) {
    let [ex, ey, ez] = basis;
    for (n, u, w) in [(ex, ey, ez), (-ex, ey, ez), (ey, ex, ez), (-ey, ex, ez), (ez, ex, ey), (-ez, ex, ey)] {
        let f = c + n * h;
        push_quad(v, f + u * h + w * h, f - u * h + w * h, f - u * h - w * h, f + u * h - w * h, n, col);
    }
}

/// Scale gizmo at `center`, oriented by `basis` (the object's axes for a single
/// selection, world for multi): a rod tipped with a cube per axis (X/Y/Z), a
/// filled square per plane (XY/YZ/XZ), and a center cube for uniform scale. `arm`
/// is the rod length (object-sized, = the rotate ring radius); `l` is the
/// screen-constant thickness scale (rod/cube radii hold a fixed on-screen size).
/// `hover` brightens one handle (0/1/2 axis, 3/4/5 plane, 6 center). `guide` (an
/// axis index) draws a long thin guide line through that axis.
fn gizmo_scale_geometry(
    center: Vec3,
    l: f32,
    arm: f32,
    basis: [Vec3; 3],
    hover: i32,
    guide: Option<usize>,
) -> Vec<GizmoVertex> {
    let mut v = Vec::new();
    let [ex, ey, ez] = basis;
    let rod_len = arm; // object-length rod, screen-constant cube on its end
    let rod_r = l * GIZMO_ROD_R;
    let cube_h = l * GIZMO_TIP * 0.5; // half-extent → width == move cone base Ø
    let axes = [(ex, [0.90, 0.27, 0.27]), (ey, [0.30, 0.80, 0.33]), (ez, [0.36, 0.48, 0.96])];
    // Guide line: a long thin rod through the active axis, drawn first (behind).
    if let Some(gi) = guide {
        let dir = basis[gi];
        let big = l * 40.0;
        push_cylinder(&mut v, center - dir * big, dir, big * 2.0, l * 0.004, axes[gi].1);
    }
    for (i, (dir, color)) in axes.into_iter().enumerate() {
        let color = hl(color, hover == i as i32);
        push_cylinder(&mut v, center, dir, rod_len, rod_r, color);
        push_cube(&mut v, center + dir * arm, cube_h, basis, color);
    }
    let planes = [
        (ex, ey, ez, [0.92, 0.82, 0.28]),
        (ey, ez, ex, [0.28, 0.82, 0.86]),
        (ex, ez, ey, [0.86, 0.36, 0.86]),
    ];
    let (o, s) = (l * 0.28, l * 0.24);
    for (i, (a, b, n, color)) in planes.into_iter().enumerate() {
        let color = hl(color, hover == 3 + i as i32);
        let c0 = center + a * o + b * o;
        let c1 = center + a * (o + s) + b * o;
        let c2 = center + a * (o + s) + b * (o + s);
        let c3 = center + a * o + b * (o + s);
        push_quad(&mut v, c0, c1, c2, c3, n, color);
    }
    // Uniform 3-axis scale handle: a cube at the center, same size as the axis
    // tip cubes. Its hit radius (thick * 0.16 in the picker) stays generous so
    // it's easy to grab despite the small visual.
    push_cube(&mut v, center, cube_h, basis, hl([0.85, 0.85, 0.88], hover == 6));
    v
}

/// What the cursor grabbed. `Empty` = hit nothing/unselected (the frontend then
/// checks the tower, else orbits); `Inert` = pressed the selected body in a
/// gizmo mode (no drag); `Orbit` = navigate.
#[derive(serde::Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum GrabResult {
    Orbit,
    Inert,
    Empty,
    Gizmo { grab: GizmoGrab },
}

/// Nearest mesh hit under the ray: object id, world face normal, hit point.
fn nearest_hit(p: &Project, ro: Vec3, rd: Vec3) -> Option<(ObjectId, Vec3, Vec3)> {
    let plate = p.active_plate();
    let mut best: Option<(f32, ObjectId, Vec3, Vec3, Vec3)> = None;
    for (id, obj) in plate.scene.objects.iter() {
        if !obj.visible {
            continue;
        }
        let Some(m) = p.meshes.get(&obj.mesh) else { continue };
        let model = obj.transform.to_mat4();
        let vert = |vi: u32| {
            let i = vi as usize * 3;
            model.transform_point3(Vec3::new(m.vertices[i], m.vertices[i + 1], m.vertices[i + 2]))
        };
        for t3 in m.indices.chunks_exact(3) {
            let (a, b, c) = (vert(t3[0]), vert(t3[1]), vert(t3[2]));
            if let Some(t) = ray_tri(ro, rd, a, b, c) {
                if best.map_or(true, |(bt, ..)| t < bt) {
                    best = Some((t, *id, a, b, c));
                }
            }
        }
    }
    best.map(|(t, id, a, b, c)| (id, (b - a).cross(c - a).normalize_or_zero(), ro + rd * t))
}

pub struct ViewportRenderer {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    bgl: wgpu::BindGroupLayout,
    mesh_pipe: wgpu::RenderPipeline,
    line_pipe: wgpu::RenderPipeline,
    axis_pipe: wgpu::RenderPipeline,
    gizmo_pipe: wgpu::RenderPipeline,
    // per-object MVP, one 256-aligned slot each
    ubuf: wgpu::Buffer,
    bind: wgpu::BindGroup,
    slot: u32,
    slots_cap: u32,
    // caches
    meshes: HashMap<MeshId, GpuMesh>,
    grid_key: Option<[f32; 6]>,
    vb_grid: wgpu::Buffer,
    n_grid: u32,
    vb_axes: wgpu::Buffer,
    // Priming tower: translucent pipeline + the box's static cube geometry, plus
    // the exact sliced mesh (pushed by the frontend; not part of the Rust scene).
    tower_pipe: wgpu::RenderPipeline,
    vb_cube: wgpu::Buffer,
    vb_cube_edges: wgpu::Buffer,
    /// Unit cylinder for the split tool's connector peg previews.
    vb_cylinder: wgpu::Buffer,
    n_cylinder: u32,
    /// Per-plate sliced tower meshes, keyed by plate. The slice event sink
    /// stores each plate's mesh here directly (no frontend round-trip); `frame`
    /// draws the active plate's. Cleared with the object meshes on project replace.
    tower_meshes: HashMap<PlateId, TowerMeshEntry>,
    /// Per-plate resolved tower placement, computed lazily in `frame` (a cascade
    /// resolve, too costly per-frame, so cached here — like the settings panel
    /// resolves once on open). `Some(None)` = resolved, no tower. Invalidated on
    /// tower-affecting edits; the active plate is read from the Project so a
    /// switch needs no re-resolve → the tower draws in the same frame as objects.
    tower_geom: HashMap<PlateId, Option<TowerGeometry>>,
    /// Live drag corner during a tower drag: `(plate, (x, y))`. Scoped to the
    /// plate so it never bleeds onto another plate's tower (a no-op commit emits
    /// no event to clear it). Overrides the resolved placement for a smooth drag.
    tower_drag: Option<(PlateId, (f32, f32))>,
    // size-dependent targets
    size: (u32, u32),
    color: wgpu::Texture,
    msaa_view: wgpu::TextureView,
    depth_view: wgpu::TextureView,
    readback: wgpu::Buffer,
    padded_bpr: u32,
}

impl ViewportRenderer {
    pub fn new() -> Self {
        let (device, queue) = crate::viewport_gpu::shared_device();

        // Each slot must be ≥ UNIFORM_BYTES *and* a multiple of the offset
        // alignment (slots are bound by dynamic offset). Round the uniform size
        // up to the alignment — on the common 256-byte-aligned GPU this is 256.
        let align = device.limits().min_uniform_buffer_offset_alignment.max(64) as u64;
        let slot = (UNIFORM_BYTES.div_ceil(align) * align) as u32;
        let slots_cap = 64u32;
        let ubuf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("viewport.mvp"),
            size: (slot * slots_cap) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                // mvp is read in the vertex stage, tint in the fragment stage.
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: std::num::NonZeroU64::new(UNIFORM_BYTES),
                },
                count: None,
            }],
        });
        let bind = make_bind(&device, &bgl, &ubuf);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let vbl = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3],
        };
        let make_pipe = |topology, cull, depth_compare, depth_write_enabled| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: None,
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: std::slice::from_ref(&vbl),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(COLOR_FMT.into())],
                }),
                primitive: wgpu::PrimitiveState {
                    topology,
                    cull_mode: cull,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    // wgpu 29: both are Option now; closure args stay concrete.
                    depth_write_enabled: Some(depth_write_enabled),
                    depth_compare: Some(depth_compare),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState {
                    count: SAMPLES,
                    ..Default::default()
                },
                multiview_mask: None,
                cache: None,
            })
        };
        use wgpu::CompareFunction::{Always, Less};
        use wgpu::PrimitiveTopology::{LineList, TriangleList};
        // No back-face culling: imported meshes (STL etc.) have no winding
        // guarantee, and mixed winding would drop valid front faces (holes). The
        // depth test still resolves the nearest face correctly.
        let mesh_pipe = make_pipe(TriangleList, None, Less, true);
        let line_pipe = make_pipe(LineList, None, Less, true);
        // Origin axis markers lie in the bed plane on top of the grid's x=0/y=0
        // lines, so they z-fight with it. Draw them always-on-top of the grid
        // (Always, no depth write) — still before the meshes, so objects occlude.
        let axis_pipe = make_pipe(LineList, None, Always, false);

        // Gizmo: solid lit triangles, drawn in its own depth-cleared pass so it
        // sits on top of the scene yet self-occludes (near rod hides far rod).
        let gizmo_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(GIZMO_SHADER.into()),
        });
        let gizmo_pipe = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("viewport.gizmo"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &gizmo_shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<GizmoVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x3],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &gizmo_shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(COLOR_FMT.into())],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: SAMPLES,
                ..Default::default()
            },
            multiview_mask: None,
            cache: None,
        });

        // Priming tower: lit translucent triangles, alpha-blended, no depth write.
        let tower_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(TOWER_SHADER.into()),
        });
        let tower_pipe = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("viewport.tower"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &tower_shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: std::slice::from_ref(&vbl),
            },
            fragment: Some(wgpu::FragmentState {
                module: &tower_shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: COLOR_FMT,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(false), // translucent — test but don't occlude
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: SAMPLES,
                ..Default::default()
            },
            multiview_mask: None,
            cache: None,
        });
        let vb_cube = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("viewport.tower.cube"),
            contents: bytemuck::cast_slice(&unit_cube_verts()),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let vb_cube_edges = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("viewport.tower.cube_edges"),
            contents: bytemuck::cast_slice(&unit_cube_edges()),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let cyl_verts = unit_cylinder_verts();
        let n_cylinder = cyl_verts.len() as u32;
        let vb_cylinder = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("viewport.cut.cylinder"),
            contents: bytemuck::cast_slice(&cyl_verts),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let gmin = DEFAULT_BED.0.map(|v| v as f32);
        let gmax = DEFAULT_BED.1.map(|v| v as f32);
        let gverts = grid_verts(gmin, gmax);
        let vb_grid = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("viewport.grid"),
            contents: bytemuck::cast_slice(&gverts),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let vb_axes = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("viewport.axes"),
            contents: bytemuck::cast_slice(&axes_verts(gmin, gmax)),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let (color, msaa_view, depth_view, readback, padded_bpr) = make_targets(&device, 8, 8);
        ViewportRenderer {
            n_grid: gverts.len() as u32,
            grid_key: Some([gmin[0], gmin[1], gmin[2], gmax[0], gmax[1], gmax[2]]),
            device,
            queue,
            bgl,
            mesh_pipe,
            line_pipe,
            axis_pipe,
            gizmo_pipe,
            ubuf,
            bind,
            slot,
            slots_cap,
            meshes: HashMap::new(),
            tower_meshes: HashMap::new(),
            tower_geom: HashMap::new(),
            tower_drag: None,
            vb_grid,
            vb_axes,
            tower_pipe,
            vb_cube,
            vb_cube_edges,
            vb_cylinder,
            n_cylinder,
            size: (0, 0),
            color,
            msaa_view,
            depth_view,
            readback,
            padded_bpr,
        }
    }

    /// The active plate's resolved tower placement, cached. Resolved lazily (a
    /// cascade resolve — too costly per-frame) the first time a plate is drawn,
    /// then reused so switching plates draws the tower in-frame with the objects.
    /// `None` = no tower (single-material / disabled).
    fn resolved_tower(&mut self, p: &Project, plate_id: PlateId) -> Option<TowerGeometry> {
        if let Some(cached) = self.tower_geom.get(&plate_id) {
            return cached.clone();
        }
        let geom = tower_geometry_for_plate(p, plate_id).ok().flatten();
        self.tower_geom.insert(plate_id, geom.clone());
        geom
    }

    /// Drop the resolved-placement cache + any live drag. Called when a
    /// tower-affecting edit (overrides, materials, printer, bed) lands, so the
    /// next frame re-resolves. (Switching plates does NOT invalidate — the cache
    /// is keyed per plate and the active plate is read from the Project.)
    fn invalidate_tower(&mut self) {
        self.tower_geom.clear();
        self.tower_drag = None;
    }

    /// Store a plate's sliced tower mesh (GPU upload + footprint + the
    /// material-count/printer it sliced at, for staleness). Replaces any previous
    /// mesh for that plate; switching to it later re-uploads nothing. Mesh
    /// normals are recomputed smooth, like `upload_mesh`.
    fn store_tower_mesh(
        &mut self,
        plate_id: PlateId,
        vertices: &[f32],
        indices: &[u32],
        material_count: usize,
        printer_instance_id: Option<String>,
    ) {
        let footprint = tower_footprint(vertices);
        let verts = smooth_verts(vertices, indices);
        let gpu = TowerGpu {
            vb: self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("viewport.tower.mesh.vb"),
                contents: bytemuck::cast_slice(&verts),
                usage: wgpu::BufferUsages::VERTEX,
            }),
            ib: self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("viewport.tower.mesh.ib"),
                contents: bytemuck::cast_slice(indices),
                usage: wgpu::BufferUsages::INDEX,
            }),
            n_indices: indices.len() as u32,
        };
        self.tower_meshes.insert(
            plate_id,
            TowerMeshEntry {
                gpu,
                footprint,
                material_count,
                printer_instance_id,
            },
        );
    }

    /// The tower's effective on-bed corner for `plate_id`: the live drag position
    /// if this plate is being dragged, else the resolved placement clamped to the
    /// bed. `frame` (what's drawn) and `viewport_tower_grab` (the press hit-test)
    /// MUST agree on this — otherwise the drawn tower and the grabbable area
    /// diverge (a clamped, OOB-origin tower would be visible but un-grabbable).
    fn effective_tower_corner(
        &self,
        plate_id: PlateId,
        geom: &TowerGeometry,
        fp: [f32; 4],
        bed_min: [f32; 2],
        bed_max: [f32; 2],
    ) -> (f32, f32) {
        match self.tower_drag {
            Some((pid, corner)) if pid == plate_id => corner,
            _ => clamp_tower_corner(fp, bed_min, bed_max, geom.x as f32, geom.y as f32),
        }
    }

    /// The plate's stored tower mesh, but only if it isn't stale — its sliced-at
    /// material count + printer must still match the resolved geometry (the tower
    /// reshapes on either, and moving it doesn't re-slice). Stale → `None`, so
    /// `frame` shows the predicted box instead.
    fn valid_tower_mesh(&self, plate_id: PlateId, geom: &TowerGeometry) -> Option<&TowerMeshEntry> {
        self.tower_meshes.get(&plate_id).filter(|e| {
            e.material_count == geom.material_count
                && e.printer_instance_id == geom.printer_instance_id
        })
    }

    /// Shared prelude for the drag commands: the active plate's id, resolved
    /// tower geometry, bed extents, and on-bed footprint (mesh bbox when valid,
    /// else the box). `None` when there's no tower or no bed.
    fn tower_drag_ctx(
        &mut self,
        p: &Project,
    ) -> Option<(PlateId, TowerGeometry, [f64; 3], [f64; 3], [f32; 4])> {
        let plate = p.active_plate();
        let bed = plate.scene.bed.as_ref()?;
        let (bed_min, bed_max) = (bed.extents.min, bed.extents.max);
        let id = plate.id;
        let geom = self.resolved_tower(p, id)?;
        let footprint = self.valid_tower_mesh(id, &geom).and_then(|e| e.footprint);
        let fp = tower_local_footprint(footprint, geom.width as f32, geom.brim as f32);
        Some((id, geom, bed_min, bed_max, fp))
    }

    fn resize(&mut self, w: u32, h: u32) {
        if self.size == (w, h) {
            return;
        }
        self.size = (w, h);
        let (color, msaa_view, depth_view, readback, padded_bpr) = make_targets(&self.device, w, h);
        self.color = color;
        self.msaa_view = msaa_view;
        self.depth_view = depth_view;
        self.readback = readback;
        self.padded_bpr = padded_bpr;
    }

    fn ensure_grid(&mut self, min: [f32; 3], max: [f32; 3]) {
        let key = [min[0], min[1], min[2], max[0], max[1], max[2]];
        if self.grid_key == Some(key) {
            return;
        }
        let gverts = grid_verts(min, max);
        self.vb_grid = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("viewport.grid"),
            contents: bytemuck::cast_slice(&gverts),
            usage: wgpu::BufferUsages::VERTEX,
        });
        self.n_grid = gverts.len() as u32;
        self.vb_axes = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("viewport.axes"),
            contents: bytemuck::cast_slice(&axes_verts(min, max)),
            usage: wgpu::BufferUsages::VERTEX,
        });
        self.grid_key = Some(key);
    }

    fn ensure_mvp_capacity(&mut self, slots: u32) {
        if slots <= self.slots_cap {
            return;
        }
        self.slots_cap = slots.next_power_of_two();
        self.ubuf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("viewport.mvp"),
            size: (self.slot * self.slots_cap) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.bind = make_bind(&self.device, &self.bgl, &self.ubuf);
    }

    /// Drop the cached GPU meshes. `MeshId`s restart at 1 in each project, so a
    /// stale entry would draw the previous project's geometry under a reused id.
    /// Called by the app-shell project-replace commands (`project_io`) under the
    /// ViewportState + Project locks, so the clear + project swap are atomic
    /// against a concurrent `frame`.
    pub fn clear_meshes(&mut self) {
        self.meshes.clear();
        // Per-plate tower meshes + resolved placements are scoped to the current
        // project; drop them so a reused PlateId can't draw the previous
        // project's tower.
        self.tower_meshes.clear();
        self.invalidate_tower();
    }

    /// Render the live scene and read it back as tight RGBA8, top row first.
    pub fn frame(&mut self, req: &FrameRequest, project: &Arc<Mutex<Project>>) -> Vec<u8> {
        let (w, h) = (req.width.max(1), req.height.max(1));
        self.resize(w, h);

        // --- gather under the project lock (cheap): bed + per-object models ---
        let (bmin, bmax, draws, hole_models, boxes, gizmo, basis, drag_pre, active_plate_id, tower_geom) = {
            let p = project.lock().unwrap();
            let plate = p.active_plate();
            let active_plate_id = plate.id;
            let (bmin, bmax) = plate
                .scene
                .bed
                .as_ref()
                .map(|b| (b.extents.min, b.extents.max))
                .unwrap_or(DEFAULT_BED);
            // The bound printer instance carries the per-slot spool colors.
            // (project→registry lock order matches the rest of the code.)
            let instance = plate
                .printer_instance_id()
                .and_then(instance_registry::lookup_instance);
            // One draw item per (object, paint-state group): (mesh, group index,
            // model, resolved color).
            // (mesh, paint-group, world matrix, color, selected) — `selected`
            // gates the split-tool per-side tint.
            let mut draws: Vec<(MeshId, usize, Mat4, [f32; 3], bool)> = Vec::new();
            // Cut-connector hole volumes (mesh + object model matrix) → translucent
            // overlay pass below.
            let mut hole_models: Vec<(MeshId, Mat4)> = Vec::new();
            // Scale-gizmo basis follows the object's orientation (world for multi).
            let basis = selection_basis(&p);
            // Local drag preview: the active grab + cursor resolve (Rust-side) to a
            // world pre-multiply applied to the whole selection this frame only.
            let (drag_pre, drag_ids): (Mat4, Vec<u64>) = match &req.gizmo_drag {
                Some(gd) => {
                    let cam_center = Vec3::from(req.center);
                    let (ro, rd) =
                        cursor_ray(w as f32, h as f32, gd.sx, gd.sy, req.az, req.el, req.dist, cam_center);
                    let eye = cam_eye(req.az, req.el, req.dist, cam_center);
                    let pre = compute_pre(&gd.grab, ro, rd, eye, cam_center, gd.shift);
                    let ids = plate
                        .scene
                        .objects
                        .iter()
                        .filter(|(id, o)| o.visible && plate.scene.selection.contains(id))
                        .map(|(id, _)| id.0)
                        .collect();
                    (pre, ids)
                }
                None => (Mat4::IDENTITY, Vec::new()),
            };
            let dragging = !drag_ids.is_empty();
            for (id, obj) in plate.scene.objects.iter() {
                if !obj.visible {
                    continue;
                }
                if !self.meshes.contains_key(&obj.mesh) {
                    match p.meshes.get(&obj.mesh) {
                        Some(m) => {
                            self.meshes.insert(obj.mesh, upload_mesh(&self.device, m));
                        }
                        None => tracing::warn!(
                            "viewport: mesh {:?} for object {:?} NOT in registry",
                            obj.mesh,
                            obj.id
                        ),
                    }
                }
                let Some(gm) = self.meshes.get(&obj.mesh) else {
                    continue;
                };
                let mut model = obj.transform.to_mat4();
                if dragging && drag_ids.contains(&id.0) {
                    model = drag_pre * model;
                }
                let selected = plate.scene.selection.contains(id);
                for (gi, g) in gm.groups.iter().enumerate() {
                    // state 0 → object's base material; state N → filament N. Both
                    // via the spool-color chain; selection tints toward blue.
                    let material = if g.state == 0 { obj.extruder_id.unwrap_or(1) } else { g.state };
                    let base = spool_color(&plate.material_to_slot, instance.as_ref(), Some(material));
                    let color = if selected && g.state == 0 {
                        SELECTED_RGB
                    } else if selected {
                        // mix toward selection blue (matches the previous paint tint)
                        let b = SELECTED_RGB;
                        [
                            base[0] * 0.45 + b[0] * 0.55,
                            base[1] * 0.45 + b[1] * 0.55,
                            base[2] * 0.45 + b[2] * 0.55,
                        ]
                    } else {
                        base
                    };
                    draws.push((obj.mesh, gi, model, color, selected));
                }
                // Cut-connector volumes ride the object. Pegs are solid positive
                // volumes, drawn in the object's base color so they read as part
                // of the print. Holes aren't carved into the prepare mesh (they're
                // negative volumes resolved at slice time) — draw them as a
                // translucent overlay marking where they sit.
                for m in plate.scene.object_modifiers.get(id).into_iter().flatten() {
                    if !self.meshes.contains_key(&m.mesh) {
                        if let Some(mesh) = p.meshes.get(&m.mesh) {
                            self.meshes.insert(m.mesh, upload_mesh(&self.device, mesh));
                        }
                    }
                    if !self.meshes.contains_key(&m.mesh) {
                        continue;
                    }
                    match m.kind {
                        ModifierKind::Peg => {
                            let base = spool_color(
                                &plate.material_to_slot,
                                instance.as_ref(),
                                Some(obj.extruder_id.unwrap_or(1)),
                            );
                            let color = if selected { SELECTED_RGB } else { base };
                            draws.push((m.mesh, 0, model, color, selected));
                        }
                        ModifierKind::Hole => hole_models.push((m.mesh, model)),
                    }
                }
            }
            // One outer AABB enclosing the whole selection (world space) → a single
            // set of corner brackets, not one box per group member. Brackets hug
            // the live (drag-previewed) bounds. They're the affordance for the
            // no-tool XY-plane move, so they're hidden once a gizmo is active.
            let boxes = (req.gizmo == GizmoMode::None && req.cut.is_none())
                .then(|| selection_world_aabb(&p, &drag_ids, drag_pre))
                .flatten()
                .map(|(mn, mx)| vec![(Mat4::IDENTITY, mn.to_array(), mx.to_array())])
                .unwrap_or_default();
            // The gizmo is sized + placed from the *resting* selection (no drag
            // preview) so it holds a fixed size through a drag.
            let gizmo = selection_gizmo(&p);
            // Resolve the active plate's tower placement here (cached) so it's
            // drawn this frame — no async frontend round-trip.
            let tower_geom = self.resolved_tower(&p, active_plate_id);
            (
                bmin, bmax, draws, hole_models, boxes, gizmo, basis, drag_pre, active_plate_id,
                tower_geom,
            )
        };
        let bmin = bmin.map(|v| v as f32);
        let bmax = bmax.map(|v| v as f32);

        self.ensure_grid(bmin, bmax);

        // Camera (frontend-owned), z up.
        let vp = view_proj(w as f32, h as f32, req.az, req.el, req.dist, Vec3::from(req.center));

        // Selection bbox corner brackets (world-space lines).
        let bracket_verts = bbox_brackets(&boxes);
        let bracket_vb = (!bracket_verts.is_empty()).then(|| {
            self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("viewport.brackets"),
                contents: bytemuck::cast_slice(&bracket_verts),
                usage: wgpu::BufferUsages::VERTEX,
            })
        });
        let bracket_slot = 1 + draws.len();
        // Three more slots after the brackets: one per origin axis color.
        let axis_slot = bracket_slot + bracket_vb.is_some() as usize;
        // Two tower slots: the box/mesh model (solid + box edges) + the brim.
        let tower_slot = axis_slot + 3;
        // One slot for the split-tool cutting-plane quad, then one per connector
        // peg preview.
        let plane_slot = tower_slot + 2;
        let connector_slot = plane_slot + 1;
        let n_conn = req.cut.as_ref().map_or(0, |c| c.connectors.len());
        // One slot per committed-cut hole overlay, after the split-tool previews.
        let hole_slot = connector_slot + n_conn;
        let total_slots = hole_slot + hole_models.len();

        // Priming tower: draw the active plate's stored sliced mesh (mesh mode)
        // when there is one, else the placement box. Placement is the resolved
        // corner unless a live drag overrides it. (mesh?, vp*model, alpha, brim)
        let bed_z = bmin[2];
        let tower_mesh =
            tower_geom.as_ref().and_then(|g| self.valid_tower_mesh(active_plate_id, g));
        let tower = tower_geom.as_ref().map(|geom| {
            let (width, brim) = (geom.width as f32, geom.brim as f32);
            // Effective (drag-or-clamped) corner — must match viewport_tower_grab
            // so the drawn tower is grabbable. The clamp keeps a persisted
            // wipe_tower override (e.g. from a prior, larger-bed printer) or an
            // off-bed default on the current bed after a printer switch / recover.
            let fp = tower_local_footprint(tower_mesh.and_then(|m| m.footprint), width, brim);
            let (gx, gy) = self.effective_tower_corner(
                active_plate_id,
                geom,
                fp,
                [bmin[0], bmin[1]],
                [bmax[0], bmax[1]],
            );
            let rot = Mat4::from_rotation_z((geom.rotation as f32).to_radians());
            if tower_mesh.is_some() {
                let model = Mat4::from_translation(Vec3::new(gx, gy, bed_z)) * rot;
                (true, vp * model, 0.45f32, None)
            } else {
                let w = width.max(0.1);
                let c = Vec3::new(gx + w * 0.5, gy + w * 0.5, bed_z + TOWER_BOX_H * 0.5);
                let model = Mat4::from_translation(c) * rot * Mat4::from_scale(Vec3::new(w, w, TOWER_BOX_H));
                let (x0, y0) = (gx - brim, gy - brim);
                let (x1, y1) = (gx + width + brim, gy + width + brim);
                let z = bed_z + 0.05;
                let v = |x, y| Vertex { pos: [x, y, z], nrm: [0.0; 3] };
                let brim = vec![
                    v(x0, y0), v(x1, y0), v(x1, y0), v(x1, y1),
                    v(x1, y1), v(x0, y1), v(x0, y1), v(x0, y0),
                ];
                (false, vp * model, 0.28f32, Some(brim))
            }
        });
        let tower_brim_vb = tower.as_ref().and_then(|(_, _, _, brim)| brim.as_ref()).map(|verts| {
            self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("viewport.tower.brim"),
                contents: bytemuck::cast_slice(verts),
                usage: wgpu::BufferUsages::VERTEX,
            })
        });

        // Scale gizmo axes follow the object's orientation (basis derived Rust-side
        // from the selection — its own axes for a single object, world for multi).
        let basis_axes = [basis * Vec3::X, basis * Vec3::Y, basis * Vec3::Z];
        // Eye position (matches view_proj), for the scale gizmo's screen-constant size.
        let eye = cam_eye(req.az, req.el, req.dist, Vec3::from(req.center));
        // Active axis guide line, shown while dragging an axis handle.
        let guide = (req.gizmo_drag.is_some() && (0..=2).contains(&req.gizmo_hover))
            .then_some(req.gizmo_hover as usize);
        // Gizmo at the selection center (draws with slot 0's vp; own colors). Move
        // and Scale are constant on-screen size (length tracks eye distance); Move
        // also follows the dragged object (drag_pre is a translation). Rotate is
        // sized to the object (ring radius `r`) and stays put.
        let screen_l = |c: Vec3| GIZMO_SCREEN_K * (eye - c).length();
        let gizmo_verts = if let Some(cut) = &req.cut {
            // Split tool: the move gizmo sits on the cut plane (rotation is via
            // the panel sliders, so only the translate gizmo is shown). Arm
            // length tracks the selection size.
            let c = Vec3::from(cut.origin);
            let r = gizmo.map(|(_, r)| r).unwrap_or(10.0);
            Some(gizmo_geometry(c, screen_l(c), r, req.gizmo_hover)).filter(|v| !v.is_empty())
        } else {
            gizmo
                .map(|(c, r)| match req.gizmo {
                    GizmoMode::Move => {
                        let c = drag_pre.transform_point3(c);
                        gizmo_geometry(c, screen_l(c), r, req.gizmo_hover)
                    }
                    GizmoMode::Rotate => {
                        gizmo_rotate_geometry(c, r, screen_l(c) * 0.012, req.gizmo_hover)
                    }
                    GizmoMode::Scale => {
                        gizmo_scale_geometry(c, screen_l(c), r, basis_axes, req.gizmo_hover, guide)
                    }
                    GizmoMode::None => Vec::new(),
                })
                .filter(|v| !v.is_empty())
        };
        let gizmo_vb = gizmo_verts.as_ref().map(|verts| {
            self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("viewport.gizmo"),
                contents: bytemuck::cast_slice(verts),
                usage: wgpu::BufferUsages::VERTEX,
            })
        });

        // Split-tool cutting plane: a translucent two-sided quad at the cut
        // origin, sized to the selection and oriented by the plane normal. The
        // red/blue lives on the mesh tint, so the quad itself is neutral.
        let plane_quad = req.cut.as_ref().map(|cut| {
            let n = Vec3::from(cut.normal).normalize_or_zero();
            let up = if n.x.abs() > 0.9 { Vec3::Y } else { Vec3::X };
            let e1 = n.cross(up).normalize();
            let e2 = n.cross(e1).normalize();
            let half = gizmo.map(|(_, r)| r).unwrap_or(20.0) * 1.2;
            let o = Vec3::from(cut.origin);
            let v = |p: Vec3| Vertex { pos: p.to_array(), nrm: n.to_array() };
            vec![
                v(o - e1 * half - e2 * half),
                v(o + e1 * half - e2 * half),
                v(o + e1 * half + e2 * half),
                v(o - e1 * half - e2 * half),
                v(o + e1 * half + e2 * half),
                v(o - e1 * half + e2 * half),
            ]
        });
        let plane_vb = plane_quad.as_ref().map(|verts| {
            self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("viewport.cut.plane"),
                contents: bytemuck::cast_slice(verts),
                usage: wgpu::BufferUsages::VERTEX,
            })
        });

        // Each connector's peg: a unit cylinder scaled to radius/height and
        // oriented from +Z to the plane normal.
        let connector_models: Vec<(Mat4, bool)> = req
            .cut
            .as_ref()
            .map(|cut| {
                let n = Vec3::from(cut.normal).normalize_or_zero();
                let rot = Quat::from_rotation_arc(Vec3::Z, n);
                cut.connectors
                    .iter()
                    .map(|c| {
                        let m = Mat4::from_scale_rotation_translation(
                            Vec3::new(c.radius, c.radius, c.height),
                            rot,
                            Vec3::from(c.pos),
                        );
                        (m, c.selected)
                    })
                    .collect()
            })
            .unwrap_or_default();

        // Pack uniforms per slot: [mat4 mvp][vec4 color]. Slot 0 = grid (vp + grid
        // line color), slots 1.. = each object's vp*model + base color, final slot
        // (when there's a selection) = brackets (vp + bracket color).
        self.ensure_mvp_capacity(total_slots as u32);
        let mut bytes = vec![0u8; self.slot as usize * total_slots];
        bytes[0..64].copy_from_slice(bytemuck::cast_slice(&vp.to_cols_array()));
        bytes[64..80].copy_from_slice(bytemuck::cast_slice(&GRID_LINE));
        for (i, (_, _, model, color, selected)) in draws.iter().enumerate() {
            let off = (i + 1) * self.slot as usize;
            bytes[off..off + 64]
                .copy_from_slice(bytemuck::cast_slice(&(vp * *model).to_cols_array()));
            let rgba = [color[0], color[1], color[2], 1.0f32];
            bytes[off + 64..off + 80].copy_from_slice(bytemuck::cast_slice(&rgba));
            // model matrix (for the split tool's world-space per-side tint).
            bytes[off + 80..off + 144].copy_from_slice(bytemuck::cast_slice(&model.to_cols_array()));
            // Split preview: tint this object only when it's selected and the
            // tool is active; otherwise the trailing planes stay zero (inert).
            if let (Some(cut), true) = (&req.cut, *selected) {
                let plane_o = [cut.origin[0], cut.origin[1], cut.origin[2], 1.0f32];
                let plane_n = [cut.normal[0], cut.normal[1], cut.normal[2], cut.keep_code()];
                bytes[off + 144..off + 160].copy_from_slice(bytemuck::cast_slice(&plane_o));
                bytes[off + 160..off + 176].copy_from_slice(bytemuck::cast_slice(&plane_n));
            }
        }
        if bracket_vb.is_some() {
            let off = bracket_slot * self.slot as usize;
            bytes[off..off + 64].copy_from_slice(bytemuck::cast_slice(&vp.to_cols_array()));
            bytes[off + 64..off + 80].copy_from_slice(bytemuck::cast_slice(&BRACKET));
        }
        for (k, color) in [AXIS_X, AXIS_Y, AXIS_Z].iter().enumerate() {
            let off = (axis_slot + k) * self.slot as usize;
            bytes[off..off + 64].copy_from_slice(bytemuck::cast_slice(&vp.to_cols_array()));
            bytes[off + 64..off + 80].copy_from_slice(bytemuck::cast_slice(color));
        }
        if let Some((_, mvp, alpha, _)) = &tower {
            // model slot: tower color + its alpha (solid + box edges share it).
            let off = tower_slot * self.slot as usize;
            bytes[off..off + 64].copy_from_slice(bytemuck::cast_slice(&mvp.to_cols_array()));
            let rgba = [TOWER_RGB[0], TOWER_RGB[1], TOWER_RGB[2], *alpha];
            bytes[off + 64..off + 80].copy_from_slice(bytemuck::cast_slice(&rgba));
            // brim slot: world lines, opaque tower color.
            let boff = (tower_slot + 1) * self.slot as usize;
            bytes[boff..boff + 64].copy_from_slice(bytemuck::cast_slice(&vp.to_cols_array()));
            let brgba = [TOWER_RGB[0], TOWER_RGB[1], TOWER_RGB[2], 1.0f32];
            bytes[boff + 64..boff + 80].copy_from_slice(bytemuck::cast_slice(&brgba));
        }
        if plane_vb.is_some() {
            // Split-tool plane quad: world verts (vp), neutral translucent fill.
            let off = plane_slot * self.slot as usize;
            bytes[off..off + 64].copy_from_slice(bytemuck::cast_slice(&vp.to_cols_array()));
            let rgba = [0.80f32, 0.80, 0.85, 0.22];
            bytes[off + 64..off + 80].copy_from_slice(bytemuck::cast_slice(&rgba));
        }
        for (k, (model, selected)) in connector_models.iter().enumerate() {
            let off = (connector_slot + k) * self.slot as usize;
            bytes[off..off + 64].copy_from_slice(bytemuck::cast_slice(&(vp * *model).to_cols_array()));
            let rgba = if *selected {
                [1.0f32, 0.85, 0.30, 0.95]
            } else {
                [0.40f32, 0.70, 1.0, 0.80]
            };
            bytes[off + 64..off + 80].copy_from_slice(bytemuck::cast_slice(&rgba));
        }
        for (k, (_, model)) in hole_models.iter().enumerate() {
            let off = (hole_slot + k) * self.slot as usize;
            bytes[off..off + 64]
                .copy_from_slice(bytemuck::cast_slice(&(vp * *model).to_cols_array()));
            let rgba = [0.85f32, 0.25, 0.25, 0.45]; // translucent red — hole marker
            bytes[off + 64..off + 80].copy_from_slice(bytemuck::cast_slice(&rgba));
        }
        self.queue.write_buffer(&self.ubuf, 0, &bytes);

        let color_view = self.color.create_view(&Default::default());
        let mut enc = self.device.create_command_encoder(&Default::default());
        {
            let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.msaa_view,
                    depth_slice: None,
                    resolve_target: Some(&color_view),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.10,
                            g: 0.10,
                            b: 0.12,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            // grid
            rp.set_pipeline(&self.line_pipe);
            rp.set_bind_group(0, &self.bind, &[0]);
            rp.set_vertex_buffer(0, self.vb_grid.slice(..));
            rp.draw(0..self.n_grid, 0..1);
            // origin axis markers: each 2-vert segment with its own color slot.
            // Always-on-top-of-the-grid pipe so they don't z-fight the x=0/y=0
            // grid lines they sit on.
            rp.set_pipeline(&self.axis_pipe);
            rp.set_vertex_buffer(0, self.vb_axes.slice(..));
            for k in 0..3u32 {
                rp.set_bind_group(0, &self.bind, &[(axis_slot as u32 + k) * self.slot]);
                rp.draw(k * 2..k * 2 + 2, 0..1);
            }
            // meshes
            rp.set_pipeline(&self.mesh_pipe);
            for (i, (mesh_id, gi, _, _, _)) in draws.iter().enumerate() {
                let off = ((i + 1) as u32) * self.slot;
                let gm = &self.meshes[mesh_id];
                let g = &gm.groups[*gi];
                rp.set_bind_group(0, &self.bind, &[off]);
                rp.set_vertex_buffer(0, gm.vb.slice(..));
                rp.set_index_buffer(g.ib.slice(..), wgpu::IndexFormat::Uint32);
                rp.draw_indexed(0..g.n_indices, 0, 0..1);
            }
            // selection bbox corner brackets
            if let Some(vb) = &bracket_vb {
                rp.set_pipeline(&self.line_pipe);
                rp.set_bind_group(0, &self.bind, &[(bracket_slot as u32) * self.slot]);
                rp.set_vertex_buffer(0, vb.slice(..));
                rp.draw(0..bracket_verts.len() as u32, 0..1);
            }
            // Priming tower (translucent, after opaque so it blends over).
            if let Some((is_mesh, _, _, _)) = &tower {
                let mslot = &[(tower_slot as u32) * self.slot];
                if *is_mesh {
                    let m = &self.tower_meshes[&active_plate_id].gpu;
                    rp.set_pipeline(&self.tower_pipe);
                    rp.set_bind_group(0, &self.bind, mslot);
                    rp.set_vertex_buffer(0, m.vb.slice(..));
                    rp.set_index_buffer(m.ib.slice(..), wgpu::IndexFormat::Uint32);
                    rp.draw_indexed(0..m.n_indices, 0, 0..1);
                } else {
                    // box solid + its wireframe edges (both off the model slot)…
                    rp.set_pipeline(&self.tower_pipe);
                    rp.set_bind_group(0, &self.bind, mslot);
                    rp.set_vertex_buffer(0, self.vb_cube.slice(..));
                    rp.draw(0..36, 0..1);
                    rp.set_pipeline(&self.line_pipe);
                    rp.set_vertex_buffer(0, self.vb_cube_edges.slice(..));
                    rp.draw(0..24, 0..1);
                    // …and the brim outline on the bed.
                    if let Some(vb) = &tower_brim_vb {
                        rp.set_bind_group(0, &self.bind, &[(tower_slot as u32 + 1) * self.slot]);
                        rp.set_vertex_buffer(0, vb.slice(..));
                        rp.draw(0..8, 0..1);
                    }
                }
            }
            // Split-tool cutting plane (translucent, two-sided — tower_pipe has
            // no backface cull). Drawn last so it blends over everything.
            if let (Some(vb), Some(verts)) = (&plane_vb, &plane_quad) {
                rp.set_pipeline(&self.tower_pipe);
                rp.set_bind_group(0, &self.bind, &[(plane_slot as u32) * self.slot]);
                rp.set_vertex_buffer(0, vb.slice(..));
                rp.draw(0..verts.len() as u32, 0..1);
            }
        }
        // Connector peg previews: their own pass with depth cleared so the pegs
        // show through the opaque mesh (they sit inside the part — depth-tested
        // they'd be invisible). Color preserved so they blend over the scene.
        if !connector_models.is_empty() {
            let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("viewport.connectors"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.msaa_view,
                    depth_slice: None,
                    resolve_target: Some(&color_view),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            rp.set_pipeline(&self.tower_pipe);
            rp.set_vertex_buffer(0, self.vb_cylinder.slice(..));
            for k in 0..connector_models.len() {
                rp.set_bind_group(0, &self.bind, &[(connector_slot as u32 + k as u32) * self.slot]);
                rp.draw(0..self.n_cylinder, 0..1);
            }
        }
        // Committed-cut hole overlays: the hole isn't carved into the half mesh,
        // so draw the negative-volume mesh translucent + depth-cleared (visible
        // through the part) to mark where it sits.
        if !hole_models.is_empty() {
            let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("viewport.cut.holes"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.msaa_view,
                    depth_slice: None,
                    resolve_target: Some(&color_view),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            rp.set_pipeline(&self.tower_pipe);
            for (k, (mesh_id, _)) in hole_models.iter().enumerate() {
                let gm = &self.meshes[mesh_id];
                let g = &gm.groups[0];
                rp.set_bind_group(0, &self.bind, &[(hole_slot + k) as u32 * self.slot]);
                rp.set_vertex_buffer(0, gm.vb.slice(..));
                rp.set_index_buffer(g.ib.slice(..), wgpu::IndexFormat::Uint32);
                rp.draw_indexed(0..g.n_indices, 0, 0..1);
            }
        }
        // Gizmo: second pass, color preserved, always self-occluding (depth Less +
        // write). Move clears depth first so its handles stay on top of the scene
        // and remain grabbable; rotate loads the scene depth so the model occludes
        // the rings' far arcs (a ring reads as wrapping around the part).
        if let (Some(vb), Some(verts)) = (&gizmo_vb, &gizmo_verts) {
            let depth_load = if req.gizmo == GizmoMode::Rotate {
                wgpu::LoadOp::Load
            } else {
                wgpu::LoadOp::Clear(1.0)
            };
            let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("viewport.gizmo"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.msaa_view,
                    depth_slice: None,
                    resolve_target: Some(&color_view),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: depth_load,
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            rp.set_pipeline(&self.gizmo_pipe);
            rp.set_bind_group(0, &self.bind, &[0]);
            rp.set_vertex_buffer(0, vb.slice(..));
            rp.draw(0..verts.len() as u32, 0..1);
        }
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.color,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &self.readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.padded_bpr),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit(Some(enc.finish()));
        read_rgba(&self.device, &self.readback, self.padded_bpr, w, h)
    }

    /// Render a 3/4 iso print thumbnail of the active plate's models only —
    /// transparent background, no bed/grid/gizmo/tower — to `size`x`size` RGBA.
    /// Mirrors the previous `renderModelThumbnail` (the frontend encodes it to a
    /// PNG). Returns a transparent image when there's nothing to show.
    pub fn thumbnail(&mut self, size: u32, project: &Arc<Mutex<Project>>) -> Vec<u8> {
        let size = size.max(1);
        let empty = || vec![0u8; (size * size * 4) as usize];
        let (draws, center, radius) = {
            let p = project.lock().unwrap();
            let plate = p.active_plate();
            let instance = plate.printer_instance_id().and_then(instance_registry::lookup_instance);
            let mut draws: Vec<(MeshId, usize, Mat4, [f32; 3])> = Vec::new();
            let mut mn = Vec3::splat(f32::MAX);
            let mut mx = Vec3::splat(f32::MIN);
            for (_, obj) in plate.scene.objects.iter() {
                if !obj.visible {
                    continue;
                }
                if !self.meshes.contains_key(&obj.mesh) {
                    if let Some(m) = p.meshes.get(&obj.mesh) {
                        self.meshes.insert(obj.mesh, upload_mesh(&self.device, m));
                    }
                }
                let Some(gm) = self.meshes.get(&obj.mesh) else {
                    continue;
                };
                let model = obj.transform.to_mat4();
                if let Some(m) = p.meshes.get(&obj.mesh) {
                    for c in mesh_bb_corners(&m.bounding_box) {
                        let w = model.transform_point3(c);
                        mn = mn.min(w);
                        mx = mx.max(w);
                    }
                }
                let n_groups = gm.groups.len();
                for gi in 0..n_groups {
                    let state = self.meshes[&obj.mesh].groups[gi].state;
                    let material = if state == 0 { obj.extruder_id.unwrap_or(1) } else { state };
                    let color = spool_color(&plate.material_to_slot, instance.as_ref(), Some(material));
                    draws.push((obj.mesh, gi, model, color));
                }
            }
            if draws.is_empty() || mn.x > mx.x {
                return empty();
            }
            let center = (mn + mx) * 0.5;
            let radius = (mx - center).length().max(0.1);
            (draws, center, radius)
        };

        // Iso 3/4 camera framing the bounding sphere (matches thumbnail.ts).
        let fov = 35f32.to_radians();
        let dir = Vec3::new(1.0, -1.0, 0.8).normalize();
        let dist = radius / (fov * 0.5).sin() * 1.15;
        let eye = center + dir * dist;
        let far = dist + radius * 2.0 + 100.0;
        let vp = perspective_rh(fov, 1.0, (dist * 0.01).max(0.1), far)
            * look_at_rh(eye, center, Vec3::Z);

        let (color, msaa_view, depth_view, readback, padded_bpr) =
            make_targets(&self.device, size, size);
        self.ensure_mvp_capacity(draws.len() as u32);
        let mut bytes = vec![0u8; self.slot as usize * draws.len()];
        for (i, (_, _, model, col)) in draws.iter().enumerate() {
            let off = i * self.slot as usize;
            bytes[off..off + 64].copy_from_slice(bytemuck::cast_slice(&(vp * *model).to_cols_array()));
            let rgba = [col[0], col[1], col[2], 1.0f32];
            bytes[off + 64..off + 80].copy_from_slice(bytemuck::cast_slice(&rgba));
        }
        self.queue.write_buffer(&self.ubuf, 0, &bytes);

        let color_view = color.create_view(&Default::default());
        let mut enc = self.device.create_command_encoder(&Default::default());
        {
            let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("viewport.thumbnail"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &msaa_view,
                    depth_slice: None,
                    resolve_target: Some(&color_view),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            rp.set_pipeline(&self.mesh_pipe);
            for (i, (mesh_id, gi, _, _)) in draws.iter().enumerate() {
                let gm = &self.meshes[mesh_id];
                let g = &gm.groups[*gi];
                rp.set_bind_group(0, &self.bind, &[(i as u32) * self.slot]);
                rp.set_vertex_buffer(0, gm.vb.slice(..));
                rp.set_index_buffer(g.ib.slice(..), wgpu::IndexFormat::Uint32);
                rp.draw_indexed(0..g.n_indices, 0, 0..1);
            }
        }
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &color,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_bpr),
                    rows_per_image: Some(size),
                },
            },
            wgpu::Extent3d { width: size, height: size, depth_or_array_layers: 1 },
        );
        self.queue.submit(Some(enc.finish()));
        read_rgba(&self.device, &readback, padded_bpr, size, size)
    }
}

fn make_bind(
    device: &wgpu::Device,
    bgl: &wgpu::BindGroupLayout,
    ubuf: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: bgl,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                buffer: ubuf,
                offset: 0,
                size: std::num::NonZeroU64::new(UNIFORM_BYTES),
            }),
        }],
    })
}

/// Tauri-managed renderer (lazily created on first frame; wgpu init is ~100ms).
#[derive(Default)]
pub struct ViewportState(pub Mutex<Option<ViewportRenderer>>);

/// Render one frame and return tight RGBA8 bytes.
#[tauri::command]
pub fn viewport_frame(
    state: tauri::State<'_, ViewportState>,
    project: tauri::State<'_, Arc<Mutex<Project>>>,
    req: FrameRequest,
) -> tauri::ipc::Response {
    let mut guard = state.0.lock().unwrap();
    let r = guard.get_or_insert_with(ViewportRenderer::new);
    tauri::ipc::Response::new(r.frame(&req, project.inner()))
}

/// Store (or drop, when `mesh` is `None`) a plate's sliced tower mesh in the
/// renderer, keyed by plate. The app-shell seam the slice event sink calls so
/// the mesh reaches the GPU without a frontend round-trip; the frontend reacts
/// to the same `plate_finished` event to re-render.
pub fn store_plate_tower_mesh(
    state: &ViewportState,
    plate_id: PlateId,
    mesh: Option<(&[f32], &[u32])>,
    material_count: usize,
    printer_instance_id: Option<String>,
) {
    let mut guard = state.0.lock().unwrap();
    let r = guard.get_or_insert_with(ViewportRenderer::new);
    match mesh {
        Some((vertices, indices)) => {
            r.store_tower_mesh(plate_id, vertices, indices, material_count, printer_instance_id)
        }
        // Sliced single-material → no tower; drop any stale mesh for this plate.
        None => {
            r.tower_meshes.remove(&plate_id);
        }
    }
}

/// Move the active plate's tower to a requested corner (no mesh re-upload) for a
/// smooth bed-plane drag. The corner is clamped so the footprint stays on the
/// bed; the clamped corner is stored as the live drag override and returned so
/// the frontend can gate + commit it. `None` when there's no tower or bed.
#[tauri::command]
pub fn viewport_move_tower(
    state: tauri::State<'_, ViewportState>,
    project: tauri::State<'_, Arc<Mutex<Project>>>,
    x: f32,
    y: f32,
) -> Option<(f32, f32)> {
    // Lock order: ViewportState before Project (matches `viewport_frame`).
    let mut guard = state.0.lock().unwrap();
    let r = guard.as_mut()?;
    let (id, _geom, bed_min, bed_max, fp) = {
        let p = project.lock().unwrap();
        r.tower_drag_ctx(&p)? // no tower / no bed → nothing to move
    };
    let (cx, cy) = clamp_tower_corner(
        fp,
        [bed_min[0] as f32, bed_min[1] as f32],
        [bed_max[0] as f32, bed_max[1] as f32],
        x,
        y,
    );
    r.tower_drag = Some((id, (cx, cy)));
    Some((cx, cy))
}

/// Press-time tower grab: if `(bx, by)` (a bed-plane point) lands on the active
/// plate's tower footprint (+brim), return the grab offset `(bx-corner)` so the
/// frontend can drive the drag (corner = bed point − offset → `viewport_move_tower`).
/// `None` = the press missed the tower (frontend orbits instead).
#[tauri::command]
pub fn viewport_tower_grab(
    state: tauri::State<'_, ViewportState>,
    project: tauri::State<'_, Arc<Mutex<Project>>>,
    bx: f32,
    by: f32,
) -> Option<(f32, f32)> {
    // Lock order: ViewportState before Project (matches `viewport_frame`).
    let mut guard = state.0.lock().unwrap();
    let r = guard.as_mut()?;
    let (id, geom, bed_min, bed_max, fp) = {
        let p = project.lock().unwrap();
        r.tower_drag_ctx(&p)?
    };
    // The same effective (drag-or-clamped) corner `frame` draws, so a click on
    // the visible tower hits even when its resolved origin is off-bed.
    let (cx, cy) = r.effective_tower_corner(
        id,
        &geom,
        fp,
        [bed_min[0] as f32, bed_min[1] as f32],
        [bed_max[0] as f32, bed_max[1] as f32],
    );
    tower_corner_hit(fp, cx, cy, bx, by).then_some((bx - cx, by - cy))
}

/// Drop the resolved-placement cache + any live drag so the next frame
/// re-resolves. The frontend calls this on tower-affecting edits (overrides,
/// materials, printer, bed, project load) — NOT on a plain plate switch.
#[tauri::command]
pub fn viewport_invalidate_tower(state: tauri::State<'_, ViewportState>) {
    if let Some(r) = state.0.lock().unwrap().as_mut() {
        r.invalidate_tower();
    }
}

/// Render a square iso print thumbnail (models only, transparent background) and
/// return it as tight RGBA8 — the frontend encodes it to a PNG for the send path.
#[tauri::command]
pub fn viewport_thumbnail(
    state: tauri::State<'_, ViewportState>,
    project: tauri::State<'_, Arc<Mutex<Project>>>,
    size: u32,
) -> tauri::ipc::Response {
    let mut guard = state.0.lock().unwrap();
    let r = guard.get_or_insert_with(ViewportRenderer::new);
    tauri::ipc::Response::new(r.thumbnail(size, project.inner()))
}

/// The active plate's bed extents — the frontend frames the camera (center +
/// distance) from this on load / bed change with a view-aware fit.
#[derive(serde::Serialize)]
pub struct SceneInfo {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

#[tauri::command]
pub fn viewport_scene_info(project: tauri::State<'_, Arc<Mutex<Project>>>) -> SceneInfo {
    let p = project.lock().unwrap();
    let (min, max) = p
        .active_plate()
        .scene
        .bed
        .as_ref()
        .map(|b| (b.extents.min, b.extents.max))
        .unwrap_or(DEFAULT_BED);
    SceneInfo {
        min: min.map(|v| v as f32),
        max: max.map(|v| v as f32),
    }
}

/// Move gizmo placement for the current selection: world center + axis length.
/// `None` when nothing is selected. The frontend rebuilds the (fixed) handle
/// layout from these to hit-test and drive constrained drags.
#[derive(serde::Serialize)]
pub struct GizmoInfo {
    pub center: [f32; 3],
    pub length: f32,
}

#[tauri::command]
pub fn viewport_gizmo(project: tauri::State<'_, Arc<Mutex<Project>>>) -> Option<GizmoInfo> {
    let p = project.lock().unwrap();
    let (c, l) = selection_gizmo(&p)?;
    Some(GizmoInfo { center: c.to_array(), length: l })
}

/// Camera + cursor (canvas pixels) + the active gizmo mode, for a grab/hover test.
#[derive(serde::Deserialize)]
pub struct GrabRequest {
    pub width: u32,
    pub height: u32,
    pub x: f32,
    pub y: f32,
    pub az: f32,
    pub el: f32,
    pub dist: f32,
    pub center: [f32; 3],
    #[serde(default)]
    pub gizmo: GizmoMode,
}

/// Hit-test the cursor for a press (or hover): the active gizmo's handles, else
/// the selected body / empty space. The returned `GizmoGrab` is opaque to the
/// frontend — it passes it back via `gizmo_drag` (preview) and to
/// `viewport_gizmo_commit` (release). Also serves the idle hover highlight (read
/// the grab's handle `idx`).
#[tauri::command]
pub fn viewport_grab(
    project: tauri::State<'_, Arc<Mutex<Project>>>,
    req: GrabRequest,
) -> GrabResult {
    let p = project.lock().unwrap();
    let (w, h) = (req.width.max(1) as f32, req.height.max(1) as f32);
    let center = Vec3::from(req.center);
    let (ro, rd) = cursor_ray(w, h, req.x, req.y, req.az, req.el, req.dist, center);
    let eye = cam_eye(req.az, req.el, req.dist, center);
    let plate = p.active_plate();

    if req.gizmo != GizmoMode::None {
        if let Some(grab) = pick_gizmo(&p, ro, rd, eye, req.gizmo) {
            return GrabResult::Gizmo { grab };
        }
        // Missed every handle: pressing the selected body is inert (handles drive
        // transforms); pressing an unselected object orbits; pressing empty space
        // returns `Empty` so the frontend can still drag the priming tower (the
        // tower is an overlay, not a scene object — it must be draggable in any
        // gizmo mode, same as with no gizmo).
        return match nearest_hit(&p, ro, rd) {
            Some((id, _, _)) if plate.scene.selection.contains(&id) => GrabResult::Inert,
            Some(_) => GrabResult::Orbit,
            None => GrabResult::Empty,
        };
    }

    // No gizmo: pressing a selected object free-moves the selection on its XY
    // plane (a Move grab with no axis constraint); an unselected object orbits
    // (selection happens on click); empty space → frontend checks the tower.
    match nearest_hit(&p, ro, rd) {
        Some((id, _, _)) if plate.scene.selection.contains(&id) => {
            let plane_z = plate.scene.objects.get(&id).map_or(0.0, |o| o.transform.to_mat4().w_axis.z);
            let plane_n = Vec3::Z;
            let plane_p = Vec3::new(0.0, 0.0, plane_z);
            GrabResult::Gizmo {
                grab: GizmoGrab {
                    idx: -1,
                    kind: GrabKind::Move,
                    plane_n: plane_n.to_array(),
                    plane_p: plane_p.to_array(),
                    axis_dir: None,
                    rot_axis: None,
                    scale_mask: None,
                    uniform: false,
                    pivot: plane_p.to_array(),
                    start_hit: ray_plane(ro, rd, plane_n, plane_p).unwrap_or(plane_p).to_array(),
                    basis: Quat::IDENTITY.to_array(),
                    scale_extent: [0.0; 3],
                },
            }
        }
        Some(_) => GrabResult::Orbit,
        None => GrabResult::Empty,
    }
}

/// Camera + cursor + the grabbed handle, for a drag commit.
#[derive(serde::Deserialize)]
pub struct CommitRequest {
    pub width: u32,
    pub height: u32,
    pub sx: f32,
    pub sy: f32,
    pub az: f32,
    pub el: f32,
    pub dist: f32,
    pub center: [f32; 3],
    pub grab: GizmoGrab,
    #[serde(default)]
    pub shift: bool,
}

/// One object's committed world transform (column-major 4x4).
#[derive(serde::Serialize)]
pub struct TransformUpdate {
    pub id: u64,
    pub transform: [f32; 16],
}

/// Resolve the final drag matrix and return `pre · start` for each selected
/// object. The frontend commits them via `scene_object_set_transform` (so the
/// mutation + events stay in the scene layer). Read-only on the project.
#[tauri::command]
pub fn viewport_gizmo_commit(
    project: tauri::State<'_, Arc<Mutex<Project>>>,
    req: CommitRequest,
) -> Vec<TransformUpdate> {
    let p = project.lock().unwrap();
    let cam_center = Vec3::from(req.center);
    let (ro, rd) = cursor_ray(
        req.width.max(1) as f32, req.height.max(1) as f32, req.sx, req.sy,
        req.az, req.el, req.dist, cam_center,
    );
    let eye = cam_eye(req.az, req.el, req.dist, cam_center);
    let pre = compute_pre(&req.grab, ro, rd, eye, cam_center, req.shift);
    let plate = p.active_plate();
    plate
        .scene
        .objects
        .iter()
        .filter(|(id, o)| o.visible && plate.scene.selection.contains(id))
        .map(|(id, o)| TransformUpdate {
            id: id.0,
            transform: (pre * o.transform.to_mat4()).to_cols_array(),
        })
        .collect()
}

/// Camera + cursor + a world plane, for tower dragging (the frontend's only
/// remaining ray need). Returns the cursor ray's hit on the plane, or `None`.
#[derive(serde::Deserialize)]
pub struct RayPlaneRequest {
    pub width: u32,
    pub height: u32,
    pub x: f32,
    pub y: f32,
    pub az: f32,
    pub el: f32,
    pub dist: f32,
    pub center: [f32; 3],
    pub plane_n: [f32; 3],
    pub plane_p: [f32; 3],
}

#[tauri::command]
pub fn viewport_ray_plane(req: RayPlaneRequest) -> Option<[f32; 3]> {
    let (w, h) = (req.width.max(1) as f32, req.height.max(1) as f32);
    let (ro, rd) = cursor_ray(w, h, req.x, req.y, req.az, req.el, req.dist, Vec3::from(req.center));
    ray_plane(ro, rd, Vec3::from(req.plane_n), Vec3::from(req.plane_p)).map(|v| v.to_array())
}

/// Camera + cursor + the split tool's cutting-plane center/size, for a press
/// hit-test on the plane's Move handles.
#[derive(serde::Deserialize)]
pub struct CutGrabRequest {
    pub width: u32,
    pub height: u32,
    pub x: f32,
    pub y: f32,
    pub az: f32,
    pub el: f32,
    pub dist: f32,
    pub center: [f32; 3],
    /// The cutting plane's current origin (the gizmo sits here).
    pub origin: [f32; 3],
    /// Handle arm length (the selection's bounding-sphere radius).
    pub arm: f32,
}

/// Hit-test the cutting plane's move handles for a press. Returns the opaque
/// `GizmoGrab` (frontend passes it back to `viewport_cut_drag`), or `None` when
/// the press missed every handle (frontend orbits / re-centers instead). Pure.
#[tauri::command]
pub fn viewport_cut_grab(req: CutGrabRequest) -> Option<GizmoGrab> {
    let (w, h) = (req.width.max(1) as f32, req.height.max(1) as f32);
    let cam_center = Vec3::from(req.center);
    let (ro, rd) = cursor_ray(w, h, req.x, req.y, req.az, req.el, req.dist, cam_center);
    let eye = cam_eye(req.az, req.el, req.dist, cam_center);
    pick_move_at(Vec3::from(req.origin), req.arm, ro, rd, eye)
}

/// Camera + cursor + the grabbed handle + the plane's current origin, for a
/// cutting-plane drag. Returns the new plane origin (the move pre-multiply
/// applied to the old origin). Pure — the plane pose lives frontend-side until
/// the cut is applied.
#[derive(serde::Deserialize)]
pub struct CutDragRequest {
    pub width: u32,
    pub height: u32,
    pub sx: f32,
    pub sy: f32,
    pub az: f32,
    pub el: f32,
    pub dist: f32,
    pub center: [f32; 3],
    pub grab: GizmoGrab,
    pub origin: [f32; 3],
    #[serde(default)]
    pub shift: bool,
}

#[tauri::command]
pub fn viewport_cut_drag(req: CutDragRequest) -> [f32; 3] {
    let cam_center = Vec3::from(req.center);
    let (ro, rd) = cursor_ray(
        req.width.max(1) as f32,
        req.height.max(1) as f32,
        req.sx,
        req.sy,
        req.az,
        req.el,
        req.dist,
        cam_center,
    );
    let eye = cam_eye(req.az, req.el, req.dist, cam_center);
    let pre = compute_pre(&req.grab, ro, rd, eye, cam_center, req.shift);
    pre.transform_point3(Vec3::from(req.origin)).to_array()
}

/// Camera + cursor + the cut plane + the objects being cut, for placing a
/// connector. Returns the cursor's hit on the plane, but only when that point
/// lies *inside* the model (so connectors can't be dropped in empty space).
#[derive(serde::Deserialize)]
pub struct CutPlaceRequest {
    pub width: u32,
    pub height: u32,
    pub x: f32,
    pub y: f32,
    pub az: f32,
    pub el: f32,
    pub dist: f32,
    pub center: [f32; 3],
    pub plane_n: [f32; 3],
    pub plane_p: [f32; 3],
    pub ids: Vec<u64>,
}

#[tauri::command]
pub fn viewport_cut_place(
    project: tauri::State<'_, Arc<Mutex<Project>>>,
    req: CutPlaceRequest,
) -> Option<[f32; 3]> {
    let p = project.lock().ok()?;
    let (w, h) = (req.width.max(1) as f32, req.height.max(1) as f32);
    let cam_center = Vec3::from(req.center);
    let (ro, rd) = cursor_ray(w, h, req.x, req.y, req.az, req.el, req.dist, cam_center);
    let n = Vec3::from(req.plane_n);
    let pp = Vec3::from(req.plane_p);
    let denom = n.dot(rd);
    if denom.abs() < 1e-9 {
        return None; // looking along the plane
    }
    let hit = ro + rd * (n.dot(pp - ro) / denom);
    // Place only when that point is inside the solid of one of the objects being
    // cut — a true point-in-mesh test, so clicks over a hollow / notch / gap in
    // the cross-section don't place. The cut expands groups; match that target set.
    let want: Vec<ObjectId> = req.ids.iter().map(|i| ObjectId(*i)).collect();
    let ids: std::collections::HashSet<u64> =
        p.group_expanded_ids(&want).iter().map(|i| i.0).collect();
    let plate = p.active_plate();
    for (id, obj) in plate.scene.objects.iter() {
        if !obj.visible || !ids.contains(&id.0) {
            continue;
        }
        let Some(m) = p.meshes.get(&obj.mesh) else { continue };
        let model = obj.transform.to_mat4();
        let vert = |vi: u32| {
            let i = vi as usize * 3;
            model.transform_point3(Vec3::new(m.vertices[i], m.vertices[i + 1], m.vertices[i + 2]))
        };
        if point_in_mesh(hit, &m.indices, vert) {
            return Some(hit.to_array());
        }
    }
    None
}

/// Inside-test for a solid: from `p`, cast several rays and count surface
/// crossings — odd means inside. A majority vote across spread directions
/// shrugs off the odd ray that grazes a shared edge/vertex (which would mis-count
/// on a single probe). `vert` maps an index to its world position.
fn point_in_mesh(p: Vec3, indices: &[u32], vert: impl Fn(u32) -> Vec3) -> bool {
    let dirs = [
        Vec3::new(0.1357, 0.5731, 0.8079),
        Vec3::new(-0.7071, 0.4999, 0.5001),
        Vec3::new(0.3001, -0.8003, 0.5197),
        Vec3::new(-0.4003, -0.5998, 0.6911),
        Vec3::new(0.9001, 0.2003, -0.3869),
    ];
    let mut inside = 0u32;
    for d in dirs {
        let d = d.normalize();
        let mut crossings = 0u32;
        for t3 in indices.chunks_exact(3) {
            if let Some(t) = ray_tri(p, d, vert(t3[0]), vert(t3[1]), vert(t3[2])) {
                if t > 1e-5 {
                    crossings += 1;
                }
            }
        }
        if crossings % 2 == 1 {
            inside += 1;
        }
    }
    inside * 2 > dirs.len() as u32
}

/// Möller–Trumbore, two-sided. Returns the ray parameter `t` (>0) at the hit.
fn ray_tri(o: Vec3, d: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Option<f32> {
    let (e1, e2) = (b - a, c - a);
    let pv = d.cross(e2);
    let det = e1.dot(pv);
    if det.abs() < 1e-7 {
        return None;
    }
    let inv = 1.0 / det;
    let tv = o - a;
    let u = tv.dot(pv) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let qv = tv.cross(e1);
    let v = d.dot(qv) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = e2.dot(qv) * inv;
    (t > 1e-4).then_some(t)
}

#[derive(serde::Deserialize)]
pub struct PickRequest {
    pub width: u32,
    pub height: u32,
    pub x: f32, // click, pixels, top-left origin
    pub y: f32,
    pub az: f32,
    pub el: f32,
    pub dist: f32,
    pub center: [f32; 3],
}

/// Ray-cast the click into the scene; returns the nearest hit object's id (CPU,
/// against the scene's mesh geometry). The frontend turns this into
/// `scene_select`/`scene_deselect`.
#[tauri::command]
pub fn viewport_pick(
    project: tauri::State<'_, Arc<Mutex<Project>>>,
    req: PickRequest,
) -> Option<u64> {
    viewport_pick_face(project, req).map(|f| f.id)
}

/// Nearest face hit: the object id, the hit triangle's world-space outward
/// normal, and the world hit point. Drives lay-flat (and later face-align):
/// the frontend rotates the picked face's normal to point down and drops the
/// contact onto the bed.
#[derive(serde::Serialize)]
pub struct FacePick {
    pub id: u64,
    pub normal: [f32; 3],
    pub point: [f32; 3],
}

#[tauri::command]
pub fn viewport_pick_face(
    project: tauri::State<'_, Arc<Mutex<Project>>>,
    req: PickRequest,
) -> Option<FacePick> {
    let p = project.lock().unwrap();
    let (w, h) = (req.width.max(1) as f32, req.height.max(1) as f32);
    let (ro, rd) = cursor_ray(w, h, req.x, req.y, req.az, req.el, req.dist, Vec3::from(req.center));
    nearest_hit(&p, ro, rd).map(|(id, normal, point)| FacePick {
        id: id.0,
        normal: normal.to_array(),
        point: point.to_array(),
    })
}


#[cfg(test)]
mod tower_geometry_tests {
    use super::*;

    #[test]
    fn footprint_is_the_xy_bbox_ignoring_z() {
        // Two verts spanning X 1..4, Y 2..7; Z is irrelevant.
        let verts = [1.0, 2.0, 9.0, 4.0, 7.0, -3.0];
        assert_eq!(tower_footprint(&verts), Some([1.0, 2.0, 4.0, 7.0]));
        assert_eq!(tower_footprint(&[]), None);
    }

    #[test]
    fn local_footprint_falls_back_to_the_square_box() {
        // No mesh → box of width 30 + brim 5 → -5..35 on both axes.
        assert_eq!(
            tower_local_footprint(None, 30.0, 5.0),
            [-5.0, -5.0, 35.0, 35.0]
        );
        // Mesh footprint passes through verbatim.
        let fp = [1.0, 2.0, 4.0, 7.0];
        assert_eq!(tower_local_footprint(Some(fp), 30.0, 5.0), fp);
    }

    #[test]
    fn clamp_keeps_the_footprint_on_the_bed() {
        // Bed 0..100; footprint local 0..10. Corner range is 0..90.
        let fp = [0.0, 0.0, 10.0, 10.0];
        let bmin = [0.0, 0.0];
        let bmax = [100.0, 100.0];
        assert_eq!(clamp_tower_corner(fp, bmin, bmax, 50.0, 50.0), (50.0, 50.0));
        assert_eq!(clamp_tower_corner(fp, bmin, bmax, -20.0, -5.0), (0.0, 0.0)); // low edge
        assert_eq!(clamp_tower_corner(fp, bmin, bmax, 200.0, 95.0), (90.0, 90.0)); // high edge
    }

    #[test]
    fn clamp_low_edge_wins_when_footprint_exceeds_the_bed() {
        // Footprint 0..120 wider than the 0..100 bed → only the low edge (0) fits.
        let fp = [0.0, 0.0, 120.0, 120.0];
        assert_eq!(
            clamp_tower_corner(fp, [0.0, 0.0], [100.0, 100.0], 50.0, 50.0),
            (0.0, 0.0)
        );
    }

    #[test]
    fn hit_test_is_corner_plus_local_footprint() {
        // Corner (10,20), local footprint 0..5 → world box X 10..15, Y 20..25.
        let fp = [0.0, 0.0, 5.0, 5.0];
        assert!(tower_corner_hit(fp, 10.0, 20.0, 12.0, 22.0));
        assert!(tower_corner_hit(fp, 10.0, 20.0, 10.0, 25.0)); // on the edge
        assert!(!tower_corner_hit(fp, 10.0, 20.0, 16.0, 22.0)); // past max X
        assert!(!tower_corner_hit(fp, 10.0, 20.0, 12.0, 19.0)); // below min Y
    }

    #[test]
    fn point_in_mesh_separates_solid_from_empty() {
        // Unit cube [-1,1]^3 as a triangle soup (12 tris).
        let v = [
            [-1.0, -1.0, -1.0], [1.0, -1.0, -1.0], [1.0, 1.0, -1.0], [-1.0, 1.0, -1.0],
            [-1.0, -1.0, 1.0], [1.0, -1.0, 1.0], [1.0, 1.0, 1.0], [-1.0, 1.0, 1.0],
        ];
        let faces: [[u32; 3]; 12] = [
            [0, 1, 2], [0, 2, 3], // -z
            [4, 6, 5], [4, 7, 6], // +z
            [0, 4, 5], [0, 5, 1], // -y
            [3, 2, 6], [3, 6, 7], // +y
            [0, 3, 7], [0, 7, 4], // -x
            [1, 5, 6], [1, 6, 2], // +x
        ];
        let idx: Vec<u32> = faces.iter().flatten().copied().collect();
        let vert = |i: u32| Vec3::from(v[i as usize]);
        assert!(point_in_mesh(Vec3::ZERO, &idx, vert)); // center
        assert!(point_in_mesh(Vec3::new(0.9, -0.5, 0.3), &idx, vert)); // near a wall, inside
        assert!(!point_in_mesh(Vec3::new(2.0, 0.0, 0.0), &idx, vert)); // outside +x
        assert!(!point_in_mesh(Vec3::new(0.0, 0.0, 5.0), &idx, vert)); // outside +z
        // A point coplanar with the cut but outside the silhouette — the case the
        // user hit: on the plane, no material there.
        assert!(!point_in_mesh(Vec3::new(3.0, 0.0, 0.0), &idx, vert));
    }
}
