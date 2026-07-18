//! Undo / redo — snapshot-based project history.
//!
//! Each committed edit records a `Project` snapshot. The clone is cheap:
//! mesh geometry is `#[serde(skip)] Arc<Vec<_>>` (shared by refcount,
//! never copied) and the mesh store is never pruned, so a restored
//! snapshot re-references the same `MeshId`s the renderer already holds.
//!
//! Snapshots are captured at the `emit_all` seam — the one place every
//! edit flows through — so any new mutation is undoable as long as it
//! emits an edit event. Rapid bursts (a multi-object drag commits N
//! parallel `set_transform`s) coalesce into one step by time window.
//!
//! `snapshots[cursor]` mirrors the live project; undo/redo move the
//! cursor and copy that snapshot back over the live project. A restore
//! emits [`SceneEvent::ProjectRestored`] (resync + dirty), which
//! `track` deliberately ignores so it doesn't record itself.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{Emitter, Manager, Window};

use super::model::{PlateId, Project};
use super::session::Session;
use crate::core::scene::state::ObjectId;
use crate::core::scene::events::{DirtyEffect, SceneEvent};

/// Hard cap on retained snapshots — bounds memory on a long session.
const MAX_DEPTH: usize = 100;
/// Edits closer together than this merge into one undo step.
const COALESCE_WINDOW: Duration = Duration::from_millis(250);

/// One undo step: the persisted [`Project`] plus the per-plate `selection`
/// — the only runtime state undo tracks (derived state like `bed` is
/// re-derived by [`Session::reconcile`] on restore, so it's not snapshotted).
#[derive(Clone)]
pub struct UndoSnapshot {
    project: Project,
    selection: HashMap<PlateId, HashSet<ObjectId>>,
}

impl UndoSnapshot {
    /// Capture the current live state from a `Session`.
    pub fn capture(session: &Session) -> Self {
        Self {
            project: session.project.clone(),
            selection: session
                .runtime
                .plates
                .iter()
                .map(|(id, rt)| (*id, rt.selection.clone()))
                .collect(),
        }
    }

    /// Restore into a `Session`: swap the project, reconcile derived runtime
    /// (beds follow the restored bindings; runtime for vanished plates is
    /// dropped), then overlay the snapshot's selection.
    fn restore_into(self, session: &mut Session) {
        session.project = self.project;
        session.reconcile();
        for (id, selection) in self.selection {
            if let Some(rt) = session.runtime.plates.get_mut(&id) {
                rt.selection = selection;
            }
        }
    }
}

pub struct UndoHistory {
    /// `snapshots[cursor]` is the live state; entries before it are undo
    /// targets, entries after are redo targets.
    snapshots: Vec<UndoSnapshot>,
    cursor: usize,
    /// When the last edit was recorded — drives burst coalescing.
    last_record: Option<Instant>,
}

impl UndoHistory {
    pub fn new(initial: UndoSnapshot) -> Self {
        Self {
            snapshots: vec![initial],
            cursor: 0,
            last_record: None,
        }
    }

    pub fn can_undo(&self) -> bool {
        self.cursor > 0
    }

    pub fn can_redo(&self) -> bool {
        self.cursor + 1 < self.snapshots.len()
    }

    /// Reset the history to a fresh baseline — used when a project is
    /// loaded/imported (you can't undo into the previous project).
    fn reset(&mut self, baseline: UndoSnapshot) {
        self.snapshots = vec![baseline];
        self.cursor = 0;
        self.last_record = None;
    }

    /// Refresh the current snapshot in place — used when only the
    /// selection changed. Selection isn't its own undo step, but the
    /// live state's snapshot should carry the latest selection so a
    /// restore (and the redo back to here) reflects it. Doesn't grow the
    /// stack, touch the redo branch, or reset the coalesce timer.
    fn refresh_current(&mut self, current: UndoSnapshot) {
        self.snapshots[self.cursor] = current;
    }

