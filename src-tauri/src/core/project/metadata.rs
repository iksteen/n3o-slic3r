//! Per-plate metadata (PR-5-1 / FR-MP-7).
//!
//! Composition order rides out on the project `.3mf` save (PR-5-8).
//! PR-5-11 (or Phase 9 polish) wires composition-order reordering;
//! the composition plugin host (Phase 8) consumes it to drive the
//! print queue. `cycle_count` was cut as MVP scope — it only ever
//! had a Phase 8 plugin consumer (PlateCycler).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlateMetadata {
    /// 1-based position in the plate composition queue. Plates
    /// with lower numbers print first. Default = plate's
    /// position in `Project.plates`; user-reorderable.
    pub composition_order: u32,
}

impl PlateMetadata {
    /// Default metadata for a freshly-added plate at `position`
    /// (1-based). If the user never touches it, the project
    /// prints plates in declaration order — the single-plate
    /// behavior users coming from Phase 4 expect.
    pub fn at_position(position: u32) -> Self {
        Self {
            composition_order: position.max(1),
        }
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
    fn default_is_order_1() {
        let md = PlateMetadata::default();
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
    }

    #[test]
    fn serde_round_trips() {
        let md = PlateMetadata {
            composition_order: 2,
        };
        let json = serde_json::to_string(&md).unwrap();
        let parsed: PlateMetadata = serde_json::from_str(&json).unwrap();
        assert_eq!(md, parsed);
    }
}
