//! Mesh-file loaders.
//!
//! STL + OBJ for PR-2-3; .3mf (project shape) lands in PR-2-4 as a
//! sibling submodule. Each loader produces a [`super::state::Mesh`]
//! with computed normals + bounding box; format dispatch is by file
//! extension with a magic-byte fallback for STL's ASCII/binary
//! variants.

pub mod obj;
pub mod stl;

use super::state::NewMesh;
use crate::core::printer::profile::BoundingBox;
use std::fmt;
use std::path::Path;

/// Errors returned by the loaders.
///
/// **Eventual home: `core/geometry/`** — `core/threemf` already
/// imports this type via `crate::core::scene::loaders::LoadError`
/// to share error reporting with the STL/OBJ loaders here. Pull
/// it (and the `compute_*` helpers below) into a sibling
/// `core/geometry/` when a third loader / writer arrives so the
/// scene module isn't a half-utility-bag. See `core/scene/mod.rs`
/// for the architectural review note.
#[derive(Debug)]
pub enum LoadError {
    Io {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    Parse {
        path: std::path::PathBuf,
        message: String,
    },
    UnsupportedExtension {
        path: std::path::PathBuf,
    },
    Empty {
        path: std::path::PathBuf,
    },
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "{}: {source}", path.display()),
            Self::Parse { path, message } => write!(f, "{}: parse error: {message}", path.display()),
            Self::UnsupportedExtension { path } => write!(
                f,
                "{}: unsupported extension (expected .stl or .obj)",
                path.display()
            ),
            Self::Empty { path } => write!(f, "{}: no geometry in file", path.display()),
        }
    }
}

impl std::error::Error for LoadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Dispatch a load by file extension. Surface used by
/// `scene_load_mesh_from_path` (PR-2-2).
pub fn load_mesh_from_path(path: &Path) -> Result<NewMesh, LoadError> {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase());
    match ext.as_deref() {
        Some("stl") => stl::load(path),
        Some("obj") => obj::load(path),
        _ => Err(LoadError::UnsupportedExtension { path: path.into() }),
    }
}

/// Compute per-vertex normals by summing each triangle's face normal
/// into its three vertices, then normalizing. Synthesizes smooth-
/// shading normals for loaders whose source format only carries
/// per-face normals (STL) or none at all (OBJ without `vn` lines).
///
/// Vertices laid out flat [x, y, z, x, y, z, ...]; indices triple
/// (3 per triangle, ccw winding).
pub(crate) fn compute_vertex_normals(vertices: &[f32], indices: &[u32]) -> Vec<f32> {
    let vert_count = vertices.len() / 3;
    let mut normals = vec![0.0_f32; vert_count * 3];

    for tri in indices.chunks_exact(3) {
        let (a, b, c) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
        let va = glam::Vec3::new(vertices[a * 3], vertices[a * 3 + 1], vertices[a * 3 + 2]);
        let vb = glam::Vec3::new(vertices[b * 3], vertices[b * 3 + 1], vertices[b * 3 + 2]);
        let vc = glam::Vec3::new(vertices[c * 3], vertices[c * 3 + 1], vertices[c * 3 + 2]);
        let face_n = (vb - va).cross(vc - va);
        if face_n.length_squared() < f32::EPSILON {
            continue; // degenerate
        }
        // Area-weighted accumulation: don't normalize the face
        // normal before summing — larger faces contribute more,
        // which produces better smooth-shaded results on irregular
        // meshes.
        for &i in &[a, b, c] {
            normals[i * 3] += face_n.x;
            normals[i * 3 + 1] += face_n.y;
            normals[i * 3 + 2] += face_n.z;
        }
    }

    // Normalize.
    for i in 0..vert_count {
        let n = glam::Vec3::new(normals[i * 3], normals[i * 3 + 1], normals[i * 3 + 2]);
        let len_sq = n.length_squared();
        if len_sq > f32::EPSILON {
            let n = n / len_sq.sqrt();
            normals[i * 3] = n.x;
            normals[i * 3 + 1] = n.y;
            normals[i * 3 + 2] = n.z;
        }
    }

    normals
}

/// Bounding box over a packed vertex array.
pub(crate) fn compute_bounding_box(vertices: &[f32]) -> BoundingBox {
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    for chunk in vertices.chunks_exact(3) {
        for axis in 0..3 {
            let v = chunk[axis] as f64;
            if v < min[axis] {
                min[axis] = v;
            }
            if v > max[axis] {
                max[axis] = v;
            }
        }
    }
    if !min[0].is_finite() {
        // Empty vertex list → identity box. Loaders catch this and
        // return Empty {} before reaching here.
        BoundingBox::default()
    } else {
        BoundingBox { min, max }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vertex_normals_on_unit_quad_point_up_z() {
        // Two triangles forming a flat XY-plane quad. Vertex normals
        // should all point along +Z (since the face normal of a
        // CCW-wound XY triangle is +Z).
        let vertices = vec![
            0.0, 0.0, 0.0, //
            1.0, 0.0, 0.0, //
            1.0, 1.0, 0.0, //
            0.0, 1.0, 0.0, //
        ];
        let indices = vec![0, 1, 2, 0, 2, 3];
        let normals = compute_vertex_normals(&vertices, &indices);
        for chunk in normals.chunks_exact(3) {
            assert!((chunk[2] - 1.0).abs() < 1e-5, "got {chunk:?}");
        }
    }

    #[test]
    fn bounding_box_basic() {
        let vertices = vec![
            0.0, 0.0, 0.0, //
            5.0, 0.0, 0.0, //
            0.0, 3.0, -2.0, //
        ];
        let bb = compute_bounding_box(&vertices);
        assert_eq!(bb.min, [0.0, 0.0, -2.0]);
        assert_eq!(bb.max, [5.0, 3.0, 0.0]);
    }
}
