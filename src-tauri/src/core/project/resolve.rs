//! Read-side cascade + tower resolution for project plates.
//!
//! The domain logic behind the project's read-only Tauri commands
//! (`core::project::commands`): composing a plate's (or a bare printer
//! instance's) authored cascade, resolving it — flattened for the
//! settings-panel ladder or tier-aware for the "why is X = Y" trace —
//! and deriving a plate's priming-tower geometry for the viewport
//! overlay. The `#[tauri::command]` wrappers in `commands` lock the
//! project state and delegate here; these functions take a `&Project`
//! so they're testable without a Tauri `State`.

use super::PlateId;
use super::Project;

/// Map a winning rule's source path to the cascade *layer* the settings
/// panel ladder renders it under. The fragment paths `compose_cascade`
/// stamps are deterministic per step, so this is an exact classification
/// (not a heuristic): the process fragment and its stamped user overrides
/// (`<process-overrides>`) → the `"user"` row (labeled "Profile" — the
/// selected quality/process profile), nozzle fragment + extruder-vector
/// assembly → `"nozzle"`, bed → `"build_plate"`, filament → `"filament"`, and
/// `machine.toml` + synthesized machine-topology rules + the instance's
/// `<machine-overrides>` → `"printer"`. Returns the frontend `CascadeLayer`
/// id, or `None` for `<plate-overrides>` (the panel draws override tiers
/// itself) / anything unrecognized.
fn layer_for_source(path: &std::path::Path) -> Option<&'static str> {
    let s = path.to_string_lossy();
    // The Profile row = the quality/process profile *and* the per-user
    // overrides stamped onto it (`<process-overrides>`), so a saved quality
    // setting attributes to Profile, not — via the frontend's null fallback —
    // to Printer.
    if s.contains("/processes/") || s == "<process-overrides>" {
        Some("user")
    } else if s.contains("/beds/") {
        Some("build_plate")
    } else if s.contains("/filament/")
        || s.contains("<filament-vector-assembly>")
        || s.contains("<filament-colour-synthesis>")
    {
        Some("filament")
    } else if s.contains("/nozzles/") || s.contains("<extruder-vector-assembly>") {
        Some("nozzle")
    } else if s.contains("machine.toml")
        || s.contains("<flush-defaults>")
        || s.contains("<filament-topology>")
        || s == "<machine-overrides>"
    {
        Some("printer")
    } else {
        None
    }
}

/// One resolved key for the settings-panel ladder: the cascade-resolved
/// `value` (fragments only — override tiers are drawn frontend-side) plus
/// the `source_layer` it won from.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PlateResolvedEntry {
    pub value: String,
    pub source_layer: Option<String>,
}

/// The whole resolved map for a plate, keyed by libslic3r setting key.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PlateResolvedJson {
    pub entries: std::collections::HashMap<String, PlateResolvedEntry>,
}

/// Resolve a plate's cascade for the settings panel: compose the bound
/// instance's fragments against the plate's effective process
/// (`plate.quality_profile` overriding the instance default) and resolve
/// each key, tagging it with the layer it came from. **No** override tiers
/// are folded in — the panel draws project/object rows from its own maps;
/// these are the fragment-resolved values that fill the cascade rows
/// (Printer / Build plate / Filament / Profile). Returns an empty map for
/// an unbound plate.
pub fn resolve_plate_cascade(p: &Project, plate_id: PlateId) -> Result<PlateResolvedJson, String> {
    use std::collections::{BTreeMap, HashMap};
    // Fragment-only resolution (no override tiers) — the panel draws
    // project/object rows from its own maps.
    let Some(resolved) = resolve_plate(p, plate_id, &BTreeMap::new())? else {
        return Ok(PlateResolvedJson {
            entries: HashMap::new(),
        });
    };
    let entries = resolved
        .into_iter()
        .map(|(k, v)| {
            (
                k,
                PlateResolvedEntry {
                    source_layer: layer_for_source(&v.winning_rule.path).map(str::to_owned),
                    value: v.value,
                },
            )
        })
        .collect();
    Ok(PlateResolvedJson { entries })
}

