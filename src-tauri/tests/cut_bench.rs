//! Ad-hoc cut benchmark (ignored). Point N3O_BENCH_MODEL at a model to time the
//! deferred cut (no-connector, painted, and dowel cases):
//!
//!   N3O_BENCH_MODEL=stormtrooper.3mf \
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
        Ok((proj, _recovery_origin)) => proj,
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

    let dowel = |dx: f32, dy: f32| slic3r_ffi::Connector {
        pos: [origin[0] + dx, origin[1] + dy, origin[2]],
        radius: 3.0,
        height: 6.0,
        r_tol: 0.1,
        h_tol: 0.1,
        z_angle: 0.0,
        ty: slic3r_ffi::ConnectorType::Dowel,
        style: slic3r_ffi::ConnectorStyle::Prism,
        shape: slic3r_ffi::ConnectorShape::Circle,
    };
    let run = |label: &str, paint: Option<&[String]>, conns: &[slic3r_ffi::Connector]| {
        let t = Instant::now();
        let r = slic3r_ffi::cut_mesh_deferred(
            &mesh.vertices,
            &mesh.indices,
            origin,
            normal,
            conns,
            paint,
        )
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

    let paint = mesh.paint_colors.as_deref().map(Vec::as_slice);
    run("no-paint, no-conn", None, &[]);
    run("paint, no-conn", paint, &[]);
    let dowels = [dowel(0.0, 0.0), dowel(10.0, 0.0), dowel(-10.0, 0.0)];
    run("paint, 3 dowels", paint, &dowels);
}
