//! OBJ loader.
//!
//! Wavefront OBJ via the `tobj` crate. Multi-group OBJ files collapse
//! to a single `Mesh` — per-volume extruder data lives in `.3mf`'s
//! `model_settings.config`, not OBJ groups. `.mtl` material
//! libraries are ignored for MVP; object library can
//! re-introduce them when the renderer cares about textures.

use super::{compute_bounding_box, LoadError};
use crate::core::scene::state::{MeshProvenance, NewMesh};
use std::path::Path;

pub fn load(path: &Path) -> Result<NewMesh, LoadError> {
    let load_opts = tobj::LoadOptions {
        triangulate: true,
        single_index: true,
        ignore_lines: true,
        ignore_points: true,
    };
    let (models, _materials) = tobj::load_obj(path, &load_opts).map_err(|e| match e {
        tobj::LoadError::OpenFileFailed | tobj::LoadError::ReadError => LoadError::Io {
            path: path.into(),
            source: std::io::Error::other(e.to_string()),
        },
        other => LoadError::Parse {
            path: path.into(),
            message: other.to_string(),
        },
    })?;

    if models.is_empty() {
        return Err(LoadError::Empty { path: path.into() });
    }

    // Merge every model's mesh into one Mesh — OBJ groups collapse.
    // Vertex indices need rebasing per model so the merged index
    // buffer keeps referring to the right vertices.
    let mut vertices: Vec<f32> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    for m in &models {
        let vert_base = (vertices.len() / 3) as u32;
        vertices.extend_from_slice(&m.mesh.positions);
        for idx in &m.mesh.indices {
            indices.push(idx + vert_base);
        }
    }

    if vertices.is_empty() || indices.is_empty() {
        return Err(LoadError::Empty { path: path.into() });
    }

    let bounding_box = compute_bounding_box(&vertices);

    Ok(NewMesh {
        vertices,
        indices,
        paint_colors: None,
        support_paint: None,
        bounding_box,
        provenance: MeshProvenance::File(path.display().to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    /// Single triangle in OBJ form, CCW in XY-plane.
    const SIMPLE_OBJ: &str = "\
v 0 0 0
v 1 0 0
v 0 1 0
f 1 2 3
";

    fn write_obj(content: &str) -> NamedTempFile {
        // tobj uses the file extension to locate the .mtl sibling;
        // give the temp file an .obj suffix so the loader sees it.
        let mut f = tempfile::Builder::new()
            .suffix(".obj")
            .tempfile()
            .expect("tempfile");
        f.write_all(content.as_bytes()).unwrap();
        f.flush().unwrap();
        f
    }

    #[test]
    fn obj_loads_single_triangle() {
        let f = write_obj(SIMPLE_OBJ);
        let mesh = load(f.path()).expect("load");
        assert_eq!(mesh.indices.len(), 3, "1 triangle");
        assert_eq!(mesh.vertices.len(), 9, "3 vertices × 3 coords");
        assert_eq!(mesh.bounding_box.min, [0.0, 0.0, 0.0]);
        assert_eq!(mesh.bounding_box.max, [1.0, 1.0, 0.0]);
    }

    #[test]
    fn obj_empty_input_errors() {
        let f = write_obj("");
        let err = load(f.path()).expect_err("empty");
        assert!(
            matches!(err, LoadError::Empty { .. } | LoadError::Parse { .. }),
            "got {err:?}"
        );
    }
}
