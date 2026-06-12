//! `Metadata/slice_info.config` — Bambu's per-plate "this is a valid sliced
//! job" registration (print time, weight, per-filament usage, nozzle). The A1
//! mini's firmware uses it to recognize the upload as a sliced plate; without
//! it the printer treats the bundle as raw G-code and shows no preview, even
//! when the thumbnail PNG + cover relationships are present.
//!
//! We synthesize it from the G-code libslic3r already produced: the Bambu
//! HEADER_BLOCK (`total estimated time`), the CONFIG block
//! (`filament_colour` / `filament_type` / `nozzle_diameter` / `printer_model`),
//! and the footer (`; filament used [g]/[mm] =`). No separate slice summary is
//! threaded through the send path, so the G-code is the single source.

use super::sliced::SlicedProjectInput;

/// Client-version string for the `X-BBL-Client-Version` header. Bambu firmware
/// accepts any slicer client (OrcaSlicer's exports show previews fine); we use
/// a current Bambu-Studio-shaped value to stay on the conservative side.
const BBL_CLIENT_VERSION: &str = "02.07.01.57";

/// Parsed slice facts pulled out of one plate's G-code.
#[derive(Debug, Default, PartialEq)]
struct GcodeMeta {
    prediction_secs: u64,
    nozzle_diameter: String,
    printer_model: String,
    filament_colours: Vec<String>,
    filament_types: Vec<String>,
    used_g: Vec<f64>,
    used_mm: Vec<f64>,
}

/// Bambu's internal model id for the `printer_model_id` field, or `None` for an
/// unmapped model (the field is then omitted rather than emitting a wrong id).
fn printer_model_id(model: &str) -> Option<&'static str> {
    match model.trim() {
        "Bambu Lab A1 mini" => Some("N1"),
        "Bambu Lab A1" => Some("N2S"),
        _ => None,
    }
}

/// Parse "3h 30m 59s" (any subset, e.g. "30m 59s" or "45s") into seconds.
fn parse_hms(s: &str) -> u64 {
    let mut total = 0u64;
    let mut num = String::new();
    for ch in s.chars() {
        if ch.is_ascii_digit() {
            num.push(ch);
        } else if let Ok(n) = num.parse::<u64>() {
            match ch {
                'h' => total += n * 3600,
                'm' => total += n * 60,
                's' => total += n,
                _ => {}
            }
            num.clear();
        }
    }
    total
}

fn parse_floats(s: &str) -> Vec<f64> {
    s.split(',').filter_map(|t| t.trim().parse::<f64>().ok()).collect()
}

fn parse_gcode_meta(gcode: &[u8]) -> GcodeMeta {
    let text = String::from_utf8_lossy(gcode);
    let mut m = GcodeMeta::default();
    for raw in text.lines() {
        let line = raw.trim_end();
        if let Some(idx) = line.find("total estimated time:") {
            let rest = &line[idx + "total estimated time:".len()..];
            m.prediction_secs = parse_hms(rest);
        } else if let Some(v) = line.strip_prefix("; filament used [g] =") {
            m.used_g = parse_floats(v);
        } else if let Some(v) = line.strip_prefix("; filament used [mm] =") {
            m.used_mm = parse_floats(v);
        } else if let Some(v) = line.strip_prefix("; filament_colour =") {
            m.filament_colours = v.split(';').map(|s| s.trim().to_string()).collect();
        } else if let Some(v) = line.strip_prefix("; filament_type =") {
            m.filament_types = v.split(';').map(|s| s.trim().to_string()).collect();
        } else if let Some(v) = line.strip_prefix("; nozzle_diameter =") {
            // May be a comma list ("0.4,0.4"); the field wants a single value.
            m.nozzle_diameter = v.split(',').next().unwrap_or("").trim().to_string();
        } else if let Some(v) = line.strip_prefix("; printer_model =") {
            m.printer_model = v.trim().to_string();
        }
    }
    m
}