    /// Record a committed edit. `current` is the post-edit live state.
    fn record(&mut self, current: UndoSnapshot, now: Instant) {
        // A new edit invalidates the redo branch.
        self.snapshots.truncate(self.cursor + 1);

        let coalesce = self
            .last_record
            .is_some_and(|t| now.duration_since(t) < COALESCE_WINDOW);
        if coalesce {
            // Merge into the current step instead of growing the stack.
            self.snapshots[self.cursor] = current;
        } else {
            self.snapshots.push(current);
            self.cursor += 1;
            if self.snapshots.len() > MAX_DEPTH {
                let overflow = self.snapshots.len() - MAX_DEPTH;
                self.snapshots.drain(0..overflow);
                self.cursor -= overflow;
            }
        }
        self.last_record = Some(now);
    }

    /// Step back one snapshot. Returns the state to restore.
    fn undo(&mut self) -> Option<UndoSnapshot> {
        if self.cursor == 0 {
            return None;
        }
        self.cursor -= 1;
        self.last_record = None; // a navigation ends any coalescing run
        Some(self.snapshots[self.cursor].clone())
    }

    /// Step forward one snapshot.
    fn redo(&mut self) -> Option<UndoSnapshot> {
        if self.cursor + 1 >= self.snapshots.len() {
            return None;
        }
        self.cursor += 1;
        self.last_record = None;
        Some(self.snapshots[self.cursor].clone())
    }
}

#[derive(Serialize, Clone)]
struct HistoryChanged {
    can_undo: bool,
    can_redo: bool,
}

fn emit_history_changed(window: &Window, history: &UndoHistory) {
    let _ = window.emit(
        "project:history_changed",
        HistoryChanged {
            can_undo: history.can_undo(),
            can_redo: history.can_redo(),
        },
    );
}

/// Maintain the undo history for a batch of just-emitted events. Called
/// from `emit_all` alongside `dirty::track`. A no-op when the history /
/// project aren't managed (headless tests emitting directly).
pub fn track(window: &Window, events: &[SceneEvent]) {
    // A restore is a history *navigation*, not a new edit — never record it.
    if events
        .iter()
        .any(|e| matches!(e, SceneEvent::ProjectRestored))
    {
        return;
    }
    let resets = events.iter().any(|e| {
        matches!(
            e,
            SceneEvent::ProjectLoaded { .. } | SceneEvent::ProjectImported { .. }
        )
    });
    let edits = events
        .iter()
        .any(|e| e.dirty_effect() == DirtyEffect::Dirties);
    // A pure selection change isn't an undo step, but the current
    // snapshot should track it so a restore reflects the latest selection.
    let selection_only = !edits
        && !resets
        && events
            .iter()
            .any(|e| matches!(e, SceneEvent::SelectionChanged { .. }));
    if !resets && !edits && !selection_only {
        return;
    }

    let (Some(history), Some(session)) = (
        window.try_state::<Arc<Mutex<UndoHistory>>>(),
        window.try_state::<Arc<Mutex<Session>>>(),
    ) else {
        return;
    };
    let snapshot = {
        let s = session.lock().expect("session poisoned");
        UndoSnapshot::capture(&s)
    };
    let mut h = history.lock().expect("history poisoned");
    if resets {
        h.reset(snapshot);
    } else if edits {
        h.record(snapshot, Instant::now());
    } else {
        // Selection-only: refresh the current snapshot, no new step.
        h.refresh_current(snapshot);
        return;
    }
    emit_history_changed(window, &h);
}

