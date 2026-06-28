//! 3MF Core spec XML parser.
//!
//! Parses the `<model>` element documented at
//! <https://3mf.io/specification/> — namespace
//! `http://schemas.microsoft.com/3dmanufacturing/core/2015/02`.
//! Returns a [`ModelDoc`] describing one `.model` part of the
//! archive: its top-level `<metadata>` entries, its `<resources>`
//! (object id → mesh OR component list), and its `<build>` items.
//!
//! The Production Extension (namespace
//! `http://schemas.microsoft.com/3dmanufacturing/production/2015/06`,
//! commonly prefixed `p:`) lets a `<component>` point at a sibling
//! `.model` file via `p:path`. We surface that as an opaque string;
//! cross-file resolution is the caller's job in [`super::mod`].
//!
//! quick-xml is used in stream mode: meshes can be large (Bambu's
//! per-part .model for the 4-color Benchy ships 33k triangles
//! inline), and we don't want to materialize the whole DOM.

use std::path::PathBuf;

use quick_xml::events::{BytesStart, Event};
use quick_xml::reader::Reader;

use crate::core::scene::loaders::LoadError;

#[derive(Debug, Clone)]
pub struct ModelDoc {
    /// File-level `<metadata name="...">value</metadata>` entries.
    pub metadata: Vec<(String, String)>,
    /// Objects under `<resources>`, keyed by their `id` attribute.
    pub objects: std::collections::BTreeMap<u32, ObjectDef>,
    /// `<build><item .../></build>` entries, in document order.
    pub build_items: Vec<BuildItem>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)] // `object_type` surfaces the 3MF type attribute for future support-mesh filtering
pub struct ObjectDef {
    pub id: u32,
    /// `type` attribute: "model", "support", "other", "solidsupport".
    /// Only "model" objects participate in the visible build for MVP.
    pub object_type: String,
    pub body: ObjectBody,
}

#[derive(Debug, Clone)]
pub enum ObjectBody {
    /// Direct mesh data. Vertices flat XYZ, indices triples (CCW).
    Mesh {
        vertices: Vec<f32>,
        indices: Vec<u32>,
        /// BBS per-triangle `paint_color` (MMU color-painting) strings,
        /// one per triangle in `indices`-triple order. Empty when no
        /// triangle is painted; otherwise dense (length = triangle count,
        /// `""` for unpainted faces). Carried verbatim — libslic3r owns
        /// the encoding; we only round-trip it.
        paint_colors: Vec<String>,
    },
    /// Tree of <component> references. Each component points to
    /// some `objectid` — possibly in a sibling .model file via the
    /// Production Extension's `p:path` attribute.
    Components(Vec<Component>),
}

#[derive(Debug, Clone)]
pub struct Component {
    pub objectid: u32,
    /// Production-extension `p:path` if present, e.g.
    /// `"/3D/Objects/object_1.model"`. None = same .model file.
    pub path: Option<String>,
    /// 4x4 column-major transform matrix. Identity if the source
    /// element had no `transform` attribute.
    pub transform: [f32; 16],
}

#[derive(Debug, Clone)]
pub struct BuildItem {
    pub objectid: u32,
    pub path: Option<String>,
    pub transform: [f32; 16],
    pub printable: bool,
}

