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
//! different key set. Out of scope for MVP per the ticket — we
//! detect the flavor and error early in [`super::mod`] when present.

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
        _ => {}
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
        _ => {}
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
