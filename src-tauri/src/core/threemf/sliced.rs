//! `.gcode.3mf` writer — Bambu sliced format (PR-3-10).
//!
//! The variant of 3MF the Bambu A1 mini accepts as send-format
//! (per PRD FR-MP-4b): a 3MF container with embedded G-code blob(s),
//! plate metadata, thumbnails, and BBS namespace extensions
//! identifying the printer + filament aggregates.
//!
//! Reader-side support for drag-drop preview of `.gcode.3mf` is
//! Phase 6 work; this module only writes. End-to-end validation
//! (does the A1 mini actually accept what we ship?) happens at
//! Phase 7a's first real print — the metadata catalog comes from
//! PR-0.5-3's spike inventory but only a real print proves it
//! correct.
//!
//! Scope cut allowed per Execution Plan §5: emit minimum-viable
//! `.gcode.3mf` (G-code body, plate JSON, basic Bambu metadata).
//! Thumbnails and AMS bindings are optional inputs — pass `None`
//! and the writer omits them. Phase 7a re-fills with real data.
//!
//! The container infrastructure is shared with PR-3-9's project
//! writer — same `zip` crate, same `Content_Types.xml` /
//! `_rels/.rels` preamble.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::core::slice::PlateSummary;

const N3O_VERSION: &str = env!("CARGO_PKG_VERSION");

/// What the caller hands to [`write_sliced_3mf`]. One entry per
/// sliced plate plus the project-wide context the bundle needs.
#[derive(Debug, Clone)]
pub struct SlicedProjectInput {
    pub plates: Vec<SlicedPlate>,
    /// Human-readable printer model name (e.g., "Bambu A1 mini").
    /// Surfaces in Bambu's namespace metadata so the printer's
    /// firmware can sanity-check the job against the connected
    /// device.
    pub printer_model: String,
    /// File-level metadata to emit on the main `3dmodel.model`
    /// `<metadata>` elements (Title, Designer, …). The reader
    /// won't expect these to be present, but Bambu Studio writes
    /// them on its sliced files so we follow suit.
    pub file_metadata: std::collections::BTreeMap<String, String>,
}

