//! Procedural primitives.
//!
//! Generates mesh data for the five primitive types the object
//! library exposes: cube, cylinder, sphere, cone, torus. Each
//! returns a [`NewMesh`] with a computed bounding box — the same
//! shape the file loaders in `loaders/{stl,obj,threemf}` produce,
//! so the registry layer doesn't care where the geometry came from.
//!
//! Dimensions are in millimeters — the same units the rest of the
//! scene uses.

use serde::{Deserialize, Serialize};

use super::state::{MeshProvenance, NewMesh};
use crate::core::printer::profile::BoundingBox;

/// One of the five primitive types the library offers.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum PrimitiveKind {
    Cube,
    Cylinder,
    Sphere,
    Cone,
    Torus,
}

/// Parameters for instantiating a primitive. Each field is meaningful
/// only for some kinds; the relevant ones for each kind are
/// documented inline. Serialization is a single object the frontend
/// can fill in from a parameter dialog — fields not relevant to the
/// chosen kind are ignored.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct PrimitiveParams {
    /// Cube: full extent along X.
    /// Torus: outer diameter.
    /// Other kinds: ignored.
    pub width: f32,
    /// Cube: full extent along Y. Other kinds: ignored.
    pub depth: f32,
    /// Cube / Cylinder / Cone: full extent along Z.
    /// Sphere: ignored.
    /// Torus: ignored.
    pub height: f32,
    /// Cylinder / Cone: radius of the cap.
    /// Sphere: radius.
    /// Torus: tube radius (the "minor" radius).
    pub radius: f32,
    /// Cylinder / Cone / Torus / Sphere: number of radial segments.
    /// For Sphere this is also the latitude segment count; longitude
    /// is taken as `radial_segments * 2` so we get round, not flat,
    /// shading at the poles. Default 32.
    pub radial_segments: u32,
}

impl PrimitiveParams {
    /// Sensible defaults for each kind. The frontend can present
    /// these as form placeholders.
    pub fn defaults_for(kind: PrimitiveKind) -> Self {
        match kind {
            PrimitiveKind::Cube => Self {
                width: 20.0,
                depth: 20.0,
                height: 20.0,
                radius: 0.0,
                radial_segments: 0,
            },
            PrimitiveKind::Cylinder => Self {
                width: 0.0,
                depth: 0.0,
                height: 20.0,
                radius: 10.0,
                radial_segments: 32,
            },
            PrimitiveKind::Sphere => Self {
                width: 0.0,
                depth: 0.0,
                height: 0.0,
                radius: 10.0,
                radial_segments: 32,
            },
            PrimitiveKind::Cone => Self {
                width: 0.0,
                depth: 0.0,
                height: 20.0,
                radius: 10.0,
                radial_segments: 32,
            },
            PrimitiveKind::Torus => Self {
                width: 30.0, // outer diameter
                depth: 0.0,
                height: 0.0,
                radius: 4.0, // tube radius
                radial_segments: 32,
            },
        }
    }
}

/// Generate the mesh for `kind` at the given `params`. The object's
/// origin sits at the geometric center for Sphere, Cube, and Torus
/// and at the base center for Cylinder + Cone (so the user's
/// "add to plate" places the object resting on Z=0).
pub fn generate(kind: PrimitiveKind, params: PrimitiveParams) -> NewMesh {
    match kind {
        PrimitiveKind::Cube => cube(params.width, params.depth, params.height),
        PrimitiveKind::Cylinder => {
            cylinder(params.radius, params.height, params.radial_segments.max(3))
        }
        PrimitiveKind::Sphere => sphere(params.radius, params.radial_segments.max(3)),
        PrimitiveKind::Cone => cone(params.radius, params.height, params.radial_segments.max(3)),
        PrimitiveKind::Torus => torus(
            params.width * 0.5,
            params.radius,
            params.radial_segments.max(3),
            params.radial_segments.max(3),
        ),
    }
}

