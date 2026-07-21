//! Dump a 3MF's combined geometry (all build objects, transforms applied) as a
//! compact JSON `{positions:[f32...], indices:[u32...]}` for the browser demo's
//! model viewer. Reuses `load_3mf` so component references + transforms resolve
//! exactly like the real importer.
//!
//! Usage: cargo run -p n3o-slic3r --example dump_mesh -- <model.3mf> <out.json>

use std::io::Write;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let src = args.next().ok_or("usage: dump_mesh <model.3mf> <out.json>")?;
    let dst = args.next().ok_or("usage: dump_mesh <model.3mf> <out.json>")?;

    let project = n3o_slic3r_lib::core::threemf::load_3mf(std::path::Path::new(&src))?;

    let mut positions: Vec<f32> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    for obj in &project.objects {
        let mesh = &project.meshes[obj.mesh_idx];
        let m = obj.transform.matrix; // column-major [f32; 16]
        let base = (positions.len() / 3) as u32;
        // Apply the object->world transform to each vertex.
        for v in mesh.vertices.chunks_exact(3) {
            let (x, y, z) = (v[0], v[1], v[2]);
            positions.push(m[0] * x + m[4] * y + m[8] * z + m[12]);
            positions.push(m[1] * x + m[5] * y + m[9] * z + m[13]);
            positions.push(m[2] * x + m[6] * y + m[10] * z + m[14]);
        }
        for &i in &mesh.indices {
            indices.push(base + i);
        }
    }

    let round = |v: f32| (v * 1000.0).round() / 1000.0;
    let mut f = std::io::BufWriter::new(std::fs::File::create(&dst)?);
    write!(f, "{{\"positions\":[")?;
    for (i, p) in positions.iter().enumerate() {
        if i > 0 {
            write!(f, ",")?;
        }
        write!(f, "{}", round(*p))?;
    }
    write!(f, "],\"indices\":[")?;
    for (i, idx) in indices.iter().enumerate() {
        if i > 0 {
            write!(f, ",")?;
        }
        write!(f, "{idx}")?;
    }
    write!(f, "]}}")?;
    f.flush()?;
    eprintln!(
        "wrote {dst}: {} verts, {} tris",
        positions.len() / 3,
        indices.len() / 3
    );
    Ok(())
}