/// One plate's sliced output + accompanying metadata.
#[derive(Debug, Clone)]
pub struct SlicedPlate {
    /// 1-based plater id matching what `model_settings.config`
    /// would have referenced. Bambu's metadata file names follow
    /// `plate_<N>.*` per the same id.
    pub plate_id: u32,
    /// G-code body bytes. PR-3-2's orchestrator will read this
    /// from the output file libslic3r wrote; the test path
    /// synthesizes a small fixture. Embedded verbatim so a Phase 6
    /// reader can re-extract byte-equal.
    pub gcode: Vec<u8>,
    /// Per-plate summary (PR-3-3). Goes into the plate JSON so the
    /// printer's UI can render expected time / filament use without
    /// re-parsing the G-code.
    pub summary: PlateSummary,
    /// Optional PNG thumbnail. When `None` the writer omits the
    /// `plate_<N>.png` entry; Bambu Studio's UI shows a placeholder
    /// in that case. Phase 7a populates from libslic3r's render.
    pub thumbnail_png: Option<Vec<u8>>,
    /// Per-plate AMS bindings — `(model_material_index, ams_slot)`.
    /// Empty until Phase 7c wires filament-sync. The bundle still
    /// validates without bindings; Bambu's printer just won't
    /// auto-map slots.
    pub ams_bindings: Vec<AmsBinding>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AmsBinding {
    pub model_material_index: u8,
    pub ams_slot: u8,
}

/// Write the input to `output` as a valid `.gcode.3mf` container.
///
/// On any I/O / zip error the partial file is left in place — the
/// caller can decide whether to clean up. The writer does NOT
/// re-read the file to verify (the ticket's "validation: re-read
/// and byte-match" step lives in PR-3-12's exit smoke, which has
/// the full parser stack to do it cheaply).
pub fn write_sliced_3mf(input: &SlicedProjectInput, output: &Path) -> Result<(), std::io::Error> {
    let file = File::create(output)?;
    let mut zip = ZipWriter::new(file);
    let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

    write_entry(&mut zip, "[Content_Types].xml", content_types_xml().as_bytes(), opts)?;
    write_entry(&mut zip, "_rels/.rels", rels_xml().as_bytes(), opts)?;
    write_entry(
        &mut zip,
        "3D/3dmodel.model",
        model_xml(input).as_bytes(),
        opts,
    )?;
    for plate in &input.plates {
        let n = plate.plate_id;
        write_entry(
            &mut zip,
            &format!("Metadata/plate_{n}.gcode"),
            &plate.gcode,
            opts,
        )?;
        write_entry(
            &mut zip,
            &format!("Metadata/plate_{n}.gcode.md5"),
            gcode_md5_hex(&plate.gcode).as_bytes(),
            opts,
        )?;
        write_entry(
            &mut zip,
            &format!("Metadata/plate_{n}.json"),
            plate_json(plate).as_bytes(),
            opts,
        )?;
        if let Some(thumb) = &plate.thumbnail_png {
            write_entry(&mut zip, &format!("Metadata/plate_{n}.png"), thumb, opts)?;
        }
    }
    zip.finish()
        .map_err(|e| std::io::Error::other(format!("finalize zip: {e}")))?;
    Ok(())
}

fn write_entry(
    zip: &mut ZipWriter<File>,
    name: &str,
    body: &[u8],
    opts: SimpleFileOptions,
) -> Result<(), std::io::Error> {
    zip.start_file(name, opts)
        .map_err(|e| std::io::Error::other(format!("start_file {name}: {e}")))?;
    zip.write_all(body)?;
    Ok(())
}

/// What [`read_sliced_3mf`] returns for one plate inside a
/// `.gcode.3mf` container. The G-code body is the verbatim bytes
/// the writer embedded; metadata + thumbnail are surfaced
/// separately so the preview UI can render them without re-
/// parsing G-code.
#[derive(Debug, Clone)]
pub struct SlicedPlateRead {
    pub plate_id: u32,
    pub gcode: Vec<u8>,
    /// `None` if the file shipped without a `plate_<N>.json` or it
    /// failed to deserialize into [`SlicedPlateMetadata`] (older
    /// Bambu Studio versions used slightly different shapes; the
    /// reader tolerates absence rather than rejecting the file).
    pub metadata: Option<SlicedPlateMetadata>,
    /// `None` when the file omitted `plate_<N>.png`. Bambu Studio
    /// emits a 600×600 preview render; we don't validate.
    pub thumbnail_png: Option<Vec<u8>>,
}

/// What [`read_sliced_3mf`] returns at the container level.
#[derive(Debug, Clone)]
pub struct SlicedRead {
    /// One entry per plate found in the container, ordered by
    /// plate_id ascending. MVP preview only renders the first plate
    /// per PR-6-14; the rest are surfaced so a future multi-plate
    /// picker can choose.
    pub plates: Vec<SlicedPlateRead>,
}

/// Open a `.gcode.3mf` and pull out every `plate_<N>.gcode` entry
/// along with its sidecar metadata + thumbnail. Reader for PR-3-10's
/// writer (`write_sliced_3mf`). Consumed by PR-6-14's drag-drop
/// preview loader.
///
/// Errors only on I/O / zip-corruption / missing G-code body; a
/// missing JSON or PNG sidecar is tolerated (metadata=None /
/// thumbnail=None) so older Bambu Studio output and hand-rolled
/// `.gcode.3mf` files still work in the preview.
pub fn read_sliced_3mf(path: &Path) -> Result<SlicedRead, std::io::Error> {
    let file = File::open(path)?;
    let mut zip = ZipArchive::new(file)
        .map_err(|e| std::io::Error::other(format!("open zip {}: {e}", path.display())))?;

    // Discover plates by scanning for `Metadata/plate_<N>.gcode`.
    let mut plate_ids: Vec<u32> = Vec::new();
    for name in zip.file_names() {
        if let Some(rest) = name.strip_prefix("Metadata/plate_") {
            if let Some(num_str) = rest.strip_suffix(".gcode") {
                // Avoid matching `plate_1.gcode.md5` etc.
                if let Ok(n) = num_str.parse::<u32>() {
                    plate_ids.push(n);
                }
            }
        }
    }
    plate_ids.sort_unstable();
    plate_ids.dedup();

    let mut plates: Vec<SlicedPlateRead> = Vec::with_capacity(plate_ids.len());
    for plate_id in plate_ids {
        let gcode = read_entry_bytes(&mut zip, &format!("Metadata/plate_{plate_id}.gcode"))?
            .ok_or_else(|| {
                std::io::Error::other(format!("missing Metadata/plate_{plate_id}.gcode body"))
            })?;
        let metadata = match read_entry_bytes(&mut zip, &format!("Metadata/plate_{plate_id}.json"))?
        {
            Some(bytes) => serde_json::from_slice::<SlicedPlateMetadata>(&bytes).ok(),
            None => None,
        };
        let thumbnail_png =
            read_entry_bytes(&mut zip, &format!("Metadata/plate_{plate_id}.png"))?;
        plates.push(SlicedPlateRead {
            plate_id,
            gcode,
            metadata,
            thumbnail_png,
        });
    }

    Ok(SlicedRead { plates })
}

fn read_entry_bytes(
    zip: &mut ZipArchive<File>,
    name: &str,
) -> Result<Option<Vec<u8>>, std::io::Error> {
    match zip.by_name(name) {
        Ok(mut entry) => {
            let mut buf = Vec::with_capacity(entry.size() as usize);
            entry.read_to_end(&mut buf)?;
            Ok(Some(buf))
        }
        Err(zip::result::ZipError::FileNotFound) => Ok(None),
        Err(e) => Err(std::io::Error::other(format!(
            "read zip entry {name}: {e}"
        ))),
    }
}

fn content_types_xml() -> String {
    // Bambu's sliced 3MF declares the same content types as a
    // project 3MF, plus an explicit `.gcode` mapping so the
    // firmware can detect the body. `.md5` and `.json` follow.
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
 <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
 <Default Extension="model" ContentType="application/vnd.ms-package.3dmanufacturing-3dmodel+xml"/>
 <Default Extension="gcode" ContentType="text/x.gcode"/>
 <Default Extension="md5" ContentType="text/plain"/>
 <Default Extension="json" ContentType="application/json"/>
 <Default Extension="png" ContentType="image/png"/>
</Types>
"#
    .into()
}

fn rels_xml() -> String {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
 <Relationship Target="/3D/3dmodel.model" Id="rel-1" Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel"/>
</Relationships>
"#
    .into()
}

fn model_xml(input: &SlicedProjectInput) -> String {
    let mut out = String::with_capacity(2048);
    out.push_str(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <model unit=\"millimeter\" xml:lang=\"en-US\" \
         xmlns=\"http://schemas.microsoft.com/3dmanufacturing/core/2015/02\" \
         xmlns:BambuStudio=\"http://schemas.bambulab.com/package/2021\">\n",
    );
    // Bambu's required version + identifying metadata.
    out.push_str(" <metadata name=\"BambuStudio:3mfVersion\">1</metadata>\n");
    out.push_str(&format!(
        " <metadata name=\"Application\">n3o-slic3r-{N3O_VERSION}</metadata>\n"
    ));
    out.push_str(&format!(
        " <metadata name=\"BambuStudio:PrinterModel\">{}</metadata>\n",
        xml_escape_text(&input.printer_model),
    ));
    for (k, v) in &input.file_metadata {
        if k == "Application" || k.starts_with("BambuStudio") {
            continue;
        }
        out.push_str(&format!(
            " <metadata name=\"{}\">{}</metadata>\n",
            xml_escape_attr(k),
            xml_escape_text(v),
        ));
    }

    // Sliced bundles still carry a `<resources>` + `<build>` per
    // 3MF Core, but with empty meshes — the firmware only needs
    // the per-plate gcode + metadata. We emit a single tiny
    // placeholder object so the spec validator is happy.
    out.push_str(
        " <resources>\n  <object id=\"1\" type=\"model\">\n   <mesh>\n    <vertices/>\n    <triangles/>\n   </mesh>\n  </object>\n </resources>\n",
    );
    out.push_str(" <build>\n");
    for plate in &input.plates {
        out.push_str(&format!(
            "  <item objectid=\"1\" transform=\"1 0 0 0 1 0 0 0 1 0 0 0\" printable=\"1\" BambuStudio:plate_id=\"{}\"/>\n",
            plate.plate_id,
        ));
    }
    out.push_str(" </build>\n");

    out.push_str("</model>\n");
    out
}

/// Bambu's per-plate `plate_<N>.json` shape — print time + filament
/// aggregates + bbox + AMS bindings + a few flags the firmware
/// surfaces in the UI. JSON shape mirrors what PR-0.5-3 observed
/// in real Bambu Studio output. Reader side ([`read_sliced_3mf`])
/// deserializes the same shape so the preview UI can show
/// estimated time + AMS bindings on `.gcode.3mf` drops without
/// re-deriving them from the parsed G-code.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlicedPlateMetadata {
    pub plate_index: u32,
    pub layer_count: u32,
    pub object_count: u32,
    pub estimated_time_seconds: u64,
    pub estimated_time_text: String,
    pub filament_used_grams: BTreeMap<u8, f64>,
    pub filament_used_mm: BTreeMap<u8, f64>,
    pub bbox_min: Option<[f32; 3]>,
    pub bbox_max: Option<[f32; 3]>,
    pub ams_bindings: Vec<AmsBinding>,
    /// Identifier of the writer; useful for diagnostics if a future
    /// reader version needs to detect older format quirks.
    pub emitter: String,
}

