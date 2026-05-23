//! Phase 5 exit-criteria smoke (PR-5-12).
//!
//! Chains every Phase 5 deliverable that affects project-state
//! shape into one repeatable test so a regression in the
//! `.3mf` save/load round-trip — the primary differentiator
//! Phase 5 was built around — fails loudly with a step-named
//! assertion rather than at some downstream consumer's leaf
//! test.
//!
//! Procedure (human-driven half, includes the slice + send-to-
//! printer walkthrough this Rust test omits) lives in
//! `docs/phase-5-smoke.md`.
//!
//! What this test covers:
//!   1. Build the 3-plate exit fixture programmatically (plate
//!      1 → A1 mini, plates 2-3 → Snapmaker U1).
//!   2. Author the override surface the exit criterion calls
//!      out (project-tier on plate 2, object-tier on a plate-1
//!      object, user-tier project-wide).
//!   3. Bind one material per plate via the auto-bind helper.
//!   4. Save → drop → load → assert every authored field
//!      survived byte-equivalent.
//!
//! What this test DOES NOT cover (manual-half only):
//!   - Slicing each of the 3 plates end-to-end. Each plate
//!     needs its own SliceJobInput + cascade; the per-plate
//!     slice loop is straightforward to drive manually via the
//!     Slice button but adds substantial orchestration here
//!     for diminishing return — phase3_smoke already pins the
//!     single-plate slice contract.
//!   - The recovery dialog flow (PR-5-10) — requires a Tauri
//!     window context the integration runner doesn't have.
//!   - The settings panel's per-plate cascade re-resolution on
//!     printer switch — frontend concern, exercised manually.

use std::path::PathBuf;

use n3o_slic3r_lib::core::printer::profile::BoundingBox;
use n3o_slic3r_lib::core::printer::{lookup, PrinterProfile};
use n3o_slic3r_lib::core::project::{
    format::{read_project, write_project},
    MaterialBinding, PlateId, PrinterBinding, Project,
};
use n3o_slic3r_lib::core::scene::state::{MeshProvenance, NewMesh, NewSceneObject};

fn a1_mini() -> PrinterProfile {
    lookup("bambu-a1-mini").expect("bambu-a1-mini bundled profile present")
}

fn snapmaker_u1() -> PrinterProfile {
    lookup("snapmaker-u1").expect("snapmaker-u1 bundled profile present")
}

fn a1_mini_binding() -> PrinterBinding {
    PrinterBinding {
        printer_identity: "bambu-a1-mini".into(),
        build_plate_identity: "Textured PEI".into(),
    }
}

fn u1_binding() -> PrinterBinding {
    PrinterBinding {
        printer_identity: "snapmaker-u1".into(),
        build_plate_identity: "Magnetic".into(),
    }
}

/// Mesh stub the smoke test plants on each plate. The geometry
/// is the minimal valid triangle libslic3r's 3MF writer accepts
/// (one face, three vertices) — we're not slicing, just exercising
/// the per-plate object surface.
fn triangle_mesh() -> NewMesh {
    NewMesh {
        vertices: vec![0.0, 0.0, 0.0, 10.0, 0.0, 0.0, 0.0, 10.0, 0.0],
        normals: vec![0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0],
        indices: vec![0, 1, 2],
        bounding_box: BoundingBox {
            min: [0.0, 0.0, 0.0],
            max: [10.0, 10.0, 0.0],
        },
        provenance: MeshProvenance::Primitive("test-triangle".into()),
    }
}

/// Tempfile path scoped to this test process so concurrent
/// `cargo test --workspace` runs don't trip on each other.
fn temp_3mf_path() -> PathBuf {
    std::env::temp_dir().join(format!("n3o-phase5-smoke-{}.3mf", std::process::id()))
}

/// Build the 3-plate exit fixture from scratch. Mirrors the
/// state the manual walkthrough produces (minus the geometry
/// import — we plant a stub triangle on each plate instead of
/// loading fourcolor.3mf / cube.stl since the smoke test
/// only validates round-trip, not slicing).
///
/// `register_object` adds to the **active** plate, so we
/// `set_active_plate(id)` before each per-plate registration —
/// the live UI does the same via the plate-tabs strip.
fn build_exit_fixture() -> Project {
    let mut p = Project::default();

    // ── Plate 1 (default plate, pre-existing) → A1 mini ──
    p.rebind_plate_printer(PlateId(1), a1_mini_binding(), &a1_mini())
        .expect("plate 1 rebind to a1 mini");
    p.set_active_plate(PlateId(1)).expect("activate plate 1");
    let mesh_a = p.register_mesh(triangle_mesh());
    let obj_a = p.register_object(NewSceneObject::at_origin(mesh_a, "plate1-cube"));
    // Object-tier override the exit criterion calls out.
    p.object_override_set(
        PlateId(1),
        obj_a,
        "enable_support".into(),
        "1".into(),
    )
    .expect("plate 1 object-tier override");

    // ── Plate 2 → U1, with a project-tier override ──
    let (id2, _) = p.add_plate(None);
    p.rebind_plate_printer(id2, u1_binding(), &snapmaker_u1())
        .expect("plate 2 rebind to u1");
    p.set_active_plate(id2).expect("activate plate 2");
    let mesh_b = p.register_mesh(triangle_mesh());
    p.register_object(NewSceneObject::at_origin(mesh_b, "plate2-fourcolor-stub"));
    p.project_override_set(id2, "layer_height".into(), "0.12".into())
        .expect("plate 2 project-tier override");

    // ── Plate 3 → U1 ──
    let (id3, _) = p.add_plate(None);
    p.rebind_plate_printer(id3, u1_binding(), &snapmaker_u1())
        .expect("plate 3 rebind to u1");
    p.set_active_plate(id3).expect("activate plate 3");
    let mesh_c = p.register_mesh(triangle_mesh());
    p.register_object(NewSceneObject::at_origin(mesh_c, "plate3-20mmbox-stub"));

    // ── Material bindings (one per plate, slot 1 → Generic PLA) ──
    p.set_material_binding(PlateId(1), 1, 1, "Generic PLA".into())
        .expect("plate 1 material binding");
    p.set_material_binding(id2, 1, 1, "Generic PLA".into())
        .expect("plate 2 material binding");
    p.set_material_binding(id3, 1, 1, "Generic PLA".into())
        .expect("plate 3 material binding");

    // ── User-tier override (project-wide) ──
    p.user_overrides
        .insert("travel_speed".into(), "300".into());

    // ── File metadata for the .3mf Title/Designer/License ──
    p.file_metadata
        .insert("Title".into(), "Phase 5 smoke fixture".into());

    p
}

