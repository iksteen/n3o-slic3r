//! Ad-hoc cut benchmark (ignored). Point N3O_BENCH_MODEL at a model and run
//! with N3O_CUT_TIMING=1 for the FFI phase breakdown:
//!
//!   N3O_BENCH_MODEL=stormtrooper.3mf N3O_CUT_TIMING=1 \
//!     cargo test -p n3o-slic3r --features test-fixtures --test cut_bench \
//!       -- --ignored --nocapture

use std::path::PathBuf;
use std::time::Instant;

use n3o_slic3r_lib::core::orca_import::import;
use n3o_slic3r_lib::core::project::format::read_project;

#[test]
#[ignore]
fn bench_cut() {
    let Ok(path) = std::env::var("N3O_BENCH_MODEL") else {
        eprintln!("set N3O_BENCH_MODEL to a model path");
        return;
    };
    let p = PathBuf::from(path);
    // Native .n3o via read_project; foreign (Orca/BBS) .3mf via the import path.
    let proj = match read_project(&p) {
        Ok(proj) => proj,
        Err(_) => import(&p).expect("import model").0,
    };
    let mesh = proj
        .meshes
        .values()
        .max_by_key(|m| m.indices.len())
        .expect("at least one mesh");
    let tris = mesh.indices.len() / 3;
    let painted = mesh
        .paint_colors
        .as_ref()
        .map_or(0, |p| p.iter().filter(|s| !s.is_empty()).count());
    eprintln!("mesh: {tris} triangles, {painted} painted faces");

    let bb = &mesh.bounding_box;
    let origin = [
        ((bb.min[0] + bb.max[0]) / 2.0) as f32,
        ((bb.min[1] + bb.max[1]) / 2.0) as f32,
        ((bb.min[2] + bb.max[2]) / 2.0) as f32,
    ];
    let normal = [0.0f32, 0.0, 1.0]; // horizontal cut through the middle

    let run = |label: &str, paint: Option<&[String]>| {
        let t = Instant::now();
        let r =
            slic3r_ffi::cut_mesh_connectors(&mesh.vertices, &mesh.indices, origin, normal, &[], paint)
                .expect("cut");
        let painted = |h: &slic3r_ffi::CutHalf| {
            h.paint.as_ref().map_or(0, |p| p.iter().filter(|s| !s.is_empty()).count())
        };
        eprintln!(
            "{label}: total {:.0} ms (pos {} tris/{} painted, neg {} tris/{} painted)",
            t.elapsed().as_secs_f64() * 1000.0,
            r.pos.indices.len() / 3,
            painted(&r.pos),
            r.neg.indices.len() / 3,
            painted(&r.neg),
        );
    };

    run("no-paint", None);
    if mesh.paint_colors.is_some() {
        run("with-paint", mesh.paint_colors.as_deref().map(Vec::as_slice));
    }
}