fn plate_json(plate: &SlicedPlate) -> String {
    let payload = SlicedPlateMetadata {
        plate_index: plate.plate_id,
        layer_count: plate.summary.layer_count,
        object_count: plate.summary.object_count,
        estimated_time_seconds: plate.summary.estimated_time_seconds,
        estimated_time_text: plate.summary.estimated_time_text.clone(),
        filament_used_grams: plate.summary.filament_used_grams.clone(),
        filament_used_mm: plate.summary.filament_used_mm.clone(),
        bbox_min: plate.summary.bbox_min,
        bbox_max: plate.summary.bbox_max,
        ams_bindings: plate.ams_bindings.clone(),
        emitter: format!("n3o-slic3r-{N3O_VERSION}"),
    };
    serde_json::to_string_pretty(&payload).expect("SlicedPlateMetadata serializes")
}

/// Bambu's plate G-code MD5 is the lowercase hex digest of the
/// raw G-code body. We compute it ourselves rather than pulling
/// in an `md5` crate dep — the algorithm is small and self-
/// contained, and we'd rather not add a transitive dep that
/// duplicates `sha2` (already present via the workspace).
///
/// MD5 lives here because it's a Bambu-firmware-side checksum
/// format; the rest of the project doesn't need it.
/// Re-exported as `core::threemf::md5_hex` for PR-7a-5's
/// FTPS upload (Bambu's MQTT `project_file` command has an
/// `md5` integrity field).
pub fn md5_hex(bytes: &[u8]) -> String {
    gcode_md5_hex(bytes)
}

