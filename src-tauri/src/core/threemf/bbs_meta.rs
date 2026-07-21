//! BambuStudio / OrcaSlicer 3MF metadata extensions.
//!
//! BBS and Orca extend the standard 3MF container with two XML
//! sidecars under `Metadata/`:
//!
//! - `model_settings.config` — per-object + per-part metadata
//!   (`name`, `extruder`, per-part transform). The outer `<object>`
//!   id matches a `<resources><object id="N">` in `3dmodel.model`,
//!   and its `<part>` children correspond to that object's
//!   `<components>` entries in document order. Per-part `extruder`
//!   is the multi-material assignment that drives multi-tool slicing.
//!
//! - `project_settings.config` — the *settings* the file was authored
//!   against (printer profile name, cascade-side config, filament
//!   selections). For MVP we surface this as opaque informational
//!   text on the Project3mf — the cascade resolver does not consume
//!   it. Phase 5 (Settings UI) is where we'd parse this to suggest a
//!   matching cascade.
//!
//! PrusaSlicer-flavor metadata (`Slic3r_PE_model.config`) uses a
//! similar shape but with `volume` instead of `part` and a slightly
//! different key set. [`parse_prusa_object_names`] lifts its object
//! display names for the geometry import; its per-volume config and
//! source info aren't adopted.

use std::collections::BTreeMap;

use quick_xml::events::{BytesStart, Event};
use quick_xml::reader::Reader;

use crate::core::scene::loaders::LoadError;

#[derive(Debug, Default, Clone)]
pub struct ModelSettings {
    pub objects: Vec<ObjectSettings>,
    /// `<plate>` entries — multi-plate layouts. MVP cares about the
    /// first plate; later phases consume per-plate config (skirt,
    /// flush, ...).
    pub plates: Vec<PlateSettings>,
}

#[derive(Debug, Clone)]
pub struct ObjectSettings {
    pub id: u32,
    pub name: Option<String>,
    pub default_extruder: Option<u8>,
    /// Object-level libslic3r config metadata (every `<metadata>` key
    /// that isn't a recognized identity key like `name`/`extruder`).
    /// These are `ModelObject::config` deltas — per-object setting
    /// overrides. Applies to all of the object's parts. Kept raw; the
    /// scene-load layer scope-gates them into `scene.object_overrides`.
    pub config: BTreeMap<String, String>,
    /// Per-part settings, in document order. Part `id` matches the
    /// 1-based index of the corresponding `<component>` inside the
    /// referenced object — but we keep the id explicit since BBS
    /// numbers them and an off-by-one would be silent.
    pub parts: Vec<PartSettings>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)] // `id` and `source_object_id` surface diagnostic info we don't yet consume
pub struct PartSettings {
    pub id: u32,
    pub name: Option<String>,
    pub extruder: Option<u8>,
    /// Per-part (`ModelVolume::config`) libslic3r config metadata — the
    /// per-volume setting overrides for a multi-volume object. Same
    /// raw-keep-and-gate-later treatment as [`ObjectSettings::config`].
    pub config: BTreeMap<String, String>,
    /// `source_object_id` lets us map a part back to its source
    /// mesh — useful when one 3MF aggregates parts originally
    /// imported from separate files. Phase 2 doesn't consume this
    /// directly but we surface it for the eventual library / undo
    /// stack.
    pub source_object_id: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct PlateSettings {
    pub plater_id: u32,
    pub name: Option<String>,
    /// Object id assignments — each `<model_instance>` references an
    /// `<object id>` from 3dmodel.model. MVP places everything on
    /// the first plate; later phases use this to scatter objects
    /// across multiple plates.
    pub object_ids: Vec<u32>,
}

pub fn parse_model_settings(
    bytes: &[u8],
    source: &std::path::Path,
) -> Result<ModelSettings, LoadError> {
    let mut reader = Reader::from_reader(bytes);
    reader.config_mut().trim_text(true);

    let mut settings = ModelSettings::default();
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => match e.name().as_ref() {
                b"object" => {
                    let obj = parse_object(&mut reader, e, source)?;
                    settings.objects.push(obj);
                }
                b"plate" => {
                    let plate = parse_plate(&mut reader, source)?;
                    settings.plates.push(plate);
                }
                _ => {}
            },
            Ok(Event::Eof) => break,
            Err(err) => {
                return Err(LoadError::Parse {
                    path: source.into(),
                    message: format!("model_settings.config: {err}"),
                });
            }
            _ => {}
        }
        buf.clear();
    }

    Ok(settings)
}