/// Compose + resolve a plate's cascade, folding `overrides` in as the
/// top-precedence layer exactly as the slice path folds
/// `Plate.project_overrides` (`slice::orchestrator::resolve_cascade`).
/// Pass an empty map for the fragment-only resolution the settings
/// ladder wants, or the plate's project overrides when the resolved
/// value has to match what actually slices (e.g. the priming-tower
/// position). Returns `None` for an unbound plate (no printer instance).
fn resolve_plate(
    p: &Project,
    plate_id: PlateId,
    overrides: &std::collections::BTreeMap<String, String>,
) -> Result<Option<crate::core::cascade::Resolved>, String> {
    let plate = p
        .plate(plate_id)
        .ok_or_else(|| format!("unknown plate id {plate_id:?}"))?;
    let Some(instance_id) = plate.printer_instance_id() else {
        return Ok(None);
    };
    let instance = crate::core::printer::lookup_instance(instance_id)
        .ok_or_else(|| format!("unknown printer instance `{instance_id}`"))?;
    resolve_instance_cascade(&instance, plate.quality_profile.as_deref(), overrides).map(Some)
}

/// Compose + resolve a printer instance's cascade, independent of any
/// plate. `quality_profile` overrides the instance's bound process when
/// set (the plate path passes the plate's); `overrides` folds in as the
/// top-precedence layer. The instance's own slot loadout supplies the
/// filament context for `when.filament.*` predicates. Used by the plate
/// resolve above and by the printer panel's machine-settings surface (to
/// show each option's resolved base value, not the bare engine default).
pub fn resolve_instance_cascade(
    instance: &crate::core::printer::PrinterInstance,
    quality_profile: Option<&str>,
    overrides: &std::collections::BTreeMap<String, String>,
) -> Result<crate::core::cascade::Resolved, String> {
    let (cascade, ctx) = compose_instance_cascade_and_ctx(instance, quality_profile)?;
    // `overrides` is the project tier — apply it as the two-phase override
    // layer (matching the slice path) rather than baking it into the cascade.
    // Empty map → an empty tier, i.e. plain fragment resolution.
    let tiers = crate::core::cascade::OverrideTiers {
        user: vec![],
        project: tier("<project-overrides>", overrides),
        object: None,
    };
    Ok(crate::core::cascade::to_resolved(
        &crate::core::cascade::resolve_with_overrides(&cascade, &tiers, &ctx),
    ))
}

/// Compose the authored cascade + build the slicing context for a printer
/// instance. Shared by the flattened resolve above and the tier-aware trace
/// resolve below. The instance's slot loadout supplies the filament context
/// for `when.filament.*` predicates.
fn compose_instance_cascade_and_ctx(
    instance: &crate::core::printer::PrinterInstance,
    quality_profile: Option<&str>,
) -> Result<
    (
        crate::core::cascade::Cascade,
        crate::core::project::SlicingContext,
    ),
    String,
> {
    let printer = crate::core::printer::lookup(&instance.vendor_profile_ref)
        .ok_or_else(|| format!("unknown vendor profile `{}`", instance.vendor_profile_ref))?;
    let bed_identity = instance.bed.identity.clone();
    let bed = crate::core::scene::build_plate::lookup(&bed_identity).unwrap_or_else(|| {
        crate::core::scene::build_plate::BuildPlate {
            libslic3r_curr_bed_type: format!("{bed_identity} Plate"),
            identity: bed_identity.clone(),
        }
    });
    // Filament context: one filament per physical slot (always ≥1, so
    // predicates resolve and the empty plate still shows the instance's
    // filaments). active_slot 0 → slot 0.
    //
    // Scope note: this is the instance's slot view. The slice path
    // (`slice::input`) instead fans one filament per *material* via the
    // plate's `material_to_slot`. They agree for the common case (material
    // i bound to slot i, the auto-bind default), but on an AMS printer
    // where the user has manually bound a material to a slot holding a
    // different filament *type*, a `when.filament.type`-gated value the
    // ladder shows can differ from what that material slices with. Process
    // / printer / bed rows (incl. the headline Profile attribution) are
    // unaffected. A per-material filament view here is the follow-up.
    let filaments: Vec<std::sync::Arc<crate::core::filament::FilamentProfile>> = instance
        .extruders
        .iter()
        .flat_map(|e| &e.slots)
        .map(|slot| {
            let id = slot
                .filament_identity
                .as_deref()
                .unwrap_or(instance.default_filament_fragment_slug.as_str());
            std::sync::Arc::new(crate::core::filament::lookup(id).unwrap_or_else(|| {
                crate::core::filament::FilamentProfile {
                    identity: id.to_owned(),
                    base_type: "PLA".into(),
                    vendor: None,
                    color: None,
                }
            }))
        })
        .collect();

    let effective = crate::core::profile_library::with_quality_profile(instance, quality_profile);
    let cascade = crate::core::profile_library::compose_cascade(&effective, &[])
        .map_err(|e| format!("compose: {e}"))?;
    let ctx = crate::core::project::SlicingContext::new(
        std::sync::Arc::new(printer),
        std::sync::Arc::new(bed),
        filaments,
    );
    Ok((cascade, ctx))
}

