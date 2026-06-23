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

use glam::{Mat4, Quat, Vec3};
use wgpu::util::DeviceExt;

use crate::core::printer::{instance_registry, PrinterInstance, SlotRef};
use crate::core::project::Project;
use crate::core::scene::state::MeshId;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    pos: [f32; 3],
    nrm: [f32; 3],
}

const SHADER: &str = r#"
struct U { mvp: mat4x4<f32>, color: vec4<f32> };
@group(0) @binding(0) var<uniform> u: U;
struct VO { @builtin(position) p: vec4<f32>, @location(0) n: vec3<f32> };
@vertex fn vs(@location(0) pos: vec3<f32>, @location(1) nrm: vec3<f32>) -> VO {
  var o: VO; o.p = u.mvp * vec4<f32>(pos, 1.0); o.n = nrm; return o;
}
@fragment fn fs(i: VO) -> @location(0) vec4<f32> {
  if (dot(i.n, i.n) < 0.01) { return vec4<f32>(u.color.rgb, 1.0); } // unlit line (grid / bbox bracket)
  let n = normalize(i.n);
  // Matte, two-sided lighting matching the Three.js viewport (ambient 0.55 +
  // key + fill). High ambient + abs() compresses contrast so the source model's
  // inconsistent/flipped normals don't carve stark facets (the "missing shapes"
  // artifact); a shinier, one-sided model exposed them.
  let key = normalize(vec3<f32>(0.5, -0.5, 0.85));
  let fill = normalize(vec3<f32>(-0.5, 0.5, 0.33));
  let d = 0.55 + abs(dot(n, key)) * 0.4 + abs(dot(n, fill)) * 0.1;
  // color = the face group's resolved color (object base / per-filament paint,
  // already selection-tinted CPU-side).
  return vec4<f32>(u.color.rgb * min(d, 1.0), 1.0);
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

const COLOR_FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
/// 4x MSAA — cheap edge anti-aliasing; the multisampled target resolves into the
/// single-sample `color` that's read back.
const SAMPLES: u32 = 4;
/// Move/Scale gizmo handle length as a fraction of the eye→gizmo distance, so it
/// holds a constant on-screen size (a TransformControls port). Match the frontend.
const GIZMO_SCREEN_K: f32 = 0.13;
/// Per-object uniform: `mat4 mvp` (64) + `vec4 tint` (16).
const UNIFORM_BYTES: u64 = 80;
// Bed extents are f64 (BoundingBox); converted to f32 for the GPU.
const DEFAULT_BED: ([f64; 3], [f64; 3]) = ([-110.0, -110.0, 0.0], [110.0, 110.0, 200.0]);
const SELECTED_RGB: [f32; 3] = [0.231, 0.510, 0.965]; // tailwind blue-500 (#3b82f6)
const DEFAULT_RGB: [f32; 3] = [0.694, 0.694, 0.694]; // #b1b1b1, matches the Three.js fallback
const GRID_LINE: [f32; 4] = [0.34, 0.36, 0.40, 1.0];
const BRACKET: [f32; 4] = [0.85, 0.88, 0.97, 1.0]; // selection bbox corner brackets
// Origin axis-marker colors (mirror the Three.js viewport + corner legend).
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

/// Which gizmo to draw at the selection center.
#[derive(serde::Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum GizmoMode {
    #[default]
    None,
    Move,
    Rotate,
    Scale,
}

/// Camera + target the frontend passes per frame (camera is frontend-owned).
/// `drag_pre` is a transient local drag preview: the listed objects are
/// world-space pre-multiplied by it for this frame only — no scene-state change —
/// so dragging stays smooth; the real transforms commit once on release.
#[derive(serde::Deserialize)]
pub struct FrameRequest {
    pub width: u32,
    pub height: u32,
    pub az: f32,
    pub el: f32,
    pub dist: f32,
    pub center: [f32; 3],
    #[serde(default)]
    pub drag_ids: Vec<u64>,
    /// World-space transform applied to `drag_ids` this frame (column-major 4x4).
    #[serde(default = "identity16")]
    pub drag_pre: [f32; 16],
    /// Which gizmo to show at the selection center.
    #[serde(default)]
    pub gizmo: GizmoMode,
    /// Orientation for the scale gizmo's axes (quaternion xyzw): a single
    /// selection scales along its own (rotated) axes; multi/identity is world.
    #[serde(default = "ident_quat")]
    pub gizmo_basis: [f32; 4],
    /// A handle drag is in progress — draws the active axis guide line.
    #[serde(default)]
    pub gizmo_dragging: bool,
    /// Hovered gizmo handle to highlight: for Move 0/1/2 = X/Y/Z axis, 3/4/5 =
    /// XY/YZ/XZ plane; for Rotate 0/1/2 = X/Y/Z ring; -1 = none.
    #[serde(default = "neg_one")]
    pub gizmo_hover: i32,
}

