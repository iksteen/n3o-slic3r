//! Decode BBS per-triangle `paint_color` strings to a dominant filament
//! state, for viewport display of MMU color-painting.
//!
//! The `paint_color` attribute on a 3MF `<triangle>` is libslic3r's
//! `FacetsAnnotation::get_triangle_as_string` encoding. It's a hex string
//! that, read in REVERSE with each nibble's bits taken LSB-first, forms the
//! `TriangleSelector` bitstream for that one triangle. A triangle may be
//! recursively split, each leaf carrying an `EnforcerBlockerType` state
//! (`0` = NONE = the object's own extruder; `N` = filament `N`, 1-based).
//!
//! For display we collapse a split triangle to its most-common leaf state —
//! the prepare-screen preview shows one colour per source triangle, not the
//! sub-triangle subdivision BBS renders. Faithful subdivision is a later
//! refinement; on a dense mesh the approximation is invisible.
//!
//! Ports two upstream halves (re-verify on submodule bump):
//! - `FacetsAnnotation::set_triangle_from_string` (Model.cpp) — the
//!   hex→bitstream step (reverse string, nibble bits LSB-first).
//! - `TriangleSelector::deserialize` (TriangleSelector.cpp:1772) — the
//!   split-tree walk + leaf-state decode.

/// Per-triangle dominant filament state for `paint_colors` (one entry per
/// triangle, in `indices`-triple order). `0` = unpainted (render with the
/// object's base material); `N` = filament `N`.
///
/// Returns `None` when nothing is painted (every entry empty or state 0), so
/// the caller can skip the per-face render path entirely.
pub fn decode_dominant_states(paint_colors: &[String]) -> Option<Vec<u8>> {
    let mut any_painted = false;
    let states: Vec<u8> = paint_colors
        .iter()
        .map(|s| {
            let st = dominant_state(s);
            any_painted |= st != 0;
            st
        })
        .collect();
    any_painted.then_some(states)
}

/// The dominant `EnforcerBlockerType` of one triangle's paint string — `0`
/// for an empty/unpainted/unparseable triangle.
fn dominant_state(paint_color: &str) -> u8 {
    if paint_color.is_empty() {
        return 0;
    }
    let bits = hex_to_bitstream(paint_color);
    let mut counts = [0u32; 256];
    let mut pos = 0usize;
    collect_leaf_states(&bits, &mut pos, &mut counts);

    // Most-common leaf state. Ties resolve to the higher state so a painted
    // filament wins a 50/50 split over NONE (the painted intent is what the
    // user cares to see).
    let mut best = 0u8;
    let mut best_count = 0u32;
    for state in 0u16..256 {
        let c = counts[state as usize];
        if c > 0 && (c > best_count || (c == best_count && state as u8 > best)) {
            best = state as u8;
            best_count = c;
        }
    }
    best
}

/// Hex string → bitstream, mirroring `set_triangle_from_string`: iterate the
/// string in REVERSE, and for each nibble append its 4 bits LSB-first.
fn hex_to_bitstream(s: &str) -> Vec<bool> {
    let mut bits = Vec::with_capacity(s.len() * 4);
    for ch in s.chars().rev() {
        let dec = match ch {
            '0'..='9' => ch as u8 - b'0',
            'A'..='F' => 10 + (ch as u8 - b'A'),
            'a'..='f' => 10 + (ch as u8 - b'a'),
            _ => return bits, // malformed → stop, decode what we have
        };
        for i in 0..4 {
            bits.push(dec & (1 << i) != 0);
        }
    }
    bits
}

/// Walk one node of the split-tree from `pos`, tallying each leaf's state.
/// Mirrors `TriangleSelector::deserialize`'s per-triangle loop: a node's
/// first nibble carries `split_sides` (low 2 bits); a leaf (`split_sides ==
/// 0`) then carries its state, a split node recurses into `split_sides + 1`
/// children.
fn collect_leaf_states(bits: &[bool], pos: &mut usize, counts: &mut [u32; 256]) {
    let Some(code) = next_nibble(bits, pos) else {
        return;
    };
    let split_sides = code & 0b11;
    if split_sides == 0 {
        // Leaf: state is `code >> 2`, or — when the low nibble's high bits are
        // `0b11` — an escape to a second nibble holding `state - 3`.
        let state = if (code & 0b1100) == 0b1100 {
            match next_nibble(bits, pos) {
                Some(n) => n + 3,
                None => return,
            }
        } else {
            code >> 2
        };
        counts[state as usize] += 1;
    } else {
        for _ in 0..(split_sides + 1) {
            collect_leaf_states(bits, pos, counts);
        }
    }
}

/// Read 4 bits LSB-first into a nibble, advancing `pos`. `None` when the
/// bitstream is exhausted (malformed/truncated input).
fn next_nibble(bits: &[bool], pos: &mut usize) -> Option<u8> {
    if *pos + 4 > bits.len() {
        return None;
    }
    let mut n = 0u8;
    for i in 0..4 {
        if bits[*pos] {
            n |= 1 << i;
        }
        *pos += 1;
    }
    Some(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_the_two_common_spinning_top_states() {
        // Real values from spinning-top.3mf: "8" is the body (856 tris),
        // "4" the accent (101). "8" = nibble 8 → state 8>>2 = 2 (filament 2);
        // "4" = nibble 4 → state 1 (filament 1).
        assert_eq!(dominant_state("8"), 2);
        assert_eq!(dominant_state("4"), 1);
    }

    #[test]
    fn empty_string_is_unpainted() {
        assert_eq!(dominant_state(""), 0);
    }

    #[test]
    fn walks_a_split_triangle_to_its_leaf_states() {
        // Synthetic split: a triangle split on one side (split_sides=1 →
        // 2 children), children = leaf state 2 then leaf state 1. Built per
        // the serialize layout: bitstream nibbles [1, 8, 4] (root split, then
        // the two leaves), which set_triangle_from_string stores as the
        // reversed string "481".
        let bits = hex_to_bitstream("481");
        let mut counts = [0u32; 256];
        let mut pos = 0;
        collect_leaf_states(&bits, &mut pos, &mut counts);
        assert_eq!(counts[1], 1, "one leaf is filament 1");
        assert_eq!(counts[2], 1, "one leaf is filament 2");
        // Only the two painted filaments appear — no spurious states from
        // mis-reading the split header as a leaf.
        for (state, &c) in counts.iter().enumerate() {
            if state != 1 && state != 2 {
                assert_eq!(c, 0, "unexpected leaf state {state}");
            }
        }
        // 1-1 tie resolves to the higher filament.
        assert_eq!(dominant_state("481"), 2);
    }

    #[test]
    fn decode_dominant_states_returns_none_when_nothing_painted() {
        assert!(decode_dominant_states(&["".into(), "".into()]).is_none());
        let states = decode_dominant_states(&["4".into(), "".into(), "8".into()])
            .expect("some painting present");
        assert_eq!(states, vec![1, 0, 2]);
    }
}
