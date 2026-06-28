//! Project dirty-state tracking — the backend-authoritative "unsaved
//! edits" signal.
//!
//! A monotonic `edit_seq` is bumped on every content edit (classified
//! by [`SceneEvent::dirty_effect`]); `clean_seq` is snapped to it on
//! save / load / import. The project is **dirty** when the two differ.
//! This single counter replaces both the old per-tick autosave content
//! hash (the worker compares `edit_seq` against what it last wrote,
//! instead of re-serializing to detect change) and the frontend's own
//! event classification (which now just reads `project:dirty_changed`).
//!
//! Tracking is maintained in `emit_all` (the one place every scene /
//! project event flows through), so any new mutation dirties the
//! project automatically as long as it emits an edit event.

use std::sync::atomic::{AtomicU64, Ordering};

use serde::Serialize;
use tauri::{Emitter, Manager, Window};

use crate::core::scene::events::{DirtyEffect, SceneEvent};

/// Backend-authoritative unsaved-edits counter. Managed as
/// `Arc<DirtyTracker>` Tauri state and shared with the autosave worker.
#[derive(Debug)]
pub struct DirtyTracker {
    /// Bumped once per content edit.
    edit_seq: AtomicU64,
    /// `edit_seq` as of the last save / load / import.
    clean_seq: AtomicU64,
}

impl Default for DirtyTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Serialize, Clone)]
struct DirtyChanged {
    dirty: bool,
}

impl DirtyTracker {
    pub fn new() -> Self {
        Self {
            edit_seq: AtomicU64::new(0),
            clean_seq: AtomicU64::new(0),
        }
    }

    /// Unsaved edits outstanding?
    pub fn is_dirty(&self) -> bool {
        self.edit_seq.load(Ordering::Acquire) != self.clean_seq.load(Ordering::Acquire)
    }

    /// Current edit sequence — the autosave worker records the value it
    /// last wrote and skips ticks where it hasn't advanced.
    pub fn edit_seq(&self) -> u64 {
        self.edit_seq.load(Ordering::Acquire)
    }

    /// Apply one batch of events' dirty effects. Returns `Some(dirty)`
    /// when the dirty state flipped (so the caller emits one
    /// `project:dirty_changed`), else `None`.
    pub(crate) fn apply(&self, events: &[SceneEvent]) -> Option<bool> {
        let before = self.is_dirty();
        for event in events {
            match event.dirty_effect() {
                DirtyEffect::Dirties => {
                    self.edit_seq.fetch_add(1, Ordering::AcqRel);
                }
                DirtyEffect::Cleans => {
                    // Snap the clean watermark to the current edit count —
                    // the on-disk form now matches memory.
                    self.clean_seq
                        .store(self.edit_seq.load(Ordering::Acquire), Ordering::Release);
                }
                DirtyEffect::Neutral => {}
            }
        }
        let after = self.is_dirty();
        (before != after).then_some(after)
    }
}

/// Update the window's [`DirtyTracker`] for a batch of just-emitted
/// events and emit `project:dirty_changed` when the dirty state flips.
/// Called from both `emit_all` choke points. A no-op when the tracker
/// isn't managed (e.g. headless tests that emit directly).
pub fn track(window: &Window, events: &[SceneEvent]) {
    let Some(tracker) = window.try_state::<std::sync::Arc<DirtyTracker>>() else {
        return;
    };
    if let Some(dirty) = tracker.apply(events) {
        let _ = window.emit("project:dirty_changed", DirtyChanged { dirty });
    }
}
