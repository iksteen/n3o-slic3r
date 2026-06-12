//! Embed a print thumbnail in a raw `.gcode` stream as a base64 PNG
//! comment block — the PrusaSlicer/OrcaSlicer convention that Klipper's
//! Moonraker (and the Mainsail / Fluidd web UIs) parse to show a preview
//! while printing. The Snapmaker U1 send path prepends this; the Bambu
//! path instead drops the PNG into the `.gcode.3mf` (`Metadata/plate_N.png`),
//! which its firmware reads directly.
//!
//! Block shape (each data line is `; ` + up to 78 base64 chars):
//! ```text
//! ; thumbnail begin 300x300 12345
//! ; iVBORw0KGgo...
//! ; ...
//! ; thumbnail end
//! ```
//! The size field is the length of the base64 string, matching what the
//! upstream slicers emit and the web UIs expect.

use base64::Engine;

/// Max base64 characters per comment line (the line itself is `; ` + 78 = 80),
/// matching PrusaSlicer/OrcaSlicer's `GCodeThumbnails` line width.
const LINE_WIDTH: usize = 78;

/// Width/height from a PNG's IHDR header, or `None` if the bytes aren't a
/// PNG (wrong 8-byte signature or missing `IHDR`). Dimensions live at fixed
/// offsets: signature[0..8], then the IHDR chunk with width at 16 and height
/// at 20, both big-endian u32.
pub fn png_dimensions(png: &[u8]) -> Option<(u32, u32)> {
    const SIG: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];
    if png.len() < 24 || png[0..8] != SIG || &png[12..16] != b"IHDR" {
        return None;
    }
    let w = u32::from_be_bytes(png[16..20].try_into().ok()?);
    let h = u32::from_be_bytes(png[20..24].try_into().ok()?);
    Some((w, h))
}

/// Build the G-code thumbnail comment block for a PNG, or `None` if the
/// bytes aren't a readable PNG. The returned string ends with a newline so
/// it prepends cleanly onto the existing G-code.
pub fn gcode_thumbnail_block(png: &[u8]) -> Option<String> {
    let (w, h) = png_dimensions(png)?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(png);

    let mut out = String::with_capacity(b64.len() + b64.len() / LINE_WIDTH + 64);
    out.push_str(&format!("; thumbnail begin {w}x{h} {}\n", b64.len()));
    let bytes = b64.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let end = (i + LINE_WIDTH).min(bytes.len());
        out.push_str("; ");
        // base64 is ASCII, so slicing on byte boundaries is safe.
        out.push_str(std::str::from_utf8(&bytes[i..end]).unwrap());
        out.push('\n');
        i = end;
    }
    out.push_str("; thumbnail end\n");
    Some(out)
}

/// Prepend the thumbnail block to a G-code byte buffer. A no-op (returns the
/// input unchanged) when the PNG can't be parsed, so a bad render never
/// blocks a print.
pub fn prepend_thumbnail(gcode: Vec<u8>, png: &[u8]) -> Vec<u8> {
    match gcode_thumbnail_block(png) {
        Some(block) => {
            let mut out = Vec::with_capacity(block.len() + gcode.len());
            out.extend_from_slice(block.as_bytes());
            out.extend_from_slice(&gcode);
            out
        }
        None => gcode,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A byte buffer that looks like a PNG to `png_dimensions`: the 8-byte
    /// signature, an IHDR chunk header, then the big-endian width/height.
    /// Trailing `tail` bytes pad the "image data" so base64 has something to
    /// chew on.
    fn fake_png(w: u32, h: u32, tail: &[u8]) -> Vec<u8> {
        let mut v = vec![137, 80, 78, 71, 13, 10, 26, 10];
        v.extend_from_slice(&[0, 0, 0, 13]); // IHDR length
        v.extend_from_slice(b"IHDR");
        v.extend_from_slice(&w.to_be_bytes());
        v.extend_from_slice(&h.to_be_bytes());
        v.extend_from_slice(tail);
        v
    }

    #[test]
    fn reads_dimensions_from_the_ihdr() {
        assert_eq!(png_dimensions(&fake_png(300, 200, &[])), Some((300, 200)));
    }

    #[test]
    fn rejects_non_png_bytes() {
        assert_eq!(png_dimensions(b"not a png at all"), None);
        assert_eq!(png_dimensions(&[]), None);
    }

    #[test]
    fn block_has_the_prusaslicer_shape() {
        let png = fake_png(64, 64, &vec![0xABu8; 200]);
        let block = gcode_thumbnail_block(&png).expect("valid png");
        let lines: Vec<&str> = block.lines().collect();
        assert_eq!(lines[0], {
            let b64 = base64::engine::general_purpose::STANDARD.encode(&png);
            &format!("; thumbnail begin 64x64 {}", b64.len())
        });
        assert_eq!(*lines.last().unwrap(), "; thumbnail end");
        // Every data line is a comment and no wider than the cap.
        for l in &lines[1..lines.len() - 1] {
            assert!(l.starts_with("; "));
            assert!(l.len() <= 2 + LINE_WIDTH, "line too wide: {l:?}");
        }
        // The concatenated payload round-trips to the original base64.
        let payload: String = lines[1..lines.len() - 1]
            .iter()
            .map(|l| l.trim_start_matches("; "))
            .collect();
        assert_eq!(
            payload,
            base64::engine::general_purpose::STANDARD.encode(&png)
        );
    }

    #[test]
    fn prepend_is_a_noop_on_a_bad_png() {
        let gcode = b"G28\nG1 X0\n".to_vec();
        assert_eq!(prepend_thumbnail(gcode.clone(), b"garbage"), gcode);
    }

    #[test]
    fn prepend_puts_the_block_before_the_gcode() {
        let png = fake_png(32, 32, &vec![1u8; 50]);
        let out = prepend_thumbnail(b"G28\n".to_vec(), &png);
        let s = String::from_utf8(out).unwrap();
        assert!(s.starts_with("; thumbnail begin 32x32 "));
        assert!(s.ends_with("; thumbnail end\nG28\n"));
    }
}