pub fn parse_model(bytes: &[u8], source: &std::path::Path) -> Result<ModelDoc, LoadError> {
    let mut reader = Reader::from_reader(bytes);
    reader.config_mut().trim_text(false);

    let mut metadata = Vec::new();
    let mut objects = std::collections::BTreeMap::new();
    let mut build_items = Vec::new();
    let mut buf = Vec::new();

    // Tag matching ignores namespace prefixes since BBS, Orca, and
    // PrusaSlicer all bind the core namespace under different
    // prefixes (default, sometimes "3mf:"). Production-extension
    // prefix is conventionally `p:` but not guaranteed; we match by
    // local name.
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => match local_name(e.name()) {
                b"metadata" => {
                    let name = attr_string(e, b"name").unwrap_or_default();
                    let raw = reader.read_text(e.name()).map_err(|err| LoadError::Parse {
                        path: source.into(),
                        message: format!("metadata: {err}"),
                    })?;
                    // quick-xml's `read_text` returns the raw escaped text;
                    // `decode()` handles the byte encoding (not entities), then
                    // unescape `&amp;` / `&lt;` / etc. so callers see the
                    // original. Lossy fallback on a malformed entity preserves
                    // the raw text rather than erroring.
                    let decoded = raw.decode().map_err(|err| LoadError::Parse {
                        path: source.into(),
                        message: format!("metadata: {err}"),
                    })?;
                    let value = quick_xml::escape::unescape(&decoded)
                        .map(|c| c.into_owned())
                        .unwrap_or_else(|_| decoded.into_owned());
                    metadata.push((name, value));
                }
                b"object" => {
                    let obj = parse_object(&mut reader, e, source)?;
                    objects.insert(obj.id, obj);
                }
                b"item" => {
                    let item = parse_build_item(e)?;
                    build_items.push(item);
                }
                _ => {}
            },
            Ok(Event::Empty(ref e)) if local_name(e.name()) == b"item" => {
                let item = parse_build_item(e)?;
                build_items.push(item);
            }
            Ok(Event::Eof) => break,
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

    Ok(ModelDoc {
        metadata,
        objects,
        build_items,
    })
}

fn parse_object(
    reader: &mut Reader<&[u8]>,
    start: &BytesStart,
    source: &std::path::Path,
) -> Result<ObjectDef, LoadError> {
    let id = attr_u32(start, b"id").ok_or_else(|| LoadError::Parse {
        path: source.into(),
        message: "<object> missing id".into(),
    })?;
    let object_type = attr_string(start, b"type").unwrap_or_else(|| "model".to_string());

    // The object body is either <mesh> or <components>. Whichever
    // comes first is the canonical shape; BBS/Orca never mix them
    // in practice.
    let mut body: Option<ObjectBody> = None;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => match local_name(e.name()) {
                b"mesh" => {
                    let (vertices, indices, paint_colors) = parse_mesh(reader, source)?;
                    body = Some(ObjectBody::Mesh {
                        vertices,
                        indices,
                        paint_colors,
                    });
                }
                b"components" => {
                    let components = parse_components(reader, source)?;
                    body = Some(ObjectBody::Components(components));
                }
                _ => {}
            },
            Ok(Event::End(ref e)) if local_name(e.name()) == b"object" => break,
            Ok(Event::Eof) => {
                return Err(LoadError::Parse {
                    path: source.into(),
                    message: format!("<object id={id}> unterminated"),
                });
            }
            Err(err) => {
                return Err(LoadError::Parse {
                    path: source.into(),
                    message: format!("xml in object {id}: {err}"),
                });
            }
            _ => {}
        }
        buf.clear();
    }

    Ok(ObjectDef {
        id,
        object_type,
        body: body.ok_or_else(|| LoadError::Parse {
            path: source.into(),
            message: format!("<object id={id}> has neither <mesh> nor <components>"),
        })?,
    })
}

fn parse_mesh(
    reader: &mut Reader<&[u8]>,
    source: &std::path::Path,
) -> Result<(Vec<f32>, Vec<u32>, Vec<String>), LoadError> {
    let mut vertices: Vec<f32> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    // One entry per triangle, in document order. BBS only writes
    // `paint_color` on painted triangles, so most are empty; we keep a
    // dense vector (aligned to triangle index) and drop it to empty if
    // nothing was painted.
    let mut paint_colors: Vec<String> = Vec::new();
    let mut any_paint = false;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Empty(ref e)) => match local_name(e.name()) {
                b"vertex" => {
                    let x = attr_f32(e, b"x").unwrap_or(0.0);
                    let y = attr_f32(e, b"y").unwrap_or(0.0);
                    let z = attr_f32(e, b"z").unwrap_or(0.0);
                    vertices.push(x);
                    vertices.push(y);
                    vertices.push(z);
                }
                b"triangle" => {
                    let v1 = attr_u32(e, b"v1").ok_or_else(|| LoadError::Parse {
                        path: source.into(),
                        message: "<triangle> missing v1".into(),
                    })?;
                    let v2 = attr_u32(e, b"v2").ok_or_else(|| LoadError::Parse {
                        path: source.into(),
                        message: "<triangle> missing v2".into(),
                    })?;
                    let v3 = attr_u32(e, b"v3").ok_or_else(|| LoadError::Parse {
                        path: source.into(),
                        message: "<triangle> missing v3".into(),
                    })?;
                    indices.push(v1);
                    indices.push(v2);
                    indices.push(v3);
                    // BBS MMU color-painting (also `slic3r:mmu_segmentation`
                    // in some exports). Opaque per-triangle string we hand
                    // straight back to libslic3r on write.
                    let paint = attr_string(e, b"paint_color")
                        .or_else(|| attr_string(e, b"slic3r:mmu_segmentation"))
                        .unwrap_or_default();
                    any_paint |= !paint.is_empty();
                    paint_colors.push(paint);
                }
                _ => {}
            },
            Ok(Event::End(ref e)) if local_name(e.name()) == b"mesh" => break,
            Ok(Event::Eof) => {
                return Err(LoadError::Parse {
                    path: source.into(),
                    message: "<mesh> unterminated".into(),
                });
            }
            Err(err) => {
                return Err(LoadError::Parse {
                    path: source.into(),
                    message: format!("xml in mesh: {err}"),
                });
            }
            _ => {}
        }
        buf.clear();
    }
    if !any_paint {
        paint_colors.clear();
    }
    Ok((vertices, indices, paint_colors))
}