/// The plate's *effective* config as a flat `key → value` map: the bound
/// instance's fragment cascade with the user + project override tiers
/// folded in — what an object on this plate resolves against before its
/// own overrides. The baseline "Add model + settings" diffs a foreign
/// project's config against. Empty for an unbound plate (no baseline →
/// every foreign key counts as a difference, the safe direction).
pub fn plate_effective_baseline(
    p: &Project,
    plate_id: PlateId,
) -> Result<std::collections::BTreeMap<String, String>, String> {
    let plate = p
        .plate(plate_id)
        .ok_or_else(|| format!("unknown plate id {plate_id:?}"))?;
    let Some(instance_id) = plate.printer_instance_id() else {
        return Ok(std::collections::BTreeMap::new());
    };
    let instance = crate::core::printer::lookup_instance(instance_id)
        .ok_or_else(|| format!("unknown printer instance `{instance_id}`"))?;
    let (cascade, ctx) =
        compose_instance_cascade_and_ctx(&instance, plate.quality_profile.as_deref())?;
    let user: std::collections::BTreeMap<String, String> = p
        .user_overrides
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let project: std::collections::BTreeMap<String, String> = plate
        .project_overrides
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let tiers = crate::core::cascade::OverrideTiers {
        user: tier("<user-overrides>", &user),
        project: tier("<project-overrides>", &project),
        object: None,
    };
    let resolved = crate::core::cascade::to_resolved(
        &crate::core::cascade::resolve_with_overrides(&cascade, &tiers, &ctx),
    );
    Ok(resolved.into_iter().map(|(k, v)| (k, v.value)).collect())
}

/// Wrap a flat override map as a single-source override tier labeled
/// `label`, or an empty tier when there's nothing to override.
fn tier(
    label: &str,
    overrides: &std::collections::BTreeMap<String, String>,
) -> Vec<crate::core::cascade::FlatOverrides> {
    if overrides.is_empty() {
        return vec![];
    }
    vec![crate::core::cascade::FlatOverrides {
        source: crate::core::cascade::SourceLocation {
            path: std::path::PathBuf::from(label),
            line: 1,
        },
        entries: overrides.clone(),
    }]
}

/// Resolved priming-tower placement + footprint for one plate, in bed
/// millimetres (world space — the bed's corner is the world origin).
/// `x`/`y` are the tower's lower-left corner (`wipe_tower_x/y`); `width`
/// is the square footprint (`prime_tower_width`); `brim` the skirt that
/// rings it; `rotation` is degrees about the tower (0 for both MVP
/// printers — carried for fidelity).
#[derive(Debug, Clone, serde::Serialize)]
pub struct TowerGeometry {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub brim: f64,
    pub rotation: f64,
    /// Distinct material count this resolved against. The viewport pairs a
    /// sliced tower mesh with the count it was sliced at and treats the
    /// mesh as stale once this diverges (the only thing that reshapes the
    /// tower; moving it does not).
    pub material_count: usize,
    /// The plate's bound printer instance. The viewport also keys the cached
    /// tower mesh on this: a rebind to a different printer reshapes the tower
    /// (and doesn't re-slice), so the mesh must go stale even when the
    /// material count is unchanged. `None` only if the plate is unbound.
    pub printer_instance_id: Option<String>,
}

