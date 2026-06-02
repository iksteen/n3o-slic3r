//! STL loader.
//!
//! Reads both ASCII (`solid …`) and binary STL via the `stl_io` crate
//! and produces a [`super::super::state::Mesh`] with computed
//! per-vertex normals + bounding box. The 80-byte binary STL header
//! starts with the characters `solid` *some* of the time, so we
//! always go through `stl_io::create_stl_reader`, which sniffs both
//! variants internally.

use super::{compute_bounding_box, compute_vertex_normals, LoadError};
use crate::core::scene::state::{MeshProvenance, NewMesh};
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

pub fn load(path: &Path) -> Result<NewMesh, LoadError> {
    let file = File::open(path).map_err(|e| LoadError::Io {
        path: path.into(),
        source: e,
    })?;
    let mut reader = BufReader::new(file);
    let stl = stl_io::read_stl(&mut reader).map_err(|e| LoadError::Parse {
        path: path.into(),
        message: e.to_string(),
    })?;

    // stl_io returns IndexedMesh: vertices (deduplicated f32 points)
    // + faces (triangle indices + per-face normal). Convert to our
    // flat-arrays shape.
    let mut vertices = Vec::with_capacity(stl.vertices.len() * 3);
    for v in &stl.vertices {
        vertices.push(v[0]);
        vertices.push(v[1]);
        vertices.push(v[2]);
    }
    let mut indices = Vec::with_capacity(stl.faces.len() * 3);
    for face in &stl.faces {
        indices.push(face.vertices[0] as u32);
        indices.push(face.vertices[1] as u32);
        indices.push(face.vertices[2] as u32);
    }

    if vertices.is_empty() || indices.is_empty() {
        return Err(LoadError::Empty { path: path.into() });
    }

    // STL gives us per-face normals; we want per-vertex for smooth
    // shading. Recompute area-weighted per-vertex normals.
    let normals = compute_vertex_normals(&vertices, &indices);
    let bounding_box = compute_bounding_box(&vertices);

    Ok(NewMesh {
        vertices,
        normals,
        indices,
        paint_colors: None,
        bounding_box,
        provenance: MeshProvenance::File(path.display().to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    /// ASCII STL of one triangle in the XY-plane. Per-vertex normals
    /// after compute_vertex_normals should be (0,0,1).
    const ASCII_TRI: &str = "\
solid triangle
  facet normal 0 0 1
    outer loop
      vertex 0 0 0
      vertex 1 0 0
      vertex 0 1 0
    endloop
  endfacet
endsolid triangle
";

    fn write_to_tempfile(content: &[u8]) -> NamedTempFile {
        let mut f = NamedTempFile::new().expect("tempfile");
        f.write_all(content).expect("write");
        f.flush().expect("flush");
        f
    }

    #[test]
    fn ascii_stl_loads_one_triangle() {
        let f = write_to_tempfile(ASCII_TRI.as_bytes());
        let mesh = load(f.path()).expect("load");
        assert_eq!(mesh.vertices.len(), 9, "3 vertices × 3 coords");
        assert_eq!(mesh.indices.len(), 3, "1 triangle × 3 indices");
        // All per-vertex normals point +Z for a CCW XY triangle.
        for chunk in mesh.normals.chunks_exact(3) {
            assert!((chunk[2] - 1.0).abs() < 1e-4, "got {chunk:?}");
        }
        assert_eq!(mesh.bounding_box.min, [0.0, 0.0, 0.0]);
        assert_eq!(mesh.bounding_box.max, [1.0, 1.0, 0.0]);
        assert!(matches!(mesh.provenance, MeshProvenance::File(_)));
    }

    // Note: NewMesh has no `id` field; loaders return NewMesh and
    // the SceneState allocates the id at register time.

    #[test]
    fn binary_stl_loads_one_triangle() {
        // 80-byte header + u32 face count + 50 bytes per triangle
        // (12 normal + 36 vertex + 2 attribute).
        let mut bytes = Vec::with_capacity(84 + 50);
        bytes.extend_from_slice(&[0u8; 80]); // header (non-"solid" so the sniffer takes the binary branch)
        bytes.extend_from_slice(&1u32.to_le_bytes()); // 1 face
                                                      // face normal (ignored — we recompute)
        for f in [0.0f32, 0.0, 1.0] {
            bytes.extend_from_slice(&f.to_le_bytes());
        }
        // 3 vertices
        for v in [
            [0.0_f32, 0.0, 0.0],
            [1.0_f32, 0.0, 0.0],
            [0.0_f32, 1.0, 0.0],
        ] {
            for component in v {
                bytes.extend_from_slice(&component.to_le_bytes());
            }
        }
        bytes.extend_from_slice(&0u16.to_le_bytes()); // attribute

        let f = write_to_tempfile(&bytes);
        let mesh = load(f.path()).expect("binary load");
        assert_eq!(mesh.indices.len(), 3);
        // stl_io dedupes vertices — for this triangle that's 3
        // distinct (one per corner).
        assert_eq!(mesh.vertices.len(), 9);
        for chunk in mesh.normals.chunks_exact(3) {
            assert!((chunk[2] - 1.0).abs() < 1e-4);
        }
    }

    #[test]
    fn empty_input_returns_empty_error() {
        let f = write_to_tempfile(b"");
        let err = load(f.path()).expect_err("empty should error");
        // Either Parse (stl_io rejects it) or Empty (we caught it).
        assert!(
            matches!(err, LoadError::Parse { .. } | LoadError::Empty { .. }),
            "got {err:?}"
        );
    }
}