#[test]
fn phase5_smoke_3plate_save_reload_roundtrip() {
    // ─ Step 1: build the fixture in-memory ─────────────────
    let original = build_exit_fixture();
    assert_eq!(original.plates.len(), 3, "fixture must have 3 plates");

    // ─ Step 2: save to a .3mf via PR-5-8 ───────────────────
    let path = temp_3mf_path();
    write_project(&original, &path).expect("step 2: write_project");
    assert!(path.exists(), "step 2: .3mf file should exist after write");

    // ─ Step 3: drop in-memory project, load saved file ─────
    drop(original);
    let reloaded = read_project(&path).expect("step 3: read_project");

    // ─ Step 4: per-field equality assertions ───────────────

    // 4a. Plate count + printer bindings survived.
    assert_eq!(reloaded.plates.len(), 3, "step 4a: plate count must be 3");
    let identities: Vec<&str> = reloaded
        .plates
        .iter()
        .map(|pl| {
            pl.printer
                .as_ref()
                .map(|b| b.printer_identity.as_str())
                .unwrap_or("<unbound>")
        })
        .collect();
    assert_eq!(
        identities,
        vec!["bambu-a1-mini", "snapmaker-u1", "snapmaker-u1"],
        "step 4a: per-plate printer bindings must round-trip",
    );
    let plates_bp: Vec<&str> = reloaded
        .plates
        .iter()
        .map(|pl| {
            pl.printer
                .as_ref()
                .map(|b| b.build_plate_identity.as_str())
                .unwrap_or("<unbound>")
        })
        .collect();
    assert_eq!(
        plates_bp,
        vec!["Textured PEI", "Magnetic", "Magnetic"],
        "step 4a: per-plate build-plate bindings must round-trip",
    );

    // 4b. Project-tier override on plate 2.
    assert_eq!(
        reloaded.plates[1]
            .project_overrides
            .get("layer_height")
            .map(|s| s.as_str()),
        Some("0.12"),
        "step 4b: plate 2 project-tier override must round-trip",
    );

    // 4c. Object-tier override on plate 1.
    //
    // The plate has one object; pick its (only) id and assert
    // the override key/value survived. Object ids regenerate
    // deterministically from the saved JSON, so we don't have
    // to track the original ObjectId across the drop.
    let plate1_obj_id = *reloaded.plates[0]
        .scene
        .objects
        .keys()
        .next()
        .expect("step 4c: plate 1 should have one object");
    assert_eq!(
        reloaded.plates[0]
            .scene
            .object_overrides
            .get(&plate1_obj_id)
            .and_then(|m| m.get("enable_support"))
            .map(|s| s.as_str()),
        Some("1"),
        "step 4c: plate 1 object-tier override must round-trip",
    );

    // 4d. User-tier override (project-wide).
    assert_eq!(
        reloaded.user_overrides.get("travel_speed").map(|s| s.as_str()),
        Some("300"),
        "step 4d: user-tier override must round-trip",
    );

    // 4e. Material bindings — one per plate, slot 1 → Generic PLA.
    for (i, plate) in reloaded.plates.iter().enumerate() {
        assert_eq!(
            plate.material_bindings.len(),
            1,
            "step 4e: plate {} should have exactly 1 material binding, got {}",
            i + 1,
            plate.material_bindings.len(),
        );
        let b: &MaterialBinding = &plate.material_bindings[0];
        assert_eq!(b.model_material, 1, "step 4e: plate {} model_material", i + 1);
        assert_eq!(b.physical_slot, 1, "step 4e: plate {} physical_slot", i + 1);
        assert_eq!(
            b.filament_identity, "Generic PLA",
            "step 4e: plate {} filament_identity",
            i + 1,
        );
    }

    // 4f. File metadata (Title / Designer / License) — the 3MF
    //     container preserves this across save/load.
    assert_eq!(
        reloaded.file_metadata.get("Title").map(|s| s.as_str()),
        Some("Phase 5 smoke fixture"),
        "step 4f: file metadata must round-trip",
    );

    // Cleanup. Failures above panic before this point, leaving
    // the temp file on disk for inspection — that's intentional.
    std::fs::remove_file(&path).ok();
}
