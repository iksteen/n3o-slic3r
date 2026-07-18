//! Send-orchestration helpers shared by the driver send/export commands.
//!
//! These are the pure/async building blocks the `#[tauri::command]`
//! functions in [`super::commands`] compose: bundle wrapping, raw
//! G-code reads, send-name derivation, the plate's AMS routing
//! collectors, the plate's printer-model lookup, and the pre-send
//! plugin hook. The commands themselves stay thin adapters over these.

use std::sync::Mutex;

use super::ams::{ams_bindings_for_plate, ams_mapping_for_plate, AmsMappingV2};
use super::traits::{DriverKind, SendPayload};
use crate::core::plugin::commands::PluginHostState;
use crate::core::plugin::{DispatchGate, HookKind, PayloadKind, PreSendHook, SendTarget};
use crate::core::project::model::sanitize_basename;
use crate::core::project::{PlateId, Session};
use crate::core::threemf::{fixture_input, write_sliced_3mf, AmsBinding};

/// Wrap a raw G-code file on disk into a Bambu-flavored
/// `.gcode.3mf` bundle byte buffer. The bundle carries the raw
/// G-code, the human-readable `Title` metadata, the per-plate AMS
/// slot map, and the plate thumbnail (see the enrichment below).
///
/// Runs on `spawn_blocking` because the writer is sync-IO + does
/// per-entry MD5 work; calling it from an async command without
/// the offload would stall the runtime.
pub(super) async fn wrap_gcode_as_3mf(
    gcode_path: String,
    plate_id: u32,
    title: String,
    ams_bindings: Vec<AmsBinding>,
    thumbnail_png: Option<Vec<u8>>,
) -> Result<Vec<u8>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let gcode_bytes =
            std::fs::read(&gcode_path).map_err(|e| format!("read gcode at {gcode_path}: {e}"))?;
        let mut input = fixture_input(plate_id, gcode_bytes);
        // Human-readable project/plate Title — the writer emits it as
        // `<metadata name="Title">` in the bundle's 3dmodel.model so the
        // printer / a re-import shows where the job came from.
        input
            .file_metadata
            .insert("Title".to_owned(), title);
        // Inject the per-plate AMS slot map. For Bambi
        // standalone (1 slot, no AMS) this is `[{material: 1,
        // ams_slot: 1}]` — identity-shaped. For a future
        // AMS-equipped instance the picker drives the values.
        if let Some(plate) = input.plates.iter_mut().find(|p| p.plate_id == plate_id) {
            plate.ams_bindings = ams_bindings;
            // The Bambu screen reads `Metadata/plate_N.png`; drop the
            // frontend-rendered preview in so it shows the model, not a
            // placeholder.
            plate.thumbnail_png = thumbnail_png;
        }
        let tmp = tempfile::Builder::new()
            .suffix(".gcode.3mf")
            .tempfile()
            .map_err(|e| format!("create temp bundle: {e}"))?;
        write_sliced_3mf(&input, tmp.path())
            .map_err(|e| format!("write .gcode.3mf bundle: {e}"))?;
        std::fs::read(tmp.path()).map_err(|e| format!("read back .gcode.3mf bundle: {e}"))
    })
    .await
    .map_err(|e| format!("wrap task join: {e}"))?
}

/// Read a raw G-code file off disk into memory for the U1 send
/// path. Sliced bundles can be tens of megabytes — the read is
/// offloaded to a blocking thread to keep the Tauri runtime
/// responsive.
pub(super) async fn read_gcode_bytes(gcode_path: String) -> Result<Vec<u8>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        std::fs::read(&gcode_path).map_err(|e| format!("read gcode at {gcode_path}: {e}"))
    })
    .await
    .map_err(|e| format!("read task join: {e}"))?
}