/// The active plate's priming-tower geometry for the viewport overlay,
/// or `None` when the plate is unbound or has no tower
/// (`enable_prime_tower` off). Visibility keys on `enable_prime_tower`,
/// not the purge-tower capability: both MVP printers run a tower (the
/// A1 mini purges through it, the U1 uses it for toolhead re-entry), and
/// only the purge-*volume* options are toolchanger-gated. The plate's
/// project overrides are folded in, so the box tracks exactly where the
/// tower slices — including a position the user has dragged it to.
pub fn tower_geometry_for_plate(
    p: &Project,
    plate_id: PlateId,
) -> Result<Option<TowerGeometry>, String> {
    let Some(plate) = p.plate(plate_id) else {
        return Ok(None);
    };
    // A wipe/prime tower is only generated for a multi-material print — ≥2
    // distinct physical filament *slots* in use. With a single slot there are no
    // tool changes, so libslic3r emits no tower regardless of `enable_prime_tower`;
    // the overlay must match. `materials_on_plate` counts each object's base
    // `extruder_id` *and* any MMU paint within its mesh (so a single painted
    // object reads as multi-material); mapping each material through
    // `material_to_slot` then collapses two materials that share one slot — same
    // physical filament, no swap, no tower. Unmapped materials key on their own
    // index so they stay distinct.
    let mut slots: std::collections::HashSet<(bool, u8, u8)> = std::collections::HashSet::new();
    for m in p.materials_on_plate(plate) {
        match plate.material_to_slot.get(&m) {
            Some(sr) => slots.insert((true, sr.extruder, sr.slot)),
            None => slots.insert((false, m, 0)),
        };
    }
    if slots.len() < 2 {
        return Ok(None);
    }
    let material_count = slots.len();
    // Fold the plate's project-tier overrides into the compose exactly as
    // the slice path does, so a dragged position resolves here too.
    let overrides: std::collections::BTreeMap<String, String> =
        plate.project_overrides.clone().into_iter().collect();
    let Some(resolved) = resolve_plate(p, plate_id, &overrides)? else {
        return Ok(None);
    };

    let enabled = resolved
        .get("enable_prime_tower")
        .map(|v| matches!(v.value.trim(), "1" | "true" | "True"))
        .unwrap_or(false);
    if !enabled {
        return Ok(None);
    }

    // Cascade-resolved value if a fragment/override sets it, else the
    // engine's compiled default (the U1 pins no position, so its tower
    // sits at libslic3r's default until dragged).
    let num = |key: &str| -> Option<f64> {
        resolved
            .get(key)
            .map(|v| v.value.clone())
            .or_else(|| crate::core::printer::engine_default_serialized(key))
            .and_then(|s| s.trim().parse::<f64>().ok())
    };

    Ok(Some(TowerGeometry {
        x: num("wipe_tower_x").unwrap_or(0.0),
        y: num("wipe_tower_y").unwrap_or(0.0),
        width: num("prime_tower_width").unwrap_or(0.0),
        brim: num("prime_tower_brim_width").unwrap_or(0.0),
        rotation: num("wipe_tower_rotation_angle").unwrap_or(0.0),
        material_count,
        printer_instance_id: plate.printer_instance_id().map(str::to_owned),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::project::mutation::test_support::bound_default_project;
    use std::path::Path;

    #[test]
    fn layer_for_source_maps_fragments_to_rows() {
        let p = |s: &str| layer_for_source(Path::new(s));
        assert_eq!(
            p("bbl/printer/bambu-lab-a1-mini/processes/0.20mm-standard.toml"),
            Some("user"),
            "process fragment → Profile row",
        );
        assert_eq!(
            p("bbl/printer/bambu-lab-a1-mini/beds/textured-pei.toml"),
            Some("build_plate")
        );
        assert_eq!(p("generic/filament/generic-pla.toml"), Some("filament"));
        assert_eq!(
            p("bbl/printer/bambu-lab-a1-mini/machine.toml"),
            Some("printer")
        );
        assert_eq!(p("<filament-topology>"), Some("printer"));
        assert_eq!(
            p("bbl/printer/bambu-lab-a1-mini/nozzles/0.4.toml"),
            Some("nozzle"),
            "nozzle fragment → its own Nozzle row, split out of Printer",
        );
        assert_eq!(p("<extruder-vector-assembly>"), Some("nozzle"));
        assert_eq!(p("<plate-overrides>"), None);
        // Synthesized override rules attribute to their tier's row — a stamped
        // quality override to Profile ("user"), the instance machine config to
        // Printer — not to the frontend's null→Printer fallback.
        assert_eq!(
            p("<process-overrides>"),
            Some("user"),
            "stamped process override → Profile row",
        );
        assert_eq!(p("<machine-overrides>"), Some("printer"));
    }

    #[test]
    fn plate_resolve_attributes_outer_wall_speed_to_the_profile_layer() {
        // FFI + the bundled profile library back compose+resolve.
        let _ = slic3r_ffi::init(None, 3);
        // Default project: plate 0 bound to the bundled A1 mini (`bambi`,
        // quality_profile = "0.20mm-standard").
        let project = bound_default_project();
        let plate_id = project.plates[0].id;
        // Sanity: the test env actually bound an instance.
        assert!(project.plates[0].printer_instance_id().is_some());

        let resolved = resolve_plate_cascade(&project, plate_id).expect("resolve");
        let ow = resolved
            .entries
            .get("outer_wall_speed")
            .expect("outer_wall_speed resolved");
        // 0.20mm-standard's process fragment sets 200, attributed to the
        // "Profile" (process/quality-profile) row.
        assert_eq!(ow.value, "200");
        assert_eq!(ow.source_layer.as_deref(), Some("user"));
    }

    #[test]
    fn plate_resolve_follows_a_per_plate_quality_profile() {
        let _ = slic3r_ffi::init(None, 3);
        let mut project = bound_default_project();
        let plate_id = project.plates[0].id;
        // Switch this plate to the Strength preset (60), leaving the
        // instance's default (Standard, 200) untouched.
        let inst = project
            .active_plate()
            .printer_instance_id()
            .and_then(crate::core::printer::lookup_instance);
        project
            .set_plate_quality_profile(plate_id, Some("0.20mm-strength".into()), inst.as_ref())
            .expect("set strength");
        let resolved = resolve_plate_cascade(&project, plate_id).expect("resolve");
        let ow = resolved.entries.get("outer_wall_speed").expect("present");
        assert_eq!(ow.value, "60", "the plate's own process wins");
        assert_eq!(ow.source_layer.as_deref(), Some("user"));
    }

    /// Add a cube on the active plate assigned to `material` (its 1-based
    /// filament index). Two distinct materials make the plate multi-material
    /// — the condition a wipe/prime tower is generated for.
    fn add_cube(p: &mut Project, material: u8) {
        use crate::core::scene::state::{MeshProvenance, NewMesh, NewSceneObject};
        use crate::core::scene::transform::Transform;
        let mesh = p.register_mesh(NewMesh {
            vertices: vec![0.0; 24],
            indices: vec![0, 1, 2],
            paint_colors: None,
            support_paint: None,
            bounding_box: crate::core::printer::profile::BoundingBox {
                min: [0.0, 0.0, 0.0],
                max: [1.0, 1.0, 1.0],
            },
            provenance: MeshProvenance::Primitive("cube".into()),
        });
        p.register_object(
            NewSceneObject {
                mesh,
                transform: Transform::IDENTITY,
                name: format!("cube-m{material}"),
                visible: true,
                extruder_id: Some(material),
                group: None,
            },
            None,
        );
    }

    #[test]
    fn tower_geometry_for_a1_mini_reads_pinned_position_and_footprint() {
        let _ = slic3r_ffi::init(None, 3);
        let mut project = bound_default_project();
        let plate_id = project.plates[0].id;
        // Multi-material plate → the tower is generated.
        add_cube(&mut project, 1);
        add_cube(&mut project, 2);
        let bound = project.plates[0].printer_instance_id().map(str::to_owned);
        let t = tower_geometry_for_plate(&project, plate_id)
            .expect("ok")
            .expect("the A1 mini runs a prime tower for a multi-material plate");
        // Position pinned in machine.toml; footprint in the process fragment.
        assert_eq!(t.x, 5.0, "wipe_tower_x");
        assert_eq!(t.y, 130.0, "wipe_tower_y");
        assert_eq!(t.width, 35.0, "prime_tower_width");
        assert_eq!(t.brim, 3.0, "prime_tower_brim_width");
        // Carries the bound printer instance so the viewport can drop a cached
        // tower mesh on a rebind to a different printer (which reshapes the
        // tower without re-slicing).
        assert!(bound.is_some(), "default plate is auto-bound");
        assert_eq!(
            t.printer_instance_id, bound,
            "carries the bound instance id"
        );
    }

    #[test]
    fn tower_geometry_is_none_for_a_single_material_plate() {
        let _ = slic3r_ffi::init(None, 3);
        let mut project = Project::default();
        let plate_id = project.plates[0].id;
        // One material (or none) → no tool changes → no tower, even though
        // enable_prime_tower is set.
        add_cube(&mut project, 1);
        assert!(
            tower_geometry_for_plate(&project, plate_id)
                .expect("ok")
                .is_none(),
            "single-material plate must not show a tower",
        );
    }

    #[test]
    fn tower_geometry_is_none_when_two_materials_share_one_slot() {
        let _ = slic3r_ffi::init(None, 3);
        let mut project = Project::default();
        let plate_id = project.plates[0].id;
        add_cube(&mut project, 1);
        add_cube(&mut project, 2);
        // Both materials mapped to the same physical slot → same filament, no
        // swap, no tower (even though two distinct material indices are in use).
        let sr = crate::core::printer::SlotRef {
            extruder: 0,
            slot: 0,
        };
        project.plates[0].material_to_slot.insert(1, sr);
        project.plates[0].material_to_slot.insert(2, sr);
        assert!(
            tower_geometry_for_plate(&project, plate_id)
                .expect("ok")
                .is_none(),
            "two materials on one slot must not show a tower",
        );
    }

    #[test]
    fn tower_geometry_tracks_a_project_override_position() {
        let _ = slic3r_ffi::init(None, 3);
        let mut project = bound_default_project();
        let plate_id = project.plates[0].id;
        add_cube(&mut project, 1);
        add_cube(&mut project, 2);
        // Dragging the tower writes a project-tier wipe_tower_x override;
        // the geometry the viewport reads must fold it in (so the box
        // tracks where the tower will actually slice).
        project
            .project_override_set(plate_id, "wipe_tower_x".into(), "42".into())
            .expect("set override");
        let t = tower_geometry_for_plate(&project, plate_id)
            .expect("ok")
            .expect("tower");
        assert_eq!(t.x, 42.0, "overridden position resolves here");
        assert_eq!(t.y, 130.0, "the untouched axis stays pinned");
    }

    #[test]
    fn plate_resolve_attributes_nozzle_keys_to_the_nozzle_layer() {
        // The nozzle fragment (via the extruder-vector assembly) is its own
        // ladder row, not folded into Printer. Check both a machine-bucket
        // key (`nozzle_diameter`, hidden in the panel) and a user-visible
        // one (`retraction_length`, shown under Retraction) so the row is
        // demonstrably reachable from the UI.
        let _ = slic3r_ffi::init(None, 3);
        let project = bound_default_project();
        let plate_id = project.plates[0].id;
        let resolved = resolve_plate_cascade(&project, plate_id).expect("resolve");
        for key in ["nozzle_diameter", "retraction_length"] {
            let e = resolved
                .entries
                .get(key)
                .unwrap_or_else(|| panic!("{key} resolved"));
            assert_eq!(e.source_layer.as_deref(), Some("nozzle"), "{key} → Nozzle");
        }
    }
}