fn parse_object(
    reader: &mut Reader<&[u8]>,
    start: &BytesStart,
    source: &std::path::Path,
) -> Result<ObjectSettings, LoadError> {
    let id = attr_u32(start, b"id").ok_or_else(|| LoadError::Parse {
        path: source.into(),
        message: "<object> missing id".into(),
    })?;

    let mut obj = ObjectSettings {
        id,
        name: None,
        default_extruder: None,
        config: BTreeMap::new(),
        parts: Vec::new(),
    };
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Empty(ref e)) if e.name().as_ref() == b"metadata" => {
                apply_object_metadata(e, &mut obj);
            }
            Ok(Event::Start(ref e)) => match e.name().as_ref() {
                b"metadata" => {
                    apply_object_metadata(e, &mut obj);
                    // Consume the matching End so the outer loop
                    // sees the next sibling.
                    drain_to_end(reader, b"metadata", source)?;
                }
                b"part" => {
                    let part = parse_part(reader, e, source)?;
                    obj.parts.push(part);
                }
                _ => {}
            },
            Ok(Event::End(ref e)) if e.name().as_ref() == b"object" => break,
            Ok(Event::Eof) => {
                return Err(LoadError::Parse {
                    path: source.into(),
                    message: format!("<object id={id}> in model_settings unterminated"),
                });
            }
            Err(err) => {
                return Err(LoadError::Parse {
                    path: source.into(),
                    message: format!("model_settings.config object {id}: {err}"),
                });
            }
            _ => {}
        }
        buf.clear();
    }

    Ok(obj)
}

fn apply_object_metadata(e: &BytesStart, obj: &mut ObjectSettings) {
    let Some(key) = attr_string(e, b"key") else {
        return;
    };
    let Some(value) = attr_string(e, b"value") else {
        return;
    };
    match key.as_str() {
        "name" => obj.name = Some(value),
        "extruder" => obj.default_extruder = value.parse().ok(),
        // Any other key is a libslic3r `ModelObject::config` override.
        _ => {
            obj.config.insert(key, value);
        }
    }
}

fn parse_part(
    reader: &mut Reader<&[u8]>,
    start: &BytesStart,
    source: &std::path::Path,
) -> Result<PartSettings, LoadError> {
    let id = attr_u32(start, b"id").ok_or_else(|| LoadError::Parse {
        path: source.into(),
        message: "<part> missing id".into(),
    })?;
    let mut part = PartSettings {
        id,
        name: None,
        extruder: None,
        config: BTreeMap::new(),
        source_object_id: None,
    };
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Empty(ref e)) if e.name().as_ref() == b"metadata" => {
                apply_part_metadata(e, &mut part);
            }
            Ok(Event::Start(ref e)) if e.name().as_ref() == b"metadata" => {
                apply_part_metadata(e, &mut part);
                drain_to_end(reader, b"metadata", source)?;
            }
            Ok(Event::End(ref e)) if e.name().as_ref() == b"part" => break,
            Ok(Event::Eof) => {
                return Err(LoadError::Parse {
                    path: source.into(),
                    message: format!("<part id={id}> unterminated"),
                });
            }
            Err(err) => {
                return Err(LoadError::Parse {
                    path: source.into(),
                    message: format!("model_settings.config part {id}: {err}"),
                });
            }
            _ => {}
        }
        buf.clear();
    }
    Ok(part)
}

fn apply_part_metadata(e: &BytesStart, part: &mut PartSettings) {
    let Some(key) = attr_string(e, b"key") else {
        return;
    };
    let Some(value) = attr_string(e, b"value") else {
        return;
    };
    match key.as_str() {
        "name" => part.name = Some(value),
        "extruder" => part.extruder = value.parse().ok(),
        "source_object_id" => part.source_object_id = value.parse().ok(),
        // Any other key is a libslic3r `ModelVolume::config` override.
        _ => {
            part.config.insert(key, value);
        }
    }
}