/// Derive the user-facing names for a plate's sliced output from the
/// project title + the plate's name:
/// - a filename-safe combined basename (`MyPrint_Lid`) for the FTPS
///   upload / U1 store name / export default, and
/// - a human-readable Title (`MyPrint — Lid`) for the `.gcode.3mf`
///   `Title` metadata.
///
/// Falls back to `untitled_Plate <n>` shape when the plate is unknown
/// (the project still contributes its title), so the names are always
/// well-formed.
pub(super) fn derive_send_names(session: &Mutex<Session>, plate_id: u32) -> (String, String) {
    let s = session
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let project_title = s.title();
    let plate_name = s
        .project
        .plate(PlateId(plate_id))
        .map(|pl| pl.name.clone())
        .unwrap_or_else(|| format!("Plate {plate_id}"));
    let combined = format!(
        "{}_{}",
        sanitize_basename(&project_title),
        sanitize_basename(&plate_name)
    );
    let title = format!("{project_title} — {plate_name}");
    (combined, title)
}

/// Look up the active project's plate-side AMS bindings for use in
/// the send/dry-send path. Returns an empty vec when the plate isn't
/// found or has no mappings — both safe defaults the firmware
/// tolerates on a single-slot, no-AMS print.
pub(super) fn collect_ams_bindings(session: &Mutex<Session>, plate_id: u32) -> Vec<AmsBinding> {
    let Ok(s) = session.lock() else {
        return Vec::new();
    };
    let Some(plate) = s.project.plate(PlateId(plate_id)) else {
        return Vec::new();
    };
    ams_bindings_for_plate(plate)
}

/// Plate-side AMS routing for the Bambu MQTT `project_file` print
/// command: `(use_ams, ams_mapping, ams_mapping2)`. Arrays are
/// sized to the plate's materials list length; empty when the
/// plate is unknown, unbound, or carries no materials — the
/// firmware falls back to the external spool in that case.
pub(super) fn collect_ams_mapping(
    session: &Mutex<Session>,
    plate_id: u32,
) -> (bool, Vec<i8>, Vec<AmsMappingV2>) {
    let default = (false, Vec::new(), Vec::new());
    let Ok(s) = session.lock() else {
        return default;
    };
    let Some(plate) = s.project.plate(PlateId(plate_id)) else {
        return default;
    };
    ams_mapping_for_plate(plate)
}

/// Resolve the printer model bound to `plate_id`, for pre-send
/// `printer_compatibility` enforcement. `None` when the plate isn't
/// bound or the instance/profile can't be resolved — the printer check
/// is then simply skipped (the gate treats `None` as "any").
pub(super) fn plate_printer_model(session: &Mutex<Session>, plate_id: u32) -> Option<String> {
    let inst_id = {
        let s = session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        s.project.plate(PlateId(plate_id))?
            .printer_instance_id()?
            .to_owned()
    };
    let inst = crate::core::printer::lookup_instance(&inst_id)?;
    let profile = crate::core::printer::lookup(&inst.vendor_profile_ref)?;
    Some(profile.model.clone())
}

/// The sticky per-print send options of the instance bound to
/// `plate_id`. Falls back to [`SendOptions::default`] (leveling on,
/// calibrations/timelapse off) when the plate isn't bound or the
/// instance can't be resolved.
pub(super) fn plate_send_options(
    session: &Mutex<Session>,
    plate_id: u32,
) -> crate::core::printer::instance::SendOptions {
    let inst_id = {
        let s = session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        s.project.plate(PlateId(plate_id))
            .and_then(|pl| pl.printer_instance_id().map(str::to_owned))
    };
    inst_id
        .and_then(|id| crate::core::printer::lookup_instance(&id))
        .map(|inst| inst.send_options)
        .unwrap_or_default()
}

/// Installed nozzle diameter per physical extruder of the instance
/// bound to `plate_id`, parsed from the instance's per-toolhead nozzle
/// SKUs. Empty when the plate isn't bound / the instance is gone / a
/// diameter doesn't parse — the start script then omits
/// `NOZZLE_DIAMETER_LIST` rather than sending a partial list the
/// firmware would misalign.
pub(super) fn plate_nozzle_diameters(session: &Mutex<Session>, plate_id: u32) -> Vec<f64> {
    let inst_id = {
        let s = session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        s.project.plate(PlateId(plate_id))
            .and_then(|pl| pl.printer_instance_id().map(str::to_owned))
    };
    let Some(inst) = inst_id.and_then(|id| crate::core::printer::lookup_instance(&id)) else {
        return Vec::new();
    };
    let diameters: Vec<f64> = inst
        .extruders
        .iter()
        .filter_map(|e| e.installed_nozzle.diameter.parse::<f64>().ok())
        .collect();
    if diameters.len() == inst.extruders.len() {
        diameters
    } else {
        Vec::new()
    }
}