fn gcode_md5_hex(bytes: &[u8]) -> String {
    let digest = md5_compute(bytes);
    let mut out = String::with_capacity(32);
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

// Self-contained MD5 implementation (RFC 1321). Used ONLY to
// produce the Bambu `plate_<N>.gcode.md5` digest. Not exposed
// elsewhere — if a future caller needs cryptographic hashing
// they should pull in `sha2` / `blake3`. MD5 is broken for
// security but Bambu firmware demands it for the plate
// checksum, so we comply.
fn md5_compute(bytes: &[u8]) -> [u8; 16] {
    let s = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5,
        9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6,
        10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    let k: [u32; 64] = [
        0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613,
        0xfd469501, 0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be, 0x6b901122, 0xfd987193,
        0xa679438e, 0x49b40821, 0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa, 0xd62f105d,
        0x02441453, 0xd8a1e681, 0xe7d3fbc8, 0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed,
        0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a, 0xfffa3942, 0x8771f681, 0x6d9d6122,
        0xfde5380c, 0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70, 0x289b7ec6, 0xeaa127fa,
        0xd4ef3085, 0x04881d05, 0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665, 0xf4292244,
        0x432aff97, 0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1,
        0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1, 0xf7537e82, 0xbd3af235, 0x2ad7d2bb,
        0xeb86d391,
    ];

    let bit_len = (bytes.len() as u64).wrapping_mul(8);
    let mut padded = bytes.to_vec();
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_le_bytes());

    let mut a0: u32 = 0x67452301;
    let mut b0: u32 = 0xefcdab89;
    let mut c0: u32 = 0x98badcfe;
    let mut d0: u32 = 0x10325476;

    for chunk in padded.chunks_exact(64) {
        let mut m = [0u32; 16];
        for (i, word) in chunk.chunks_exact(4).enumerate() {
            m[i] = u32::from_le_bytes([word[0], word[1], word[2], word[3]]);
        }
        let (mut a, mut b, mut c, mut d) = (a0, b0, c0, d0);
        for i in 0..64 {
            let (f, g) = match i {
                0..=15 => ((b & c) | (!b & d), i),
                16..=31 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                32..=47 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let temp = d;
            d = c;
            c = b;
            b = b.wrapping_add(
                a.wrapping_add(f)
                    .wrapping_add(k[i])
                    .wrapping_add(m[g])
                    .rotate_left(s[i] as u32),
            );
            a = temp;
        }
        a0 = a0.wrapping_add(a);
        b0 = b0.wrapping_add(b);
        c0 = c0.wrapping_add(c);
        d0 = d0.wrapping_add(d);
    }
    let mut out = [0u8; 16];
    out[0..4].copy_from_slice(&a0.to_le_bytes());
    out[4..8].copy_from_slice(&b0.to_le_bytes());
    out[8..12].copy_from_slice(&c0.to_le_bytes());
    out[12..16].copy_from_slice(&d0.to_le_bytes());
    out
}

fn xml_escape_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(c),
        }
    }
    out
}