/// Build the `slice_info.config` XML for the whole bundle (one `<plate>` per
/// input plate, parsed from that plate's G-code).
pub fn slice_info_config_xml(input: &SlicedProjectInput) -> String {
    let mut out = String::with_capacity(1024);
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<config>\n");
    out.push_str("  <header>\n");
    out.push_str("    <header_item key=\"X-BBL-Client-Type\" value=\"slicer\"/>\n");
    out.push_str(&format!(
        "    <header_item key=\"X-BBL-Client-Version\" value=\"{BBL_CLIENT_VERSION}\"/>\n"
    ));
    out.push_str("  </header>\n");

    for plate in &input.plates {
        let n = plate.plate_id;
        let meta = parse_gcode_meta(&plate.gcode);
        let nozzle = if meta.nozzle_diameter.is_empty() {
            "0.4".to_string()
        } else {
            meta.nozzle_diameter.clone()
        };
        let total_weight: f64 = meta.used_g.iter().sum();

        out.push_str("  <plate>\n");
        out.push_str(&format!("    <metadata key=\"index\" value=\"{n}\"/>\n"));
        out.push_str(&format!(
            "    <metadata key=\"nozzle_diameters\" value=\"{nozzle}\"/>\n"
        ));
        if let Some(id) = printer_model_id(&meta.printer_model) {
            out.push_str(&format!(
                "    <metadata key=\"printer_model_id\" value=\"{id}\"/>\n"
            ));
        }
        out.push_str("    <metadata key=\"timelapse_type\" value=\"0\"/>\n");
        out.push_str(&format!(
            "    <metadata key=\"prediction\" value=\"{}\"/>\n",
            meta.prediction_secs
        ));
        out.push_str(&format!(
            "    <metadata key=\"weight\" value=\"{total_weight:.2}\"/>\n"
        ));
        out.push_str("    <metadata key=\"outside\" value=\"false\"/>\n");
        out.push_str("    <metadata key=\"support_used\" value=\"false\"/>\n");
        out.push_str("    <metadata key=\"label_object_enabled\" value=\"false\"/>\n");

        // One <filament> per used filament; colour/type index by position.
        for i in 0..meta.used_g.len() {
            let id = i + 1;
            let ty = meta.filament_types.get(i).map(String::as_str).unwrap_or("PLA");
            let colour = meta
                .filament_colours
                .get(i)
                .map(|c| c.to_uppercase())
                .unwrap_or_else(|| "#FFFFFF".to_string());
            let used_g = meta.used_g[i];
            let used_m = meta.used_mm.get(i).copied().unwrap_or(0.0) / 1000.0;
            out.push_str(&format!(
                "    <filament id=\"{id}\" type=\"{ty}\" color=\"{colour}\" \
                 used_m=\"{used_m:.2}\" used_g=\"{used_g:.2}\" \
                 nozzle_diameter=\"{nozzle}\"/>\n"
            ));
        }
        out.push_str("  </plate>\n");
    }
    out.push_str("</config>\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::slice::PlateSummary;
    use crate::core::threemf::sliced::SlicedPlate;

    const SAMPLE_GCODE: &str = "; HEADER_BLOCK_START\n\
; model printing time: 3h 24m 35s; total estimated time: 3h 30m 59s\n\
; total layer number: 110\n\
; filament: 1,2\n\
; HEADER_BLOCK_END\n\
; CONFIG_BLOCK_START\n\
; filament_colour = #ec984c;#ff6910\n\
; filament_type = PLA;PLA\n\
; nozzle_diameter = 0.4\n\
; printer_model = Bambu Lab A1 mini\n\
; CONFIG_BLOCK_END\n\
G28\n\
; filament used [mm] = 20833.44, 1484.29\n\
; filament used [g] = 66.15, 4.43\n";

    fn input_from(gcode: &str) -> SlicedProjectInput {
        SlicedProjectInput {
            printer_model: "Bambu Lab A1 mini".into(),
            file_metadata: std::collections::BTreeMap::new(),
            plates: vec![SlicedPlate {
                plate_id: 1,
                gcode: gcode.as_bytes().to_vec(),
                summary: PlateSummary::default(),
                thumbnail_png: None,
                ams_bindings: vec![],
            }],
        }
    }

    #[test]
    fn parses_time_filament_and_config() {
        let m = parse_gcode_meta(SAMPLE_GCODE.as_bytes());
        assert_eq!(m.prediction_secs, 3 * 3600 + 30 * 60 + 59);
        assert_eq!(m.used_g, vec![66.15, 4.43]);
        assert_eq!(m.used_mm, vec![20833.44, 1484.29]);
        assert_eq!(m.filament_colours, vec!["#ec984c", "#ff6910"]);
        assert_eq!(m.filament_types, vec!["PLA", "PLA"]);
        assert_eq!(m.nozzle_diameter, "0.4");
        assert_eq!(m.printer_model, "Bambu Lab A1 mini");
    }

    #[test]
    fn emits_plate_with_filaments_and_prediction() {
        let xml = slice_info_config_xml(&input_from(SAMPLE_GCODE));
        assert!(xml.contains("key=\"index\" value=\"1\""));
        assert!(xml.contains("key=\"prediction\" value=\"12659\""));
        assert!(xml.contains("key=\"weight\" value=\"70.58\"")); // 66.15 + 4.43
        assert!(xml.contains("key=\"printer_model_id\" value=\"N1\""));
        // Colour upper-cased, length mm→m, both filaments present.
        assert!(xml.contains(
            "<filament id=\"1\" type=\"PLA\" color=\"#EC984C\" used_m=\"20.83\" used_g=\"66.15\""
        ));
        assert!(xml.contains("id=\"2\" type=\"PLA\" color=\"#FF6910\" used_m=\"1.48\" used_g=\"4.43\""));
    }

    #[test]
    fn degenerate_gcode_yields_a_valid_empty_plate() {
        let xml = slice_info_config_xml(&input_from("G28\n"));
        assert!(xml.contains("<config>"));
        assert!(xml.contains("key=\"index\" value=\"1\""));
        assert!(xml.contains("key=\"prediction\" value=\"0\""));
        // No filament usage parsed → no <filament> rows, still valid XML.
        assert!(!xml.contains("<filament "));
        assert!(xml.contains("</config>"));
    }

    #[test]
    fn parse_hms_handles_subsets() {
        assert_eq!(parse_hms(" 3h 30m 59s"), 12659);
        assert_eq!(parse_hms(" 45s"), 45);
        assert_eq!(parse_hms(" 2m 5s"), 125);
    }
}