/// The firmware `extruder_map_table` for the plate, as `(logical,
/// physical)` pairs — one per material (logical slot `= material − 1`).
/// The U1 slices in logical material space (`T<material − 1>`); this
/// table is what routes each logical tool to its bound physical toolhead
/// at print time. Sticky on the printer, so we always send the full set
/// of referenced slots to overwrite whatever a prior session (or
/// Snapmaker's own software) left — otherwise a stale table misroutes.
///
/// Physical target = the material's bound toolhead
/// (`material_to_slot[m].extruder`); an unbound material falls back to
/// toolhead 0 (`pre_slice_gate` binds every referenced material, so this
/// only covers index gaps that the G-code never emits). Empty when the
/// plate isn't found.
pub(super) fn u1_map_table(session: &Mutex<Session>, plate_id: u32) -> Vec<(u8, u8)> {
    let s = session
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some(plate) = s.project.plate(PlateId(plate_id)) else {
        return Vec::new();
    };
    (1..=plate.material_count())
        .map(|m| {
            let physical = plate.material_to_slot.get(&m).map_or(0, |sr| sr.extruder);
            (m - 1, physical)
        })
        .collect()
}

/// Translate the G-code's used *logical* slots to the *physical*
/// toolheads they map to — `FLOW_CALIBRATE_EXTRUDERS` is per-physical.
/// Deduped + sorted (two materials may share a toolhead). Logical slots
/// with no map entry are dropped.
pub(super) fn physical_extruders_used(used_logical: &[u8], map_table: &[(u8, u8)]) -> Vec<u8> {
    let mut physical: Vec<u8> = used_logical
        .iter()
        .filter_map(|l| map_table.iter().find(|(lg, _)| lg == l).map(|(_, p)| *p))
        .collect();
    physical.sort_unstable();
    physical.dedup();
    physical
}

/// Per-logical nozzle diameters for `NOZZLE_DIAMETER_LIST` (which the
/// firmware keys by logical slot): each map-table entry's physical
/// toolhead's nozzle, in logical order. Empty when the physical nozzles
/// couldn't be resolved — omit rather than send a zero-filled list the
/// firmware would reject.
pub(super) fn logical_nozzle_diameters(map_table: &[(u8, u8)], physical_nozzles: &[f64]) -> Vec<f64> {
    if physical_nozzles.is_empty() {
        return Vec::new();
    }
    map_table
        .iter()
        .map(|(_, physical)| physical_nozzles.get(*physical as usize).copied().unwrap_or(0.0))
        .collect()
}

/// The U1's flow-calibration gating facts, read off the sliced G-code's
/// own footer: per-slot filament use in mm and the slots with nonzero
/// use. Indices are the G-code's filament order = **logical** slots
/// (`material − 1`) on the firmware-routed path. `FILAMENT_USED_MM` wants
/// exactly this (per-logical); the nonzero-use list is translated to
/// physical toolheads by [`physical_extruders_used`] before it becomes
/// `FLOW_CALIBRATE_EXTRUDERS`. Empty when the G-code carries no usage
/// lines — the firmware then keeps its persisted values.
pub(super) fn u1_usage_from_gcode(bytes: &[u8]) -> (Vec<u8>, Vec<f64>) {
    let summary = crate::core::slice::summary::build_summary_from_bytes(
        bytes,
        std::path::Path::new("send-buffer.gcode"),
    );
    let Some(max_index) = summary.filament_used_mm.keys().max().copied() else {
        return (Vec::new(), Vec::new());
    };
    let used_mm: Vec<f64> = (0..=max_index)
        .map(|i| summary.filament_used_mm.get(&i).copied().unwrap_or(0.0))
        .collect();
    let extruders_used: Vec<u8> = summary
        .filament_used_mm
        .iter()
        .filter(|(_, mm)| **mm > 0.0)
        .map(|(i, _)| *i)
        .collect();
    (extruders_used, used_mm)
}