fn parse_plate(
    reader: &mut Reader<&[u8]>,
    source: &std::path::Path,
) -> Result<PlateSettings, LoadError> {
    let mut plate = PlateSettings {
        plater_id: 1,
        name: None,
        object_ids: Vec::new(),
    };
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Empty(ref e)) if e.name().as_ref() == b"metadata" => {
                apply_plate_metadata(e, &mut plate);
            }
            Ok(Event::Start(ref e)) => match e.name().as_ref() {
                b"metadata" => {
                    apply_plate_metadata(e, &mut plate);
                    drain_to_end(reader, b"metadata", source)?;
                }
                b"model_instance" => {
                    let object_id = scan_model_instance(reader, source)?;
                    if let Some(id) = object_id {
                        plate.object_ids.push(id);
                    }
                }
                _ => {}
            },
            Ok(Event::End(ref e)) if e.name().as_ref() == b"plate" => break,
            Ok(Event::Eof) => {
                return Err(LoadError::Parse {
                    path: source.into(),
                    message: "<plate> unterminated".into(),
                });
            }
            Err(err) => {
                return Err(LoadError::Parse {
                    path: source.into(),
                    message: format!("model_settings.config plate: {err}"),
                });
            }
            _ => {}
        }
        buf.clear();
    }
    Ok(plate)
}

fn apply_plate_metadata(e: &BytesStart, plate: &mut PlateSettings) {
    let Some(key) = attr_string(e, b"key") else {
        return;
    };
    let Some(value) = attr_string(e, b"value") else {
        return;
    };
    match key.as_str() {
        "plater_id" => {
            if let Ok(n) = value.parse() {
                plate.plater_id = n;
            }
        }
        "plater_name" => plate.name = Some(value),
        _ => {}
    }
}

fn scan_model_instance(
    reader: &mut Reader<&[u8]>,
    source: &std::path::Path,
) -> Result<Option<u32>, LoadError> {
    let mut object_id: Option<u32> = None;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Empty(ref e))
                if e.name().as_ref() == b"metadata"
                    && attr_string(e, b"key").as_deref() == Some("object_id") =>
            {
                object_id = attr_string(e, b"value").and_then(|s| s.parse().ok());
            }
            Ok(Event::Start(ref e)) if e.name().as_ref() == b"metadata" => {
                if attr_string(e, b"key").as_deref() == Some("object_id") {
                    object_id = attr_string(e, b"value").and_then(|s| s.parse().ok());
                }
                drain_to_end(reader, b"metadata", source)?;
            }
            Ok(Event::End(ref e)) if e.name().as_ref() == b"model_instance" => break,
            Ok(Event::Eof) => {
                return Err(LoadError::Parse {
                    path: source.into(),
                    message: "<model_instance> unterminated".into(),
                });
            }
            _ => {}
        }
        buf.clear();
    }
    Ok(object_id)
}

fn drain_to_end(
    reader: &mut Reader<&[u8]>,
    tag: &[u8],
    source: &std::path::Path,
) -> Result<(), LoadError> {
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::End(ref e)) if e.name().as_ref() == tag => return Ok(()),
            Ok(Event::Eof) => {
                return Err(LoadError::Parse {
                    path: source.into(),
                    message: format!("<{}> unterminated", String::from_utf8_lossy(tag)),
                });
            }
            Err(err) => {
                return Err(LoadError::Parse {
                    path: source.into(),
                    message: format!("xml: {err}"),
                });
            }
            _ => {}
        }
        buf.clear();
    }
}

/// PrusaSlicer / Slic3r PE `Slic3r_PE_model.config`: object display names keyed
/// by `<object id>`. Prusa's shape is `<object id="N"><metadata type="object"
/// key="name" value="…"/><volume …>…</volume></object>` — we lift only the
/// object-level name for the geometry import (its per-`volume` config + source
/// info are out of scope). `type="object"` filters out the sibling `<volume>`
/// name metadata that shares `key="name"`.
pub fn parse_prusa_object_names(
    bytes: &[u8],
    source: &std::path::Path,
) -> Result<BTreeMap<u32, String>, LoadError> {
    let mut reader = Reader::from_reader(bytes);
    reader.config_mut().trim_text(true);

    let mut names = BTreeMap::new();
    let mut current: Option<u32> = None;
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) if e.name().as_ref() == b"object" => {
                current = attr_u32(e, b"id");
            }
            Ok(Event::End(ref e)) if e.name().as_ref() == b"object" => {
                current = None;
            }
            Ok(Event::Empty(ref e)) | Ok(Event::Start(ref e))
                if e.name().as_ref() == b"metadata" =>
            {
                if let Some(id) = current {
                    if attr_string(e, b"type").as_deref() == Some("object")
                        && attr_string(e, b"key").as_deref() == Some("name")
                    {
                        if let Some(value) = attr_string(e, b"value") {
                            names.insert(id, value);
                        }
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(err) => {
                return Err(LoadError::Parse {
                    path: source.into(),
                    message: format!("Slic3r_PE_model.config: {err}"),
                });
            }
            _ => {}
        }
        buf.clear();
    }

    Ok(names)
}

