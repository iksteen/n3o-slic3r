//! Pure string helpers for project/plate naming. No state — kept out of
//! [`super::model`] so the persisted type definitions stay focused.

/// Filename-safe basename for sliced output: keep `[A-Za-z0-9._-]`, map any other
/// char (spaces, slashes, …) to `_`, collapse `_` runs, trim leading/trailing
/// separators, and fall back to "untitled" if nothing usable remains.
pub fn sanitize_basename(s: &str) -> String {
    let mapped: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let mut out = String::with_capacity(mapped.len());
    let mut prev_us = false;
    for c in mapped.chars() {
        if c == '_' {
            if !prev_us {
                out.push('_');
            }
            prev_us = true;
        } else {
            out.push(c);
            prev_us = false;
        }
    }
    let trimmed = out.trim_matches(|c| c == '_' || c == '.' || c == '-');
    if trimmed.is_empty() {
        "untitled".to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// Default name for a plate at the given 1-based position
/// — "Plate 1", "Plate 2", …. Users can rename in the UI.
pub fn plate_default_name(position: u32) -> String {
    format!("Plate {position}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_basename_keeps_safe_chars_collapses_runs_trims() {
        assert_eq!(sanitize_basename("My Print"), "My_Print");
        assert_eq!(sanitize_basename("a/b\\c"), "a_b_c");
        assert_eq!(sanitize_basename("  spaced  "), "spaced");
        assert_eq!(sanitize_basename("a___b"), "a_b");
        assert_eq!(sanitize_basename("v1.2-final"), "v1.2-final");
        // Nothing usable left → the documented fallback.
        assert_eq!(sanitize_basename(""), "untitled");
        assert_eq!(sanitize_basename("///"), "untitled");
        assert_eq!(sanitize_basename("   "), "untitled");
    }
}