fn neg_one() -> i32 {
    -1
}

fn identity16() -> [f32; 16] {
    Mat4::IDENTITY.to_cols_array()
}

fn ident_quat() -> [f32; 4] {
    [0.0, 0.0, 0.0, 1.0]
}

/// One paint-state index group within a mesh: the triangles painted with
/// filament `state` (0 = unpainted → object's base color), as an index buffer
/// into the shared vertex buffer.
struct MeshGroup {
    state: u8,
    ib: wgpu::Buffer,
    n_indices: u32,
}

struct GpuMesh {
    vb: wgpu::Buffer,
    // One group for an unpainted mesh (state 0); one per filament for a painted
    // mesh. The vertices stay shared/indexed — only the triangles are partitioned,
    // so painting never multiplies the vertex count.
    groups: Vec<MeshGroup>,
}

/// Build our interleaved vertex buffer for a `Mesh` and upload it. Normals are
/// **recomputed** (area-weighted smooth vertex normals from the geometry), not
/// taken from `m.normals`: imported models often carry bad/flat per-vertex
/// normals that shade as faceted "missing shapes" even though the geometry is
/// fine (it slices smooth). The mesh is welded (shared vertices), so accumulating
/// face normals per vertex yields the smooth shading the slice shows.
fn upload_mesh(device: &wgpu::Device, m: &crate::core::scene::state::Mesh) -> GpuMesh {
    let vcount = m.vertices.len() / 3;
    let pos: Vec<Vec3> = (0..vcount)
        .map(|i| Vec3::new(m.vertices[3 * i], m.vertices[3 * i + 1], m.vertices[3 * i + 2]))
        .collect();
    let mut nrm = vec![Vec3::ZERO; vcount];
    for tri in m.indices.chunks_exact(3) {
        let (a, b, c) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
        if a < vcount && b < vcount && c < vcount {
            let face = (pos[b] - pos[a]).cross(pos[c] - pos[a]); // area-weighted
            nrm[a] += face;
            nrm[b] += face;
            nrm[c] += face;
        }
    }
    let verts: Vec<Vertex> = (0..vcount)
        .map(|i| Vertex { pos: pos[i].to_array(), nrm: nrm[i].normalize_or_zero().to_array() })
        .collect();
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

/// Square rod from `start` along `dir` for `len`, half-width `hw`. Six faces,
/// per-face normals — a solid bar instead of a 1px line.
fn push_rod(v: &mut Vec<GizmoVertex>, start: Vec3, dir: Vec3, len: f32, hw: f32, col: [f32; 3]) {
    // Two axes orthogonal to dir.
    let up = if dir.x.abs() > 0.9 { Vec3::Y } else { Vec3::X };
    let u = dir.cross(up).normalize();
    let w = dir.cross(u).normalize();
    let end = start + dir * len;
    for (base, n) in [(start, -dir), (end, dir)] {
        let p = [base + u * hw + w * hw, base - u * hw + w * hw, base - u * hw - w * hw, base + u * hw - w * hw];
        push_quad(v, p[0], p[1], p[2], p[3], n, col);
    }
    for (n, t) in [(u, w), (-u, w), (w, u), (-w, u)] {
        let a = start + n * hw + t * hw;
        let b = start + n * hw - t * hw;
        let c = end + n * hw - t * hw;
        let d = end + n * hw + t * hw;
        push_quad(v, a, b, c, d, n, col);
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

/// Move gizmo at `center` with axis length `l`: one rod+ball handle per axis
/// (X/Y/Z) + a filled square per plane (XY/YZ/XZ). `hover` brightens one handle
/// (0/1/2 = X/Y/Z axis, 3/4/5 = XY/YZ/XZ plane, -1 = none).
fn gizmo_geometry(center: Vec3, l: f32, hover: i32) -> Vec<GizmoVertex> {
    let mut v = Vec::new();
    let rod_hw = l * 0.012; // matches the rotate ring tube
    let cone_h = l * 0.22;
    let cone_r = l * 0.07;
    let rod_len = l - cone_h; // rod runs up to the arrowhead base
    let axes = [
        (Vec3::X, [0.90, 0.27, 0.27]),
        (Vec3::Y, [0.30, 0.80, 0.33]),
        (Vec3::Z, [0.36, 0.48, 0.96]),
    ];
    for (i, (dir, color)) in axes.into_iter().enumerate() {
        let color = hl(color, hover == i as i32);
        push_rod(&mut v, center, dir, rod_len, rod_hw, color);
        push_cone(&mut v, center + dir * l, dir, cone_h, cone_r, color);
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

/// Rotate gizmo at `center` with ring radius `l`: one ring per axis (X/Y/Z) in
/// the plane perpendicular to it. `hover` brightens ring 0/1/2 (X/Y/Z), -1 none.
fn gizmo_rotate_geometry(center: Vec3, l: f32, hover: i32) -> Vec<GizmoVertex> {
    let mut v = Vec::new();
    let rings = [
        (Vec3::X, [0.90, 0.27, 0.27]),
        (Vec3::Y, [0.30, 0.80, 0.33]),
        (Vec3::Z, [0.36, 0.48, 0.96]),
    ];
    for (i, (axis, color)) in rings.into_iter().enumerate() {
        push_torus(&mut v, center, axis, l, l * 0.012, hl(color, hover == i as i32));
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

/// Scale gizmo at `center`, axis length `l`, oriented by `basis` (the object's
/// axes for a single selection, world for multi): a rod tipped with a cube per
/// axis (X/Y/Z), a filled square per plane (XY/YZ/XZ), and a center cube for
/// uniform scale. `hover` brightens one handle (0/1/2 axis, 3/4/5 plane, 6 center).
/// `guide` (an axis index) draws a long thin guide line through that axis.
fn gizmo_scale_geometry(
    center: Vec3,
    l: f32,
    basis: [Vec3; 3],
    hover: i32,
    guide: Option<usize>,
) -> Vec<GizmoVertex> {
    let mut v = Vec::new();
    let [ex, ey, ez] = basis;
    let rod_len = l; // rod runs all the way into the cube at the tip
    let rod_hw = l * 0.012; // matches the rotate ring tube
    let cube_h = l * 0.06;
    let axes = [(ex, [0.90, 0.27, 0.27]), (ey, [0.30, 0.80, 0.33]), (ez, [0.36, 0.48, 0.96])];
    // Guide line: a long thin rod through the active axis, drawn first (behind).
    if let Some(gi) = guide {
        let dir = basis[gi];
        let big = l * 40.0;
        push_rod(&mut v, center - dir * big, dir, big * 2.0, l * 0.004, axes[gi].1);
    }
    for (i, (dir, color)) in axes.into_iter().enumerate() {
        let color = hl(color, hover == i as i32);
        push_rod(&mut v, center, dir, rod_len, rod_hw, color);
        push_cube(&mut v, center + dir * l, cube_h, basis, color);
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
    // Uniform 3-axis scale handle: a cube at the center.
    push_cube(&mut v, center, l * 0.12, basis, hl([0.85, 0.85, 0.88], hover == 6));
    v
}

/// Gizmo center + handle length for the active plate's selection: the world AABB
/// center, and the bounding-*sphere* radius (max distance from that center to any
/// world corner). A handle at that radius encloses the part for any shape — every
/// point is within it by definition — and it's invariant to orientation (rotating
/// a part keeps its corners equidistant from the center), so the gizmo holds its
/// size through a turn where a bounding-box extent would grow/shrink. Min 3mm;
/// shared by the renderer and the hit-test command. `None` if nothing's selected.
fn selection_gizmo(p: &Project) -> Option<(Vec3, f32)> {
    let plate = p.active_plate();
    let mut corners: Vec<Vec3> = Vec::new();
    for (id, obj) in plate.scene.objects.iter() {
        if !obj.visible || !plate.scene.selection.contains(id) {
            continue;
        }
        let Some(m) = p.meshes.get(&obj.mesh) else {
            continue;
        };
        let model = obj.transform.to_mat4();
        let bb = m.bounding_box;
        for &sx in &[false, true] {
            for &sy in &[false, true] {
                for &sz in &[false, true] {
                    let c = Vec3::new(
                        (if sx { bb.max[0] } else { bb.min[0] }) as f32,
                        (if sy { bb.max[1] } else { bb.min[1] }) as f32,
                        (if sz { bb.max[2] } else { bb.min[2] }) as f32,
                    );
                    corners.push(model.transform_point3(c));
                }
            }
        }
    }
    if corners.is_empty() {
        return None;
    }
    let mut mn = Vec3::splat(f32::MAX);
    let mut mx = Vec3::splat(f32::MIN);
    for c in &corners {
        mn = mn.min(*c);
        mx = mx.max(*c);
    }
    let center = (mn + mx) * 0.5;
    let radius = corners.iter().map(|c| (*c - center).length()).fold(0.0, f32::max);
    Some((center, radius.max(3.0)))
}

/// World-space AABB enclosing the active plate's current selection, with `pre`
/// applied to `drag_ids` (so brackets/gizmo follow a preview).
fn selection_world_aabb(p: &Project, drag_ids: &[u64], pre: Mat4) -> Option<(Vec3, Vec3)> {
    let plate = p.active_plate();
    let mut mn = Vec3::splat(f32::MAX);
    let mut mx = Vec3::splat(f32::MIN);
    let mut any = false;
    for (id, obj) in plate.scene.objects.iter() {
        if !obj.visible || !plate.scene.selection.contains(id) {
            continue;
        }
        let Some(m) = p.meshes.get(&obj.mesh) else {
            continue;
        };
        let mut model = obj.transform.to_mat4();
        if drag_ids.contains(&id.0) {
            model = pre * model;
        }
        let bb = m.bounding_box;
        for &sx in &[false, true] {
            for &sy in &[false, true] {
                for &sz in &[false, true] {
                    let c = Vec3::new(
                        (if sx { bb.max[0] } else { bb.min[0] }) as f32,
                        (if sy { bb.max[1] } else { bb.min[1] }) as f32,
                        (if sz { bb.max[2] } else { bb.min[2] }) as f32,
                    );
                    let w = model.transform_point3(c);
                    mn = mn.min(w);
                    mx = mx.max(w);
                    any = true;
                }
            }
        }
    }
    any.then_some((mn, mx))
}

pub struct ViewportRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    bgl: wgpu::BindGroupLayout,
    mesh_pipe: wgpu::RenderPipeline,
    line_pipe: wgpu::RenderPipeline,
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
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::default());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        }))
        .expect("wgpu: no adapter");
        let info = adapter.get_info();
        tracing::info!("viewport wgpu adapter: {} | {:?}", info.name, info.backend);
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default(), None))
                .expect("wgpu: request_device");

        let slot = device
            .limits()
            .min_uniform_buffer_offset_alignment
            .max(64);
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
            bind_group_layouts: &[&bgl],
            push_constant_ranges: &[],
        });
        let vbl = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3],
        };
        let make_pipe = |topology, cull| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: None,
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: "vs",
                    compilation_options: Default::default(),
                    buffers: std::slice::from_ref(&vbl),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: "fs",
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
                    depth_write_enabled: true,
                    depth_compare: wgpu::CompareFunction::Less,
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState {
                    count: SAMPLES,
                    ..Default::default()
                },
                multiview: None,
            })
        };
        // No back-face culling: imported meshes (STL etc.) have no winding
        // guarantee, and mixed winding would drop valid front faces (holes). The
        // depth test still resolves the nearest face correctly.
        let mesh_pipe = make_pipe(wgpu::PrimitiveTopology::TriangleList, None);
        let line_pipe = make_pipe(wgpu::PrimitiveTopology::LineList, None);

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
                entry_point: "vs",
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<GizmoVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x3],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &gizmo_shader,
                entry_point: "fs",
                compilation_options: Default::default(),
                targets: &[Some(COLOR_FMT.into())],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: SAMPLES,
                ..Default::default()
            },
            multiview: None,
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
            gizmo_pipe,
            ubuf,
            bind,
            slot,
            slots_cap,
            meshes: HashMap::new(),
            vb_grid,
            vb_axes,
            size: (0, 0),
            color,
            msaa_view,
            depth_view,
            readback,
            padded_bpr,
        }
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

    /// Render the live scene and read it back as tight RGBA8, top row first.
    pub fn frame(&mut self, req: &FrameRequest, project: &Arc<Mutex<Project>>) -> Vec<u8> {
        let (w, h) = (req.width.max(1), req.height.max(1));
        self.resize(w, h);

        // --- gather under the project lock (cheap): bed + per-object models ---
        let (bmin, bmax, draws, boxes, gizmo) = {
            let p = project.lock().unwrap();
            let plate = p.active_plate();
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
            let mut draws: Vec<(MeshId, usize, Mat4, [f32; 3])> = Vec::new();
            // Local drag preview: world pre-multiply applied to dragged objects.
            let drag_pre = Mat4::from_cols_array(&req.drag_pre);
            let dragging = !req.drag_ids.is_empty();
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
                if dragging && req.drag_ids.contains(&id.0) {
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
                        // mix toward selection blue (matches the Three.js paint tint)
                        let b = SELECTED_RGB;
                        [
                            base[0] * 0.45 + b[0] * 0.55,
                            base[1] * 0.45 + b[1] * 0.55,
                            base[2] * 0.45 + b[2] * 0.55,
                        ]
                    } else {
                        base
                    };
                    draws.push((obj.mesh, gi, model, color));
                }
            }
            // One outer AABB enclosing the whole selection (world space) → a single
            // set of corner brackets, not one box per group member. Brackets hug
            // the live (drag-previewed) bounds. They're the affordance for the
            // no-tool XY-plane move, so they're hidden once a gizmo is active.
            let boxes = (req.gizmo == GizmoMode::None)
                .then(|| selection_world_aabb(&p, &req.drag_ids, drag_pre))
                .flatten()
                .map(|(mn, mx)| vec![(Mat4::IDENTITY, mn.to_array(), mx.to_array())])
                .unwrap_or_default();
            // The gizmo is sized + placed from the *resting* selection (no drag
            // preview) so it holds a fixed size through a drag.
            let gizmo = selection_gizmo(&p);
            (bmin, bmax, draws, boxes, gizmo)
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
        let total_slots = axis_slot + 3;

        // Scale gizmo axes follow the object's orientation (world for multi).
        let basis_q = Quat::from_xyzw(
            req.gizmo_basis[0],
            req.gizmo_basis[1],
            req.gizmo_basis[2],
            req.gizmo_basis[3],
        )
        .normalize();
        let basis = [basis_q * Vec3::X, basis_q * Vec3::Y, basis_q * Vec3::Z];
        // Eye position (matches view_proj), for the scale gizmo's screen-constant size.
        let (ce, se) = (req.el.cos(), req.el.sin());
        let (ca, sa) = (req.az.cos(), req.az.sin());
        let eye = Vec3::from(req.center) + req.dist * Vec3::new(ce * ca, ce * sa, se);
        // Active axis guide line, shown while dragging an axis handle.
        let guide = (req.gizmo_dragging && (0..=2).contains(&req.gizmo_hover))
            .then_some(req.gizmo_hover as usize);
        // Gizmo at the selection center (draws with slot 0's vp; own colors). Move
        // and Scale are constant on-screen size (length tracks eye distance); Move
        // also follows the dragged object (drag_pre is a translation). Rotate is
        // sized to the object (ring radius `r`) and stays put.
        let screen_l = |c: Vec3| GIZMO_SCREEN_K * (eye - c).length();
        let gizmo_verts = gizmo
            .map(|(c, r)| match req.gizmo {
                GizmoMode::Move => {
                    let c = Mat4::from_cols_array(&req.drag_pre).transform_point3(c);
                    gizmo_geometry(c, screen_l(c), req.gizmo_hover)
                }
                GizmoMode::Rotate => gizmo_rotate_geometry(c, r, req.gizmo_hover),
                GizmoMode::Scale => gizmo_scale_geometry(c, screen_l(c), basis, req.gizmo_hover, guide),
                GizmoMode::None => Vec::new(),
            })
            .filter(|v| !v.is_empty());
        let gizmo_vb = gizmo_verts.as_ref().map(|verts| {
            self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("viewport.gizmo"),
                contents: bytemuck::cast_slice(verts),
                usage: wgpu::BufferUsages::VERTEX,
            })
        });

        // Pack uniforms per slot: [mat4 mvp][vec4 color]. Slot 0 = grid (vp + grid
        // line color), slots 1.. = each object's vp*model + base color, final slot
        // (when there's a selection) = brackets (vp + bracket color).
        self.ensure_mvp_capacity(total_slots as u32);
        let mut bytes = vec![0u8; self.slot as usize * total_slots];
        bytes[0..64].copy_from_slice(bytemuck::cast_slice(&vp.to_cols_array()));
        bytes[64..80].copy_from_slice(bytemuck::cast_slice(&GRID_LINE));
        for (i, (_, _, model, color)) in draws.iter().enumerate() {
            let off = (i + 1) * self.slot as usize;
            bytes[off..off + 64]
                .copy_from_slice(bytemuck::cast_slice(&(vp * *model).to_cols_array()));
            let rgba = [color[0], color[1], color[2], 1.0f32];
            bytes[off + 64..off + 80].copy_from_slice(bytemuck::cast_slice(&rgba));
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
        self.queue.write_buffer(&self.ubuf, 0, &bytes);

        let color_view = self.color.create_view(&Default::default());
        let mut enc = self.device.create_command_encoder(&Default::default());
        {
            let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.msaa_view,
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
            });
            // grid
            rp.set_pipeline(&self.line_pipe);
            rp.set_bind_group(0, &self.bind, &[0]);
            rp.set_vertex_buffer(0, self.vb_grid.slice(..));
            rp.draw(0..self.n_grid, 0..1);
            // origin axis markers: each 2-vert segment with its own color slot.
            rp.set_vertex_buffer(0, self.vb_axes.slice(..));
            for k in 0..3u32 {
                rp.set_bind_group(0, &self.bind, &[(axis_slot as u32 + k) * self.slot]);
                rp.draw(k * 2..k * 2 + 2, 0..1);
            }
            // meshes
            rp.set_pipeline(&self.mesh_pipe);
            for (i, (mesh_id, gi, _, _)) in draws.iter().enumerate() {
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
            });
            rp.set_pipeline(&self.gizmo_pipe);
            rp.set_bind_group(0, &self.bind, &[0]);
            rp.set_vertex_buffer(0, vb.slice(..));
            rp.draw(0..verts.len() as u32, 0..1);
        }
        enc.copy_texture_to_buffer(
            wgpu::ImageCopyTexture {
                texture: &self.color,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::ImageCopyBuffer {
                buffer: &self.readback,
                layout: wgpu::ImageDataLayout {
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

        let slice = self.readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        self.device.poll(wgpu::Maintain::Wait);
        let mapped = slice.get_mapped_range();
        let row = (w * 4) as usize;
        let mut out = vec![0u8; row * h as usize];
        for y in 0..h as usize {
            let src = y * self.padded_bpr as usize;
            out[y * row..(y + 1) * row].copy_from_slice(&mapped[src..src + row]);
        }
        drop(mapped);
        self.readback.unmap();
        out
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

fn make_targets(
    device: &wgpu::Device,
    w: u32,
    h: u32,
) -> (wgpu::Texture, wgpu::TextureView, wgpu::TextureView, wgpu::Buffer, u32) {
    let ext = wgpu::Extent3d {
        width: w,
        height: h,
        depth_or_array_layers: 1,
    };
    // Single-sample resolve target — what gets read back.
    let color = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("viewport.color"),
        size: ext,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: COLOR_FMT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    // Multisampled color the scene actually renders into.
    let msaa = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("viewport.msaa"),
        size: ext,
        mip_level_count: 1,
        sample_count: SAMPLES,
        dimension: wgpu::TextureDimension::D2,
        format: COLOR_FMT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let depth = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("viewport.depth"),
        size: ext,
        mip_level_count: 1,
        sample_count: SAMPLES,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let padded_bpr = (w * 4).div_ceil(256) * 256;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("viewport.readback"),
        size: (padded_bpr * h) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    (
        color,
        msaa.create_view(&Default::default()),
        depth.create_view(&Default::default()),
        readback,
        padded_bpr,
    )
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

/// Drop the cached GPU meshes. `MeshId`s restart at 1 in a freshly loaded
/// project, so without this the renderer would serve the previous project's
/// geometry under the reused ids. Called by the frontend on `project:loaded`.
#[tauri::command]
pub fn viewport_reset(state: tauri::State<'_, ViewportState>) {
    if let Some(r) = state.0.lock().unwrap().as_mut() {
        r.meshes.clear();
    }
}

/// Initial camera framing for the active plate's bed — the frontend sets the
/// orbit center + distance from this on load / bed change.
#[derive(serde::Serialize)]
pub struct SceneInfo {
    pub center: [f32; 3],
    pub distance: f32,
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
    let span = (max[0] - min[0]).max(max[1] - min[1]).max(1.0);
    SceneInfo {
        center: [
            ((min[0] + max[0]) / 2.0) as f32,
            ((min[1] + max[1]) / 2.0) as f32,
            0.0,
        ],
        distance: (span * 1.7) as f32,
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

/// View-projection for the orbit camera (z up). Shared by render and pick so the
/// click ray matches exactly what's drawn.
fn view_proj(w: f32, h: f32, az: f32, el: f32, dist: f32, center: Vec3) -> Mat4 {
    let (ce, se) = (el.cos(), el.sin());
    let (ca, sa) = (az.cos(), az.sin());
    let eye = center + dist * Vec3::new(ce * ca, ce * sa, se);
    let far = (dist * 10.0).max(1000.0);
    let proj = Mat4::perspective_rh(45f32.to_radians(), w / h, 0.1, far);
    proj * Mat4::look_at_rh(eye, center, Vec3::Z)
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
    let p = project.lock().unwrap();
    let plate = p.active_plate();
    let (w, h) = (req.width.max(1) as f32, req.height.max(1) as f32);
    let vp = view_proj(w, h, req.az, req.el, req.dist, Vec3::from(req.center));
    let inv = vp.inverse();
    let ndc = Vec3::new(2.0 * req.x / w - 1.0, 1.0 - 2.0 * req.y / h, 0.0);
    let ro = inv.project_point3(ndc);
    let far = inv.project_point3(Vec3::new(ndc.x, ndc.y, 1.0));
    let rd = (far - ro).normalize();

    let mut best: Option<(f32, u64)> = None;
    for (id, obj) in plate.scene.objects.iter() {
        if !obj.visible {
            continue;
        }
        let Some(m) = p.meshes.get(&obj.mesh) else {
            continue;
        };
        let model = obj.transform.to_mat4();
        let vert = |vi: u32| {
            let i = vi as usize * 3;
            model.transform_point3(Vec3::new(m.vertices[i], m.vertices[i + 1], m.vertices[i + 2]))
        };
        for t3 in m.indices.chunks_exact(3) {
            if let Some(t) = ray_tri(ro, rd, vert(t3[0]), vert(t3[1]), vert(t3[2])) {
                if best.map_or(true, |(bt, _)| t < bt) {
                    best = Some((t, id.0));
                }
            }
        }
    }
    best.map(|(_, id)| id)
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
    let plate = p.active_plate();
    let (w, h) = (req.width.max(1) as f32, req.height.max(1) as f32);
    let vp = view_proj(w, h, req.az, req.el, req.dist, Vec3::from(req.center));
    let inv = vp.inverse();
    let ndc = Vec3::new(2.0 * req.x / w - 1.0, 1.0 - 2.0 * req.y / h, 0.0);
    let ro = inv.project_point3(ndc);
    let far = inv.project_point3(Vec3::new(ndc.x, ndc.y, 1.0));
    let rd = (far - ro).normalize();

    let mut best: Option<(f32, u64, Vec3, Vec3, Vec3)> = None;
    for (id, obj) in plate.scene.objects.iter() {
        if !obj.visible {
            continue;
        }
        let Some(m) = p.meshes.get(&obj.mesh) else {
            continue;
        };
        let model = obj.transform.to_mat4();
        let vert = |vi: u32| {
            let i = vi as usize * 3;
            model.transform_point3(Vec3::new(m.vertices[i], m.vertices[i + 1], m.vertices[i + 2]))
        };
        for t3 in m.indices.chunks_exact(3) {
            let (a, b, c) = (vert(t3[0]), vert(t3[1]), vert(t3[2]));
            if let Some(t) = ray_tri(ro, rd, a, b, c) {
                if best.map_or(true, |(bt, ..)| t < bt) {
                    best = Some((t, id.0, a, b, c));
                }
            }
        }
    }
    best.map(|(t, id, a, b, c)| {
        // Geometric normal of the world-space triangle (winding gives outward).
        let normal = (b - a).cross(c - a).normalize_or_zero();
        FacePick {
            id,
            normal: normal.to_array(),
            point: (ro + rd * t).to_array(),
        }
    })
}

/// Whether the Strategy-A wgpu viewport is enabled (`N3O_WGPU=1`).
pub fn enabled() -> bool {
    std::env::var_os("N3O_WGPU").is_some()
}
