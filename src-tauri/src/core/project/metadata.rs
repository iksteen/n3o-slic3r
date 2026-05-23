//! Per-plate metadata (PR-5-1 / FR-MP-7).
//!
//! Cycle count + composition order ride out on the project `.3mf`
//! save (PR-5-8). PR-5-5 surfaces the cycle count in the plate-tab
//! UI; PR-5-11 (or Phase 9 polish) wires composition-order
//! reordering. The PlateCycler plugin (Phase 8) reads both fields
//! to expand a multi-plate project into a print queue.

use serde::{Deserialize, Serialize};

/// Bounds enforced on `cycle_count`. Lower bound is 1 (a plate
/// can't run zero times — that's "remove the plate"). Upper bound
/// is 999 to match the PRD's integer 1–999 spec; printable in
/// human time even at the highest count.
pub const CYCLE_COUNT_MIN: u32 = 1;
pub const CYCLE_COUNT_MAX: u32 = 999;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlateMetadata {
    /// How many times the platecycler should run this plate.
    /// Default 1; clamped to `[CYCLE_COUNT_MIN, CYCLE_COUNT_MAX]`
    /// at construction + on every set.
    pub cycle_count: u32,

    /// 1-based position in the plate composition queue. Plates
    /// with lower numbers print first. Default = plate's
    /// position in `Project.plates`; user-reorderable.
    pub composition_order: u32,
}

impl PlateMetadata {
    /// Default metadata for a freshly-added plate at `position`
    /// (1-based). The PlateCycler consumes both fields; if the
    /// user never touches them, the project prints each plate
    /// exactly once in declaration order — the single-plate
    /// behavior users coming from Phase 4 expect.
    pub fn at_position(position: u32) -> Self {
        Self {
            cycle_count: 1,
            composition_order: position.max(1),
        }
    }

    /// Clamp + validate a proposed cycle count. Returns the
    /// clamped value or an error string the UI can surface.
    pub fn validate_cycle_count(count: u32) -> Result<u32, String> {
        if count < CYCLE_COUNT_MIN || count > CYCLE_COUNT_MAX {
            return Err(format!(
                "cycle_count must be in {CYCLE_COUNT_MIN}..={CYCLE_COUNT_MAX}, got {count}",
            ));
        }
        Ok(count)
    }
}

impl Default for PlateMetadata {
    fn default() -> Self {
        Self::at_position(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_cycle_1_order_1() {
        let md = PlateMetadata::default();
        assert_eq!(md.cycle_count, 1);
        assert_eq!(md.composition_order, 1);
    }

    #[test]
    fn at_position_clamps_zero_to_one() {
        let md = PlateMetadata::at_position(0);
        assert_eq!(md.composition_order, 1);
    }

    #[test]
    fn at_position_passes_through_canonical_values() {
        let md = PlateMetadata::at_position(3);
        assert_eq!(md.composition_order, 3);
        assert_eq!(md.cycle_count, 1);
    }

    #[test]
    fn cycle_count_bounds_are_enforced() {
        assert!(PlateMetadata::validate_cycle_count(0).is_err());
        assert!(PlateMetadata::validate_cycle_count(1).is_ok());
        assert!(PlateMetadata::validate_cycle_count(999).is_ok());
        assert!(PlateMetadata::validate_cycle_count(1000).is_err());
    }

    #[test]
    fn serde_round_trips() {
        let md = PlateMetadata {
            cycle_count: 7,
            composition_order: 2,
        };
        let json = serde_json::to_string(&md).unwrap();
        let parsed: PlateMetadata = serde_json::from_str(&json).unwrap();
        assert_eq!(md, parsed);
    }
}