fn parse_components(
    reader: &mut Reader<&[u8]>,
    source: &std::path::Path,
) -> Result<Vec<Component>, LoadError> {
    let mut comps = Vec::new();
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Empty(ref e)) | Ok(Event::Start(ref e))
                if local_name(e.name()) == b"component" =>
            {
                let objectid = attr_u32(e, b"objectid").ok_or_else(|| LoadError::Parse {
                    path: source.into(),
                    message: "<component> missing objectid".into(),
                })?;
                let path = attr_string(e, b"path").or_else(|| attr_string(e, b"p:path"));
                let transform = parse_transform_attr(e).unwrap_or_else(identity_matrix);
                comps.push(Component {
                    objectid,
                    path,
                    transform,
                });
            }
            Ok(Event::End(ref e)) if local_name(e.name()) == b"components" => break,
            Ok(Event::Eof) => {
                return Err(LoadError::Parse {
                    path: source.into(),
                    message: "<components> unterminated".into(),
                });
            }
            Err(err) => {
                return Err(LoadError::Parse {
                    path: source.into(),
                    message: format!("xml in components: {err}"),
                });
            }
            _ => {}
        }
        buf.clear();
    }
    Ok(comps)
}

fn parse_build_item(e: &BytesStart) -> Result<BuildItem, LoadError> {
    let objectid = attr_u32(e, b"objectid").ok_or_else(|| LoadError::Parse {
        path: PathBuf::new(),
        message: "<item> missing objectid".into(),
    })?;
    let path = attr_string(e, b"path").or_else(|| attr_string(e, b"p:path"));
    let transform = parse_transform_attr(e).unwrap_or_else(identity_matrix);
    let printable = attr_string(e, b"printable")
        .map(|v| v != "0" && !v.eq_ignore_ascii_case("false"))
        .unwrap_or(true);
    Ok(BuildItem {
        objectid,
        path,
        transform,
        printable,
    })
}

/// 3MF `transform` attribute: 12 floats space-separated, representing
/// a 4×3 row-truncated affine matrix in column-major form. Spec
/// columns: (a b c) (d e f) (g h i) (tx ty tz), producing the 4×4
/// matrix `[[a,b,c,0],[d,e,f,0],[g,h,i,0],[tx,ty,tz,1]]`. We pack
/// directly into glam's column-major [f32; 16] layout.
fn parse_transform_attr(e: &BytesStart) -> Option<[f32; 16]> {
    let raw = attr_string(e, b"transform")?;
    let nums: Vec<f32> = raw
        .split_whitespace()
        .map(|s| s.parse::<f32>().ok())
        .collect::<Option<_>>()?;
    if nums.len() != 12 {
        return None;
    }
    let [a, b, c, d, e_, f, g, h, i, tx, ty, tz]: [f32; 12] = nums.try_into().ok()?;
    Some([
        a, b, c, 0.0, //
        d, e_, f, 0.0, //
        g, h, i, 0.0, //
        tx, ty, tz, 1.0,
    ])
}

fn identity_matrix() -> [f32; 16] {
    [
        1.0, 0.0, 0.0, 0.0, //
        0.0, 1.0, 0.0, 0.0, //
        0.0, 0.0, 1.0, 0.0, //
        0.0, 0.0, 0.0, 1.0,
    ]
}