fn xml_escape_attr(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

/// Test-only helper: produce a `SlicedProjectInput` with the given
/// G-code bytes on a single plate. Exposed so the integration
/// smoke can build one without re-creating all the boilerplate.
pub fn fixture_input(plate_id: u32, gcode: Vec<u8>) -> SlicedProjectInput {
    SlicedProjectInput {
        printer_model: "Bambu A1 mini".into(),
        file_metadata: std::collections::BTreeMap::new(),
        plates: vec![SlicedPlate {
            plate_id,
            gcode,
            summary: PlateSummary::default(),
            thumbnail_png: None,
            ams_bindings: vec![],
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use zip::ZipArchive;

    use std::path::PathBuf;

    fn tempfile_path() -> PathBuf {
        std::env::temp_dir().join(format!(
            "n3o-test-sliced-{}.gcode.3mf",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ))
    }

    fn unzip_entry(path: &Path, name: &str) -> Vec<u8> {
        let file = File::open(path).unwrap();
        let mut zip = ZipArchive::new(file).unwrap();
        let mut entry = zip.by_name(name).expect("entry exists");
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf).unwrap();
        buf
    }

    fn entry_names(path: &Path) -> Vec<String> {
        let file = File::open(path).unwrap();
        let zip = ZipArchive::new(file).unwrap();
        zip.file_names().map(|s| s.to_owned()).collect()
    }

    #[test]
    fn read_sliced_round_trips_gcode_metadata_and_thumbnail() {
        let mut summary = PlateSummary::default();
        summary.layer_count = 7;
        summary.estimated_time_seconds = 123;
        summary.estimated_time_text = "2m 3s".into();
        summary.filament_used_grams.insert(0, 1.5);
        let original_gcode = b";test\nG28\nG1 X1\n".to_vec();
        let thumb = vec![0x89, 0x50, 0x4E, 0x47];
        let input = SlicedProjectInput {
            printer_model: "Bambu A1 mini".into(),
            file_metadata: BTreeMap::new(),
            plates: vec![SlicedPlate {
                plate_id: 1,
                gcode: original_gcode.clone(),
                summary,
                thumbnail_png: Some(thumb.clone()),
                ams_bindings: vec![AmsBinding {
                    model_material_index: 0,
                    ams_slot: 2,
                }],
            }],
        };
        let path = tempfile_path();
        write_sliced_3mf(&input, &path).expect("write");

        let read = read_sliced_3mf(&path).expect("read");
        assert_eq!(read.plates.len(), 1);
        let plate = &read.plates[0];
        assert_eq!(plate.plate_id, 1);
        assert_eq!(plate.gcode, original_gcode);
        assert_eq!(plate.thumbnail_png.as_deref(), Some(thumb.as_slice()));
        let meta = plate.metadata.as_ref().expect("metadata");
        assert_eq!(meta.layer_count, 7);
        assert_eq!(meta.estimated_time_text, "2m 3s");
        assert_eq!(meta.ams_bindings.len(), 1);
        assert_eq!(meta.ams_bindings[0].ams_slot, 2);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn read_sliced_orders_plates_ascending() {
        let mut input = fixture_input(2, b"G28 ; plate 2\n".to_vec());
        input.plates.push(SlicedPlate {
            plate_id: 1,
            gcode: b"G28 ; plate 1\n".to_vec(),
            summary: PlateSummary::default(),
            thumbnail_png: None,
            ams_bindings: vec![],
        });
        let path = tempfile_path();
        write_sliced_3mf(&input, &path).expect("write");

        let read = read_sliced_3mf(&path).expect("read");
        let ids: Vec<u32> = read.plates.iter().map(|p| p.plate_id).collect();
        assert_eq!(ids, vec![1, 2]);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn read_sliced_tolerates_missing_json_and_thumbnail() {
        // Hand-roll a minimal .gcode.3mf without sidecars.
        let path = tempfile_path();
        {
            let f = File::create(&path).unwrap();
            let mut zip = ZipWriter::new(f);
            let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
            zip.start_file("Metadata/plate_1.gcode", opts).unwrap();
            zip.write_all(b"G28 ; bare\n").unwrap();
            zip.finish().unwrap();
        }
        let read = read_sliced_3mf(&path).expect("tolerant read");
        assert_eq!(read.plates.len(), 1);
        assert!(read.plates[0].metadata.is_none());
        assert!(read.plates[0].thumbnail_png.is_none());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn writes_expected_zip_layout_for_single_plate() {
        let input = fixture_input(1, b"G28\nG1 X10\n".to_vec());
        let path = tempfile_path();
        write_sliced_3mf(&input, &path).expect("write");

        let names = entry_names(&path);
        for required in [
            "[Content_Types].xml",
            "_rels/.rels",
            "3D/3dmodel.model",
            "Metadata/plate_1.gcode",
            "Metadata/plate_1.gcode.md5",
            "Metadata/plate_1.json",
        ] {
            assert!(
                names.iter().any(|n| n == required),
                "expected zip entry {required:?} (got {names:?})"
            );
        }
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn gcode_body_round_trips_byte_for_byte() {
        let original = b"; estimated printing time = 30s\nG28\nG1 X10\n".to_vec();
        let input = fixture_input(1, original.clone());
        let path = tempfile_path();
        write_sliced_3mf(&input, &path).expect("write");

        let extracted = unzip_entry(&path, "Metadata/plate_1.gcode");
        assert_eq!(extracted, original, "embedded gcode must round-trip");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn md5_matches_known_digest() {
        // Reference vector from RFC 1321 Appendix A.5.
        let digest = md5_compute(b"abc");
        let mut hex = String::new();
        for b in digest {
            use std::fmt::Write;
            let _ = write!(hex, "{b:02x}");
        }
        assert_eq!(hex, "900150983cd24fb0d6963f7d28e17f72");

        // Empty input.
        let digest = md5_compute(b"");
        let mut hex = String::new();
        for b in digest {
            use std::fmt::Write;
            let _ = write!(hex, "{b:02x}");
        }
        assert_eq!(hex, "d41d8cd98f00b204e9800998ecf8427e");
    }

    #[test]
    fn plate_md5_file_contains_hex_digest_of_gcode() {
        let body = b"hello gcode";
        let input = fixture_input(1, body.to_vec());
        let path = tempfile_path();
        write_sliced_3mf(&input, &path).expect("write");

        let md5_bytes = unzip_entry(&path, "Metadata/plate_1.gcode.md5");
        let md5_str = std::str::from_utf8(&md5_bytes).unwrap();
        let expected = {
            let d = md5_compute(body);
            let mut h = String::new();
            for b in d {
                use std::fmt::Write;
                let _ = write!(h, "{b:02x}");
            }
            h
        };
        assert_eq!(md5_str, expected);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn plate_json_carries_summary_fields() {
        let mut summary = PlateSummary::default();
        summary.estimated_time_seconds = 1234;
        summary.estimated_time_text = "20m 34s".into();
        summary.layer_count = 42;
        summary.filament_used_grams.insert(0, 4.21);
        let input = SlicedProjectInput {
            printer_model: "Bambu A1 mini".into(),
            file_metadata: std::collections::BTreeMap::new(),
            plates: vec![SlicedPlate {
                plate_id: 1,
                gcode: b"G28\n".to_vec(),
                summary,
                thumbnail_png: None,
                ams_bindings: vec![AmsBinding {
                    model_material_index: 0,
                    ams_slot: 2,
                }],
            }],
        };
        let path = tempfile_path();
        write_sliced_3mf(&input, &path).expect("write");

        let json_bytes = unzip_entry(&path, "Metadata/plate_1.json");
        let json = serde_json::from_slice::<serde_json::Value>(&json_bytes).unwrap();
        assert_eq!(json["estimated_time_seconds"], 1234);
        assert_eq!(json["estimated_time_text"], "20m 34s");
        assert_eq!(json["layer_count"], 42);
        assert_eq!(json["filament_used_grams"]["0"], 4.21);
        assert_eq!(json["ams_bindings"][0]["ams_slot"], 2);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn thumbnail_when_present_is_embedded() {
        let mut input = fixture_input(1, b"G28\n".to_vec());
        // Minimal 1×1 PNG (canonical 67-byte form).
        input.plates[0].thumbnail_png = Some(vec![
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, // PNG sig
            0x00, 0x00, 0x00, 0x0D, // IHDR len
            b'I', b'H', b'D', b'R', 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06,
            0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4, 0x89,
        ]);
        let path = tempfile_path();
        write_sliced_3mf(&input, &path).expect("write");
        let names = entry_names(&path);
        assert!(names.iter().any(|n| n == "Metadata/plate_1.png"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn multi_plate_writes_per_plate_files() {
        let input = SlicedProjectInput {
            printer_model: "Bambu A1 mini".into(),
            file_metadata: std::collections::BTreeMap::new(),
            plates: vec![
                SlicedPlate {
                    plate_id: 1,
                    gcode: b"plate1".to_vec(),
                    summary: PlateSummary::default(),
                    thumbnail_png: None,
                    ams_bindings: vec![],
                },
                SlicedPlate {
                    plate_id: 2,
                    gcode: b"plate2".to_vec(),
                    summary: PlateSummary::default(),
                    thumbnail_png: None,
                    ams_bindings: vec![],
                },
            ],
        };
        let path = tempfile_path();
        write_sliced_3mf(&input, &path).expect("write");
        let names = entry_names(&path);
        for required in [
            "Metadata/plate_1.gcode",
            "Metadata/plate_1.json",
            "Metadata/plate_2.gcode",
            "Metadata/plate_2.json",
        ] {
            assert!(names.iter().any(|n| n == required));
        }
        let _ = std::fs::remove_file(&path);
    }
}