/// Lift the "geometry-intent" subset of a PrusaSlicer / Slic3r PE print config
/// (`Metadata/Slic3r_PE.config`) — the keys that define how *this model* is
/// meant to print (shells, walls, infill, raft/brim, seam) — translated to
/// their OrcaSlicer names. Printer/filament/machine keys (bed shape, temps,
/// speeds, gcode) are deliberately NOT adopted: this is a model import, not a
/// profile adoption.
///
/// `Slic3r_PE.config` is Prusa's INI-style dump — each real line is
/// `; key = value`. Only the keys below are read. All are **object- or
/// region-scoped** in libslic3r, so they survive `gate_object_overrides` and
/// ride in as per-object overrides — a shell-only model (`top_solid_layers=0`)
/// then prints as designed without touching the user's own profile.
///
/// Values are carried verbatim: the FFI's override apply uses libslic3r's
/// forward-compatibility substitution + skips any value the deserializer
/// rejects, so a Prusa enum value with no Orca equivalent silently falls back
/// to the profile default rather than breaking the slice — no validation here.
///
/// Not carried: skirt (`skirts`/`skirt_distance`/`skirt_height`) and
/// `spiral_vase`. Those are **print-global** in OrcaSlicer (`PrintConfig`), not
/// object-overridable, so they can't ride this per-object channel.
pub fn parse_prusa_geometry_overrides(bytes: &[u8]) -> BTreeMap<String, String> {
    // PrusaSlicer key → OrcaSlicer key. Renames verified against
    // OrcaSlicer PrintConfig.cpp; every target is object/region-scoped.
    const MAP: &[(&str, &str)] = &[
        // Shells / walls / infill — the model's shape recipe.
        ("top_solid_layers", "top_shell_layers"),
        ("bottom_solid_layers", "bottom_shell_layers"),
        ("perimeters", "wall_loops"),
        ("fill_density", "sparse_infill_density"),
        ("fill_pattern", "sparse_infill_pattern"),
        ("top_fill_pattern", "top_surface_pattern"),
        ("bottom_fill_pattern", "bottom_surface_pattern"),
        ("fill_angle", "infill_direction"),
        ("infill_overlap", "infill_wall_overlap"),
        // Raft / brim (object-scoped) + seam.
        ("raft_layers", "raft_layers"),
        ("brim_width", "brim_width"),
        ("seam_position", "seam_position"),
    ];
    let text = String::from_utf8_lossy(bytes);
    let mut out = BTreeMap::new();
    for line in text.lines() {
        // Strip the leading `; ` comment marker Prusa writes on every line.
        let line = line.trim_start().trim_start_matches(';').trim();
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if let Some((_, orca)) = MAP.iter().find(|(prusa, _)| *prusa == key.trim()) {
            out.insert((*orca).to_owned(), value.trim().to_owned());
        }
    }
    // Prusa has no auto-brim: `brim_width = 0` means NO brim, `> 0` an outer
    // brim of that width. OrcaSlicer's `brim_type` defaults to `auto_brim`,
    // which generates (and auto-sizes) a brim regardless of `brim_width` — so
    // carry Prusa's intent explicitly, or `brim_width` alone is ignored.
    if let Some(width) = out.get("brim_width") {
        let has_brim = width.parse::<f64>().unwrap_or(0.0) > 0.0;
        out.insert(
            "brim_type".to_owned(),
            if has_brim { "outer_only" } else { "no_brim" }.to_owned(),
        );
    }
    out
}

fn attr_string(e: &BytesStart, key: &[u8]) -> Option<String> {
    for attr in e.attributes().flatten() {
        if attr.key.as_ref() == key {
            return Some(String::from_utf8_lossy(&attr.value).into_owned());
        }
    }
    None
}