/// Axis-aligned box with sharp edges. We split per-face — each face
/// owns 4 vertices — so the renderer doesn't average across edges.
fn cube(w: f32, d: f32, h: f32) -> NewMesh {
    let hx = w * 0.5;
    let hy = d * 0.5;
    let hz = h * 0.5;
    // 6 faces × 4 corner vertices = 24 vertices; 6 × 2 triangles = 36 indices.
    let mut vertices = Vec::with_capacity(72);
    let mut indices = Vec::with_capacity(36);

    // Face emit helper. Vertex winding is CCW seen from outside.
    let mut emit = |corners: [[f32; 3]; 4]| {
        let base = (vertices.len() / 3) as u32;
        for c in &corners {
            vertices.extend_from_slice(c);
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    };

    emit([[-hx, -hy, hz], [hx, -hy, hz], [hx, hy, hz], [-hx, hy, hz]]);
    emit([
        [-hx, hy, -hz],
        [hx, hy, -hz],
        [hx, -hy, -hz],
        [-hx, -hy, -hz],
    ]);
    emit([[hx, -hy, -hz], [hx, hy, -hz], [hx, hy, hz], [hx, -hy, hz]]);
    emit([
        [-hx, hy, -hz],
        [-hx, -hy, -hz],
        [-hx, -hy, hz],
        [-hx, hy, hz],
    ]);
    emit([[hx, hy, -hz], [-hx, hy, -hz], [-hx, hy, hz], [hx, hy, hz]]);
    emit([
        [-hx, -hy, -hz],
        [hx, -hy, -hz],
        [hx, -hy, hz],
        [-hx, -hy, hz],
    ]);

    let bounding_box = BoundingBox {
        min: [-hx as f64, -hy as f64, -hz as f64],
        max: [hx as f64, hy as f64, hz as f64],
    };
    NewMesh {
        vertices,
        indices,
        paint_colors: None,
        support_paint: None,
        bounding_box,
        provenance: MeshProvenance::Primitive("cube".into()),
    }
}

/// Cylinder centered on the Z axis, base at Z=0, top at Z=h.
fn cylinder(radius: f32, height: f32, segments: u32) -> NewMesh {
    let segs = segments as usize;
    // 2*segs vertices for side strip (top + bottom ring) with shared
    // wrap-around — we duplicate the seam vertex so UV mapping would
    // work later. Plus 2 caps (each with one center vertex + segs ring
    // vertices).
    let mut vertices: Vec<f32> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    let two_pi = std::f32::consts::PI * 2.0;

    // --- Side strip ----------------------------------------------------
    // Two rings (top + bottom).
    let side_base = (vertices.len() / 3) as u32;
    for i in 0..=segs {
        let theta = (i as f32 / segs as f32) * two_pi;
        let (sx, sy) = (theta.cos(), theta.sin());
        // Bottom
        vertices.extend_from_slice(&[sx * radius, sy * radius, 0.0]);
        // Top
        vertices.extend_from_slice(&[sx * radius, sy * radius, height]);
    }
    for i in 0..segs as u32 {
        let b0 = side_base + i * 2;
        let t0 = b0 + 1;
        let b1 = side_base + (i + 1) * 2;
        let t1 = b1 + 1;
        indices.extend_from_slice(&[b0, b1, t1, b0, t1, t0]);
    }

    // --- Top cap --------------------------------------------------------
    let top_center = (vertices.len() / 3) as u32;
    vertices.extend_from_slice(&[0.0, 0.0, height]);
    let top_ring_base = (vertices.len() / 3) as u32;
    for i in 0..segs {
        let theta = (i as f32 / segs as f32) * two_pi;
        vertices.extend_from_slice(&[theta.cos() * radius, theta.sin() * radius, height]);
    }
    for i in 0..segs as u32 {
        let next = (i + 1) % segs as u32;
        indices.extend_from_slice(&[top_center, top_ring_base + i, top_ring_base + next]);
    }

    // --- Bottom cap -----------------------------------------------------
    let bot_center = (vertices.len() / 3) as u32;
    vertices.extend_from_slice(&[0.0, 0.0, 0.0]);
    let bot_ring_base = (vertices.len() / 3) as u32;
    for i in 0..segs {
        let theta = (i as f32 / segs as f32) * two_pi;
        vertices.extend_from_slice(&[theta.cos() * radius, theta.sin() * radius, 0.0]);
    }
    for i in 0..segs as u32 {
        let next = (i + 1) % segs as u32;
        // Reversed winding so the cap faces down.
        indices.extend_from_slice(&[bot_center, bot_ring_base + next, bot_ring_base + i]);
    }

    let bounding_box = BoundingBox {
        min: [-radius as f64, -radius as f64, 0.0],
        max: [radius as f64, radius as f64, height as f64],
    };
    NewMesh {
        vertices,
        indices,
        paint_colors: None,
        support_paint: None,
        bounding_box,
        provenance: MeshProvenance::Primitive("cylinder".into()),
    }
}

/// UV sphere centered at origin. `segments` controls *both* longitude
/// and latitude — longitude = `2 * segments`, latitude = `segments`
/// (so a 32-seg sphere is 64 lon × 32 lat = ~2000 vertices).
fn sphere(radius: f32, segments: u32) -> NewMesh {
    let lat_count = segments.max(2) as usize;
    let lon_count = (segments.max(2) * 2) as usize;
    let mut vertices: Vec<f32> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    let pi = std::f32::consts::PI;
    let two_pi = pi * 2.0;
    for lat in 0..=lat_count {
        let v = lat as f32 / lat_count as f32;
        let phi = v * pi; // 0 (top) .. pi (bottom)
        let cos_phi = phi.cos();
        let sin_phi = phi.sin();
        for lon in 0..=lon_count {
            let u = lon as f32 / lon_count as f32;
            let theta = u * two_pi;
            let nx = sin_phi * theta.cos();
            let ny = sin_phi * theta.sin();
            let nz = cos_phi;
            vertices.extend_from_slice(&[nx * radius, ny * radius, nz * radius]);
        }
    }
    let stride = lon_count + 1;
    for lat in 0..lat_count {
        for lon in 0..lon_count {
            let a = (lat * stride + lon) as u32;
            let b = a + 1;
            let c = a + stride as u32;
            let d = c + 1;
            indices.extend_from_slice(&[a, c, d, a, d, b]);
        }
    }

    let bounding_box = BoundingBox {
        min: [-radius as f64; 3],
        max: [radius as f64; 3],
    };
    NewMesh {
        vertices,
        indices,
        paint_colors: None,
        support_paint: None,
        bounding_box,
        provenance: MeshProvenance::Primitive("sphere".into()),
    }
}

/// Cone centered on the Z axis, base at Z=0, apex at Z=h.
fn cone(radius: f32, height: f32, segments: u32) -> NewMesh {
    let segs = segments as usize;
    let mut vertices: Vec<f32> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    let two_pi = std::f32::consts::PI * 2.0;

    // Side strip: triangles from each base segment to the apex.
    let base_ring_start = (vertices.len() / 3) as u32;
    for i in 0..=segs {
        let theta = (i as f32 / segs as f32) * two_pi;
        let (cx, cy) = (theta.cos(), theta.sin());
        // Base vertex.
        vertices.extend_from_slice(&[cx * radius, cy * radius, 0.0]);
    }
    // Apex vertex per segment (split so the tip stays a distinct
    // vertex per face rather than a single shared apex).
    let apex_start = (vertices.len() / 3) as u32;
    for _ in 0..segs {
        vertices.extend_from_slice(&[0.0, 0.0, height]);
    }
    for i in 0..segs as u32 {
        indices.extend_from_slice(&[base_ring_start + i, base_ring_start + i + 1, apex_start + i]);
    }

    // Bottom cap (faces down).
    let cap_center = (vertices.len() / 3) as u32;
    vertices.extend_from_slice(&[0.0, 0.0, 0.0]);
    let cap_ring_start = (vertices.len() / 3) as u32;
    for i in 0..segs {
        let theta = (i as f32 / segs as f32) * two_pi;
        vertices.extend_from_slice(&[theta.cos() * radius, theta.sin() * radius, 0.0]);
    }
    for i in 0..segs as u32 {
        let next = (i + 1) % segs as u32;
        indices.extend_from_slice(&[cap_center, cap_ring_start + next, cap_ring_start + i]);
    }

    let bounding_box = BoundingBox {
        min: [-radius as f64, -radius as f64, 0.0],
        max: [radius as f64, radius as f64, height as f64],
    };
    NewMesh {
        vertices,
        indices,
        paint_colors: None,
        support_paint: None,
        bounding_box,
        provenance: MeshProvenance::Primitive("cone".into()),
    }
}

/// Torus lying flat in the XY plane, ring centered at origin.
/// `major_radius` is the distance from the torus center to the tube
/// center; `minor_radius` is the tube radius. `tube_segments`
/// subdivides the major loop; `ring_segments` subdivides the tube
/// cross-section.
fn torus(major_radius: f32, minor_radius: f32, tube_segments: u32, ring_segments: u32) -> NewMesh {
    let tube = tube_segments.max(3) as usize;
    let ring = ring_segments.max(3) as usize;
    let mut vertices: Vec<f32> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    let two_pi = std::f32::consts::PI * 2.0;
    for i in 0..=tube {
        let u = i as f32 / tube as f32 * two_pi;
        let (cu, su) = (u.cos(), u.sin());
        for j in 0..=ring {
            let v = j as f32 / ring as f32 * two_pi;
            let (cv, sv) = (v.cos(), v.sin());
            let x = (major_radius + minor_radius * cv) * cu;
            let y = (major_radius + minor_radius * cv) * su;
            let z = minor_radius * sv;
            vertices.extend_from_slice(&[x, y, z]);
        }
    }
    let stride = ring + 1;
    for i in 0..tube {
        for j in 0..ring {
            let a = (i * stride + j) as u32;
            let b = a + 1;
            let c = a + stride as u32;
            let d = c + 1;
            indices.extend_from_slice(&[a, c, d, a, d, b]);
        }
    }

    let outer = major_radius + minor_radius;
    let bounding_box = BoundingBox {
        min: [-outer as f64, -outer as f64, -minor_radius as f64],
        max: [outer as f64, outer as f64, minor_radius as f64],
    };
    NewMesh {
        vertices,
        indices,
        paint_colors: None,
        support_paint: None,
        bounding_box,
        provenance: MeshProvenance::Primitive("torus".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn closed_mesh_invariants(mesh: &NewMesh) {
        assert_eq!(mesh.vertices.len() % 3, 0, "vertices flat XYZ");
        assert_eq!(mesh.indices.len() % 3, 0, "triangle triples");
        for i in &mesh.indices {
            assert!(
                (*i as usize) * 3 < mesh.vertices.len(),
                "index out of range"
            );
        }
    }

    #[test]
    fn cube_has_24_vertices_36_indices_correct_bbox() {
        let m = cube(2.0, 4.0, 6.0);
        assert_eq!(m.vertices.len(), 72, "24 vertices × 3 coords");
        assert_eq!(m.indices.len(), 36, "6 faces × 2 tris × 3 corners");
        assert_eq!(m.bounding_box.min, [-1.0, -2.0, -3.0]);
        assert_eq!(m.bounding_box.max, [1.0, 2.0, 3.0]);
        closed_mesh_invariants(&m);
    }

    #[test]
    fn cylinder_has_correct_bbox_and_invariants() {
        let m = cylinder(5.0, 10.0, 16);
        // Base at z=0, top at z=10, radius 5.
        assert_eq!(m.bounding_box.max, [5.0, 5.0, 10.0]);
        assert_eq!(m.bounding_box.min, [-5.0, -5.0, 0.0]);
        closed_mesh_invariants(&m);
    }

    #[test]
    fn sphere_has_correct_bbox_and_invariants() {
        let m = sphere(7.5, 16);
        assert_eq!(m.bounding_box.min, [-7.5; 3]);
        assert_eq!(m.bounding_box.max, [7.5; 3]);
        closed_mesh_invariants(&m);
    }

    #[test]
    fn cone_has_correct_bbox_and_invariants() {
        let m = cone(3.0, 8.0, 16);
        assert_eq!(m.bounding_box.min, [-3.0, -3.0, 0.0]);
        assert_eq!(m.bounding_box.max, [3.0, 3.0, 8.0]);
        closed_mesh_invariants(&m);
    }

    #[test]
    fn torus_has_correct_bbox_and_invariants() {
        let m = torus(10.0, 2.0, 16, 16);
        let outer = 10.0 + 2.0;
        assert_eq!(m.bounding_box.min, [-outer, -outer, -2.0]);
        assert_eq!(m.bounding_box.max, [outer, outer, 2.0]);
        closed_mesh_invariants(&m);
    }

    #[test]
    fn generate_routes_kinds_to_correct_provenance() {
        for kind in [
            PrimitiveKind::Cube,
            PrimitiveKind::Cylinder,
            PrimitiveKind::Sphere,
            PrimitiveKind::Cone,
            PrimitiveKind::Torus,
        ] {
            let m = generate(kind, PrimitiveParams::defaults_for(kind));
            match m.provenance {
                MeshProvenance::Primitive(name) => {
                    let expected = match kind {
                        PrimitiveKind::Cube => "cube",
                        PrimitiveKind::Cylinder => "cylinder",
                        PrimitiveKind::Sphere => "sphere",
                        PrimitiveKind::Cone => "cone",
                        PrimitiveKind::Torus => "torus",
                    };
                    assert_eq!(name, expected);
                }
                _ => panic!("primitive should yield Primitive provenance"),
            }
        }
    }
}