/// Apply an undo or redo step, replacing the live project with the
/// stored snapshot and emitting the resync + history events. Returns
/// whether a step was applied. The shared body of the two commands.
pub fn apply_step(
    window: &Window,
    session: &Arc<Mutex<Session>>,
    history: &Arc<Mutex<UndoHistory>>,
    redo: bool,
) -> bool {
    let restored = {
        let mut h = history.lock().expect("history poisoned");
        if redo {
            h.redo()
        } else {
            h.undo()
        }
    };
    let Some(restored) = restored else {
        return false;
    };
    {
        let mut s = session.lock().expect("session poisoned");
        restored.restore_into(&mut s);
    }
    // ProjectRestored is classified Dirties, so emit_all's dirty::track
    // marks the project dirty again; history::track ignores it (a
    // navigation, not a new edit). The frontend resyncs off this event.
    super::commands::emit_all(window, &[SceneEvent::ProjectRestored]);
    let h = history.lock().expect("history poisoned");
    emit_history_changed(window, &h);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proj() -> Project {
        Project::default()
    }

    /// Wrap a bare project into an undo step (empty selection) for the
    /// history tests, which exercise the stack mechanics, not selection.
    fn snap(project: Project) -> UndoSnapshot {
        UndoSnapshot {
            project,
            selection: HashMap::new(),
        }
    }

    fn t0() -> Instant {
        Instant::now()
    }

    #[test]
    fn fresh_history_has_nothing_to_undo_or_redo() {
        let h = UndoHistory::new(snap(proj()));
        assert!(!h.can_undo());
        assert!(!h.can_redo());
    }

    #[test]
    fn record_then_undo_redo_round_trips() {
        let base = proj();
        let mut h = UndoHistory::new(snap(base.clone()));
        let mut edited = proj();
        edited.user_overrides.insert("k".into(), "v".into());
        h.record(snap(edited.clone()), t0());

        assert!(h.can_undo());
        assert!(!h.can_redo());
        let undone = h.undo().expect("undo");
        assert!(undone.project.user_overrides.is_empty(), "undo returns the baseline");
        assert!(!h.can_undo());
        assert!(h.can_redo());
        let redone = h.redo().expect("redo");
        assert_eq!(redone.project.user_overrides.get("k").map(String::as_str), Some("v"));
    }

    #[test]
    fn a_new_edit_after_undo_drops_the_redo_branch() {
        let mut h = UndoHistory::new(snap(proj()));
        h.record(snap(proj()), t0());
        h.record(snap(proj()), t0() + COALESCE_WINDOW * 2);
        h.undo();
        assert!(h.can_redo());
        // Recording a fresh edit (well past the window) truncates redo.
        h.record(snap(proj()), t0() + COALESCE_WINDOW * 10);
        assert!(!h.can_redo());
    }

    #[test]
    fn refresh_current_updates_the_top_snapshot_without_a_new_step() {
        // A selection change refreshes the live snapshot in place (using
        // user_overrides here as a stand-in marker), so a redo back to it
        // reflects the latest selection — but it adds no undo step.
        let mut h = UndoHistory::new(snap(proj()));
        h.record(snap(proj()), t0());
        let mut marked = proj();
        marked.user_overrides.insert("sel".into(), "x".into());
        h.refresh_current(snap(marked));

        assert_eq!(h.snapshots.len(), 2, "no new step added");
        assert!(h.can_undo());
        assert!(!h.can_redo());
        h.undo();
        let redone = h.redo().expect("redo");
        assert_eq!(
            redone.project.user_overrides.get("sel").map(String::as_str),
            Some("x"),
            "redo lands on the refreshed snapshot",
        );
    }

    #[test]
    fn bursts_within_the_window_coalesce_to_one_step() {
        let base = proj();
        let mut h = UndoHistory::new(snap(base));
        let start = t0();
        // Three rapid edits (a multi-object drag's parallel commits).
        h.record(snap(proj()), start);
        h.record(snap(proj()), start + Duration::from_millis(20));
        h.record(snap(proj()), start + Duration::from_millis(40));
        // One undo returns to the pre-burst baseline.
        assert!(h.can_undo());
        h.undo();
        assert!(!h.can_undo(), "the whole burst is a single undo step");
    }

    #[test]
    fn undo_snapshot_captures_and_restores_selection() {
        // Selection is the one runtime field undo tracks: capture it from a
        // Session, mutate it away, then restore and confirm it comes back.
        let mut session = Session::new(proj());
        let id = session.project.active_plate().id;
        session.plate_runtime_mut(id).selection.insert(ObjectId(3));

        let snap = UndoSnapshot::capture(&session);
        session.plate_runtime_mut(id).selection.clear();
        snap.restore_into(&mut session);

        assert!(
            session
                .plate_runtime(id)
                .unwrap()
                .selection
                .contains(&ObjectId(3)),
            "undo restores the tracked selection",
        );
    }

    #[test]
    fn depth_cap_drops_oldest_and_keeps_cursor_valid() {
        let mut h = UndoHistory::new(snap(proj()));
        // Spread well past the coalesce window so each is its own step.
        for i in 0..(MAX_DEPTH + 20) {
            h.record(snap(proj()), t0() + COALESCE_WINDOW * (i as u32 + 1) * 2);
        }
        assert_eq!(h.snapshots.len(), MAX_DEPTH);
        assert_eq!(h.cursor, MAX_DEPTH - 1);
        assert!(h.can_undo());
        assert!(!h.can_redo());
    }
}
