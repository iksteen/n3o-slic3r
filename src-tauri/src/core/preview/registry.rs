//! Preview handle registry.
//!
//! Tauri-managed state that owns every loaded G-code preview.
//! Handle-based addressing lets the frontend reference a single
//! parse+IR build across multiple commands (buffers, stats,
//! hover) without re-loading the file each time.
//!
//! Memory model: each `LoadedPreview` holds the typed line stream
//! (`Vec<Line>`) alongside the rendered IR. The line stream is
//! ~5× the IR's size — a 50MB gcode produces ~250MB of in-memory
//! Lines. The drop command frees both. Holding the lines is
//! load-bearing for hover inspection, which fetches
//! the original gcode line via back-reference.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::core::gcode::{HeaderMetadata, Line};

use super::ir::PreviewGeometry;
use super::stats::{FullJobStats, PerLayerStats};

/// Opaque handle issued by [`PreviewRegistry::insert`]. 1-based;
/// `0` is reserved as "no preview".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PreviewHandle(pub u64);

/// Everything a preview command needs to answer subsequent calls
/// (buffers / stats / hover) without re-parsing the source file.
pub struct LoadedPreview {
    pub source_path: PathBuf,
    pub header: HeaderMetadata,
    pub geometry: PreviewGeometry,
    pub layer_stats: Vec<PerLayerStats>,
    pub job_stats: FullJobStats,
    /// Original typed line stream. Heavy (~5× the geometry) but
    /// required for [`super::commands::preview_segment_detail`]
    /// to look up the source line text on hover.
    pub lines: Vec<Line>,
}

/// Tauri-managed state. Wraps a short-held mutex around the
/// per-handle table. Allocator is `AtomicU64`; allocation is
/// lock-free.
pub struct PreviewRegistry {
    next_id: AtomicU64,
    slots: Mutex<HashMap<PreviewHandle, LoadedPreview>>,
}

impl PreviewRegistry {
    pub fn new() -> Self {
        Self {
            next_id: AtomicU64::new(1),
            slots: Mutex::new(HashMap::new()),
        }
    }

    /// Allocate a fresh handle. Lock-free.
    pub fn alloc_id(&self) -> PreviewHandle {
        PreviewHandle(self.next_id.fetch_add(1, Ordering::Relaxed))
    }

    /// Insert a loaded preview against the supplied handle. The
    /// caller (the load command) typically calls `alloc_id` first
    /// so it can include the id in the response before inserting.
    pub fn insert(&self, handle: PreviewHandle, preview: LoadedPreview) {
        if let Ok(mut guard) = self.slots.lock() {
            guard.insert(handle, preview);
        }
    }

    /// Take ownership of a slot's preview for the duration of a
    /// closure. Holds the registry lock for the closure's lifetime
    /// — keep the closure short. Returns whatever the closure
    /// returns, or `None` when the handle is unknown.
    pub fn with<R>(&self, handle: PreviewHandle, f: impl FnOnce(&LoadedPreview) -> R) -> Option<R> {
        let guard = self.slots.lock().ok()?;
        guard.get(&handle).map(f)
    }

    /// Drop the preview at `handle` + return whether one was
    /// present. Frees the heavy line stream + IR allocations.
    pub fn remove(&self, handle: PreviewHandle) -> bool {
        match self.slots.lock() {
            Ok(mut guard) => guard.remove(&handle).is_some(),
            Err(_) => false,
        }
    }
}

impl Default for PreviewRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::gcode::header::HeaderMetadata;
    use crate::core::preview::ir::BoundingBox;
    use crate::core::preview::stats::HeightStats;

    fn empty_preview() -> LoadedPreview {
        LoadedPreview {
            source_path: PathBuf::from("/tmp/empty.gcode"),
            header: HeaderMetadata::default(),
            geometry: PreviewGeometry::default(),
            layer_stats: vec![],
            job_stats: FullJobStats {
                total_duration_seconds: 0.0,
                layer_count: 0,
                feature_breakdown: HashMap::new(),
                filament_used_mm: HashMap::new(),
                bounding_box: BoundingBox::default(),
                layer_heights: HeightStats {
                    min: 0.0,
                    max: 0.0,
                    variable: false,
                },
            },
            lines: vec![],
        }
    }

    #[test]
    fn alloc_id_starts_at_one_and_monotonic() {
        let r = PreviewRegistry::new();
        let a = r.alloc_id();
        let b = r.alloc_id();
        assert_eq!(a.0, 1);
        assert_eq!(b.0, 2);
        assert_ne!(a, b);
    }

    #[test]
    fn insert_lookup_remove_round_trip() {
        let r = PreviewRegistry::new();
        let h = r.alloc_id();
        r.insert(h, empty_preview());
        let layer_count = r.with(h, |p| p.job_stats.layer_count).expect("present");
        assert_eq!(layer_count, 0);
        assert!(r.remove(h));
        assert!(r.with(h, |_| ()).is_none(), "removed slot is gone");
        assert!(!r.remove(h), "second remove is a no-op");
    }

    #[test]
    fn concurrent_loads_get_distinct_handles() {
        let r = PreviewRegistry::new();
        let h1 = r.alloc_id();
        let h2 = r.alloc_id();
        r.insert(h1, empty_preview());
        r.insert(h2, empty_preview());
        assert_ne!(h1, h2);
        assert!(r.with(h1, |_| ()).is_some());
        assert!(r.with(h2, |_| ()).is_some());
    }
}