fn local_name<'a>(name: quick_xml::name::QName<'a>) -> &'a [u8] {
    let bytes = name.into_inner();
    match bytes.iter().position(|&c| c == b':') {
        Some(idx) => &bytes[idx + 1..],
        None => bytes,
    }
}

fn attr_string(e: &BytesStart, key: &[u8]) -> Option<String> {
    // BBS sometimes namespaces attributes (`p:UUID`, `p:path`); match
    // on local name to stay tolerant of namespace prefix choice.
    for attr in e.attributes().flatten() {
        let k = attr.key.as_ref();
        let local = match k.iter().position(|&c| c == b':') {
            Some(idx) => &k[idx + 1..],
            None => k,
        };
        if local == key || k == key {
            return Some(String::from_utf8_lossy(&attr.value).into_owned());
        }
    }
    None
}

fn attr_u32(e: &BytesStart, key: &[u8]) -> Option<u32> {
    attr_string(e, key)?.parse().ok()
}

fn attr_f32(e: &BytesStart, key: &[u8]) -> Option<f32> {
    attr_string(e, key)?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    const TRIANGLE_MODEL: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<model unit="millimeter" xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02">
  <metadata name="Title">test</metadata>
  <resources>
    <object id="1" type="model">
      <mesh>
        <vertices>
          <vertex x="0" y="0" z="0"/>
          <vertex x="1" y="0" z="0"/>
          <vertex x="0" y="1" z="0"/>
        </vertices>
        <triangles>
          <triangle v1="0" v2="1" v3="2"/>
        </triangles>
      </mesh>
    </object>
  </resources>
  <build>
    <item objectid="1" transform="1 0 0 0 1 0 0 0 1 5 6 7"/>
  </build>
</model>
"#;

    #[test]
    fn parses_single_triangle_object() {
        let doc = parse_model(TRIANGLE_MODEL.as_bytes(), Path::new("test.model")).expect("parse");
        assert_eq!(
            doc.metadata,
            vec![("Title".to_string(), "test".to_string())]
        );
        let obj = doc.objects.get(&1).expect("object 1");
        match &obj.body {
            ObjectBody::Mesh {
                vertices, indices, ..
            } => {
                assert_eq!(vertices, &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0]);
                assert_eq!(indices, &[0, 1, 2]);
            }
            ObjectBody::Components(_) => panic!("expected mesh body"),
        }
        assert_eq!(doc.build_items.len(), 1);
        let item = &doc.build_items[0];
        assert_eq!(item.objectid, 1);
        assert!(item.printable);
        // tx, ty, tz are the last three of the [f32;16] column-major matrix.
        assert_eq!(item.transform[12], 5.0);
        assert_eq!(item.transform[13], 6.0);
        assert_eq!(item.transform[14], 7.0);
    }

    const COMPONENT_MODEL: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<model xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02"
       xmlns:p="http://schemas.microsoft.com/3dmanufacturing/production/2015/06">
  <resources>
    <object id="9" type="model">
      <components>
        <component p:path="/3D/Objects/object_1.model" objectid="1" transform="1 0 0 0 1 0 0 0 1 10 0 0"/>
        <component p:path="/3D/Objects/object_1.model" objectid="2" transform="1 0 0 0 1 0 0 0 1 20 0 0"/>
      </components>
    </object>
  </resources>
  <build>
    <item objectid="9"/>
  </build>
</model>
"#;

    #[test]
    fn parses_component_references_with_paths() {
        let doc =
            parse_model(COMPONENT_MODEL.as_bytes(), Path::new("3dmodel.model")).expect("parse");
        let obj = doc.objects.get(&9).expect("object 9");
        let comps = match &obj.body {
            ObjectBody::Components(c) => c,
            _ => panic!("expected components body"),
        };
        assert_eq!(comps.len(), 2);
        assert_eq!(comps[0].objectid, 1);
        assert_eq!(comps[0].path.as_deref(), Some("/3D/Objects/object_1.model"));
        assert_eq!(comps[0].transform[12], 10.0);
        assert_eq!(comps[1].transform[12], 20.0);
        // Default printable=true when not specified.
        assert!(doc.build_items[0].printable);
    }
}