fn attr_u32(e: &BytesStart, key: &[u8]) -> Option<u32> {
    attr_string(e, key)?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn prusa_geometry_overrides_translate_keys_and_brim_intent() {
        // brim_width=0 → no_brim (Orca's auto_brim would otherwise ignore the 0).
        let no_brim = parse_prusa_geometry_overrides(
            b"; perimeters = 6\n; fill_pattern = honeycomb\n; brim_width = 0\n; nozzle_diameter = 0.4\n",
        );
        assert_eq!(no_brim.get("wall_loops").map(String::as_str), Some("6"));
        assert_eq!(
            no_brim.get("sparse_infill_pattern").map(String::as_str),
            Some("honeycomb"),
        );
        assert_eq!(no_brim.get("brim_width").map(String::as_str), Some("0"));
        assert_eq!(no_brim.get("brim_type").map(String::as_str), Some("no_brim"));
        // Printer keys are not lifted.
        assert!(!no_brim.contains_key("nozzle_diameter"));

        // brim_width>0 → outer_only, so the explicit width is honored, not
        // auto-sized by auto_brim.
        let with_brim = parse_prusa_geometry_overrides(b"; brim_width = 8\n");
        assert_eq!(with_brim.get("brim_width").map(String::as_str), Some("8"));
        assert_eq!(
            with_brim.get("brim_type").map(String::as_str),
            Some("outer_only"),
        );
    }

    const FOURCOLOR_SAMPLE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<config>
  <object id="9">
    <metadata key="name" value="benchy"/>
    <metadata key="extruder" value="1"/>
    <part id="1">
      <metadata key="name" value="Object_1"/>
      <metadata key="extruder" value="1"/>
      <metadata key="source_object_id" value="0"/>
    </part>
    <part id="2">
      <metadata key="name" value="Object_2"/>
      <metadata key="extruder" value="2"/>
    </part>
  </object>
  <plate>
    <metadata key="plater_id" value="1"/>
    <model_instance>
      <metadata key="object_id" value="9"/>
    </model_instance>
  </plate>
</config>
"#;

    #[test]
    fn collects_object_and_part_level_config_overrides() {
        // Object-level config (ModelObject::config) and part-level config
        // (ModelVolume::config) land in their respective `config` maps,
        // separate from identity keys (name/extruder/source_object_id).
        let src = r#"<?xml version="1.0" encoding="UTF-8"?>
<config>
  <object id="3">
    <metadata key="name" value="thing"/>
    <metadata key="extruder" value="1"/>
    <metadata key="layer_height" value="0.3"/>
    <part id="1">
      <metadata key="name" value="vol"/>
      <metadata key="extruder" value="2"/>
      <metadata key="wall_loops" value="5"/>
    </part>
  </object>
</config>
"#;
        let settings = parse_model_settings(src.as_bytes(), Path::new("ms.config")).expect("parse");
        let obj = &settings.objects[0];
        assert_eq!(obj.name.as_deref(), Some("thing"));
        assert_eq!(obj.default_extruder, Some(1));
        assert_eq!(
            obj.config.get("layer_height").map(String::as_str),
            Some("0.3"),
            "object-level config key collected",
        );
        assert!(!obj.config.contains_key("name") && !obj.config.contains_key("extruder"));
        let part = &obj.parts[0];
        assert_eq!(
            part.config.get("wall_loops").map(String::as_str),
            Some("5"),
            "part-level config key collected",
        );
        assert!(!part.config.contains_key("source_object_id"));
    }

    #[test]
    fn parses_object_and_parts() {
        let settings = parse_model_settings(FOURCOLOR_SAMPLE.as_bytes(), Path::new("ms.config"))
            .expect("parse");
        assert_eq!(settings.objects.len(), 1);
        let obj = &settings.objects[0];
        assert_eq!(obj.id, 9);
        assert_eq!(obj.name.as_deref(), Some("benchy"));
        assert_eq!(obj.default_extruder, Some(1));
        assert_eq!(obj.parts.len(), 2);
        assert_eq!(obj.parts[0].extruder, Some(1));
        assert_eq!(obj.parts[1].extruder, Some(2));
        assert_eq!(obj.parts[0].source_object_id, Some(0));
    }

    #[test]
    fn parses_plate_assignments() {
        let settings = parse_model_settings(FOURCOLOR_SAMPLE.as_bytes(), Path::new("ms.config"))
            .expect("parse");
        assert_eq!(settings.plates.len(), 1);
        assert_eq!(settings.plates[0].plater_id, 1);
        assert_eq!(settings.plates[0].object_ids, vec![9]);
    }
}
