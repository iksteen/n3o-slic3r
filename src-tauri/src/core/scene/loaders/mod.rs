//! Mesh-file loaders.
//!
//! STL + OBJ here; .3mf (project shape) lives in a sibling
//! submodule. Each loader produces a [`super::state::Mesh`] with
//! a computed bounding box; format dispatch is by file
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
            Self::Parse { path, message } => {
                write!(f, "{}: parse error: {message}", path.display())
            }
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
/// `scene_load_mesh_from_path`.
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