/// Run the pre-send hook over `payload`, swapping in any plugin-edited
/// bytes. No-op when no plugin declares the hook; a panic in plugin Lua
/// is caught and the original bytes are sent unchanged.
pub(super) fn apply_pre_send(
    host: &PluginHostState,
    payload: SendPayload,
    plate_id: u32,
    kind: DriverKind,
    printer_model: Option<String>,
) -> SendPayload {
    let lock = || {
        host.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    };
    // Per-plate/project plugin activation doesn't apply to a whole-job
    // send, so the gate carries no per-level overrides — only the printer
    // model (for compatibility). The host still applies the global tier.
    let gate = DispatchGate {
        printer_model,
        ..Default::default()
    };
    if !lock().any_active_hook(HookKind::PreSend, &gate) {
        return payload;
    }

    let (payload_kind, bytes) = match &payload {
        SendPayload::Gcode { bytes, .. } => (PayloadKind::Gcode, bytes.clone()),
        // A `.gcode.3mf` bundle is an opaque zip; letting a text-editing
        // plugin (e.g. one written for U1 raw G-code) rewrite its bytes
        // would silently corrupt the archive. Skip pre-send for it for
        // now — editing the bundle is an advanced, opt-in concern.
        SendPayload::Gcode3mf { .. } => return payload,
    };
    let hook = PreSendHook {
        kind: payload_kind,
        target: SendTarget {
            driver_kind: match kind {
                DriverKind::Bambu => "bambu".to_string(),
                DriverKind::U1 => "u1".to_string(),
                DriverKind::Moonraker => "moonraker".to_string(),
            },
            plate_id,
        },
    };
    let edited = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        lock().dispatch_gated(&hook, bytes.clone(), &gate)
    })) {
        Ok(b) => b,
        Err(_) => {
            tracing::error!("pre-send plugin hook panicked; sending unmodified payload");
            bytes
        }
    };

    match payload {
        SendPayload::Gcode {
            file_name, u1_start, ..
        } => SendPayload::Gcode {
            bytes: edited,
            file_name,
            u1_start,
        },
        SendPayload::Gcode3mf {
            plate_id,
            file_basename,
            use_ams,
            ams_mapping,
            ams_mapping2,
            options,
            ..
        } => SendPayload::Gcode3mf {
            bytes: edited,
            plate_id,
            file_basename,
            use_ams,
            ams_mapping,
            ams_mapping2,
            options,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::project::Project;
    use std::sync::Arc;

    #[test]
    fn u1_map_table_pairs_each_material_with_its_bound_toolhead() {
        use crate::core::printer::SlotRef;
        let mut project = Project::default();
        let plate = &mut project.plates[0];
        plate.material_to_slot.insert(1, SlotRef { extruder: 2, slot: 0 });
        plate.material_to_slot.insert(2, SlotRef { extruder: 0, slot: 0 });
        let mtx = Mutex::new(Session::new(project));
        // logical = material − 1; physical = the bound toolhead.
        assert_eq!(u1_map_table(&mtx, 1), vec![(0, 2), (1, 0)]);
    }

    #[test]
    fn u1_map_table_falls_back_to_toolhead_0_for_unbound_gaps() {
        use crate::core::printer::SlotRef;
        let mut project = Project::default();
        // Only material 3 is bound; 1 and 2 are gaps (never emitted).
        project.plates[0]
            .material_to_slot
            .insert(3, SlotRef { extruder: 1, slot: 0 });
        let mtx = Mutex::new(Session::new(project));
        assert_eq!(u1_map_table(&mtx, 1), vec![(0, 0), (1, 0), (2, 1)]);
    }

    #[test]
    fn physical_extruders_used_translates_and_dedupes() {
        // logical 0 and 2 both map to physical toolhead 1 → deduped.
        let map = vec![(0u8, 1u8), (1, 0), (2, 1)];
        assert_eq!(physical_extruders_used(&[0, 2], &map), vec![1]);
        assert_eq!(physical_extruders_used(&[1, 0], &map), vec![0, 1]);
        // A logical slot with no map entry is dropped.
        assert_eq!(physical_extruders_used(&[5], &map), Vec::<u8>::new());
    }

    #[test]
    fn logical_nozzle_diameters_indexes_by_mapped_toolhead() {
        // logical 0 → toolhead 2 (0.6), logical 1 → toolhead 0 (0.4).
        let map = vec![(0u8, 2u8), (1, 0)];
        let physical = vec![0.4, 0.4, 0.6, 0.4];
        assert_eq!(logical_nozzle_diameters(&map, &physical), vec![0.6, 0.4]);
        // No physical data → empty (omit rather than send zeros).
        assert_eq!(logical_nozzle_diameters(&map, &[]), Vec::<f64>::new());
    }

    #[test]
    fn u1_usage_from_gcode_reads_footer_and_gates_extruders() {
        // libslic3r-style footer: per-filament usage, comma-separated.
        // Extruder 1 is unused (0.0) — it must appear in the mm array
        // (index-aligned) but not in the used-extruder list.
        let gcode = b"G1 X0\n; filament used [mm] = 500.00,0.00,600.60\n";
        let (used, mm) = u1_usage_from_gcode(gcode);
        assert_eq!(used, vec![0, 2]);
        assert_eq!(mm, vec![500.0, 0.0, 600.6]);

        // No usage lines at all → both empty, so the start script omits
        // the flow-gating params entirely.
        let (used, mm) = u1_usage_from_gcode(b"G1 X0\n");
        assert!(used.is_empty());
        assert!(mm.is_empty());
    }

    #[test]
    fn send_options_default_matches_legacy_hardcoded_behavior() {
        // An instance .toml written before send_options existed must
        // deserialize to the old hardcoded send behavior: leveling on,
        // calibrations + timelapse off.
        let options: crate::core::printer::instance::SendOptions =
            toml::from_str("").expect("empty table deserializes via defaults");
        assert!(options.bed_leveling);
        assert!(!options.flow_calibration);
        assert!(!options.vibration_calibration);
        assert!(!options.timelapse);
    }

    #[test]
    fn derive_send_names_combines_project_title_and_plate_name() {
        use std::path::PathBuf;

        // Unsaved, default-named plate → untitled_Plate 1.
        let project = Project::default();
        let (basename, title) =
            derive_send_names(&Mutex::new(Session::new(project.clone())), 1);
        assert_eq!(basename, "Untitled_Plate_1");
        assert_eq!(title, "Untitled — Plate 1");

        // Saved as MyPrint.3mf (source_path lives in SessionRuntime),
        // plate renamed "Lid".
        let mut project = project;
        if let Some(plate) = project.plates.first_mut() {
            plate.name = "Lid".into();
        }
        let mut session = Session::new(project);
        session.runtime.source_path = Some(PathBuf::from("/tmp/MyPrint.3mf"));
        let (basename, title) = derive_send_names(&Mutex::new(session.clone()), 1);
        assert_eq!(basename, "MyPrint_Lid");
        assert_eq!(title, "MyPrint — Lid");

        // Unknown plate id still produces a well-formed name from the project.
        let (basename, title) = derive_send_names(&Mutex::new(session), 9);
        assert_eq!(basename, "MyPrint_Plate_9");
        assert_eq!(title, "MyPrint — Plate 9");
    }

    fn host_with_pre_send(lua: &str) -> PluginHostState {
        use crate::core::plugin::PluginHost;
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("p");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("plugin.toml"),
            "name=\"p\"\nversion=\"1.0.0\"\nentry=\"main.lua\"\nhooks=[\"pre_send\"]\n\
             enabled_by_default=true\n",
        )
        .unwrap();
        std::fs::write(dir.join("main.lua"), lua).unwrap();
        // `load` reads the entry Lua into the runtime, so the temp dir
        // can drop right after.
        Arc::new(Mutex::new(PluginHost::load(&[tmp.path().to_path_buf()])))
    }

    #[test]
    fn apply_pre_send_skips_printer_incompatible_plugin() {
        use crate::core::plugin::PluginHost;
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("u1-only");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("plugin.toml"),
            "name=\"u1-only\"\nversion=\"1.0.0\"\nentry=\"main.lua\"\n\
             hooks=[\"pre_send\"]\nprinter_compatibility=[\"Snapmaker U1\"]\n\
             enabled_by_default=true\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("main.lua"),
            r#"function on_pre_send(p, t) return "CLOBBERED" end"#,
        )
        .unwrap();
        let host: PluginHostState =
            Arc::new(Mutex::new(PluginHost::load(&[tmp.path().to_path_buf()])));

        let mk = || SendPayload::Gcode {
            bytes: b"G1 X0".to_vec(),
            file_name: "p.gcode".into(),
            u1_start: None,
        };
        // Wrong printer model → the U1-only plugin is skipped (gate
        // enforces printer_compatibility), payload unchanged.
        match apply_pre_send(
            &host,
            mk(),
            1,
            DriverKind::U1,
            Some("Bambu Lab A1 mini".into()),
        ) {
            SendPayload::Gcode { bytes, .. } => {
                assert_eq!(bytes, b"G1 X0".to_vec(), "incompatible plugin skipped")
            }
            other => panic!("expected Gcode, got {other:?}"),
        }
        // Matching model → it runs and clobbers the bytes.
        match apply_pre_send(&host, mk(), 1, DriverKind::U1, Some("Snapmaker U1".into())) {
            SendPayload::Gcode { bytes, .. } => {
                assert_eq!(bytes, b"CLOBBERED".to_vec(), "compatible plugin ran")
            }
            other => panic!("expected Gcode, got {other:?}"),
        }
    }

    #[test]
    fn apply_pre_send_rewrites_gcode_and_preserves_fields() {
        let host = host_with_pre_send(
            r#"function on_pre_send(p, t) return p.bytes .. "\n; via " .. t.driver_kind end"#,
        );
        let payload = SendPayload::Gcode {
            bytes: b"G1 X0".to_vec(),
            file_name: "plate-7.gcode".into(),
            u1_start: None,
        };
        match apply_pre_send(&host, payload, 7, DriverKind::U1, None) {
            SendPayload::Gcode {
                bytes, file_name, ..
            } => {
                assert_eq!(bytes, b"G1 X0\n; via u1".to_vec());
                assert_eq!(file_name, "plate-7.gcode", "file_name preserved");
            }
            other => panic!("expected Gcode, got {other:?}"),
        }
    }

    #[test]
    fn apply_pre_send_skips_gcode_3mf_bundle() {
        // Even a clobbering plugin can't touch the opaque bundle.
        let host = host_with_pre_send(r#"function on_pre_send(p, t) return "CLOBBERED" end"#);
        let original = vec![0x50, 0x4b, 0x03, 0x04]; // "PK\x03\x04" zip header
        let payload = SendPayload::Gcode3mf {
            bytes: original.clone(),
            plate_id: 3,
            file_basename: "MyPrint_Lid".into(),
            use_ams: true,
            ams_mapping: vec![],
            ams_mapping2: vec![],
            options: Default::default(),
        };
        match apply_pre_send(&host, payload, 3, DriverKind::Bambu, None) {
            SendPayload::Gcode3mf {
                bytes,
                plate_id,
                file_basename,
                use_ams,
                ..
            } => {
                assert_eq!(bytes, original, ".gcode.3mf bytes must be untouched");
                assert_eq!(plate_id, 3);
                assert_eq!(file_basename, "MyPrint_Lid");
                assert!(use_ams);
            }
            other => panic!("expected Gcode3mf, got {other:?}"),
        }
    }
}
