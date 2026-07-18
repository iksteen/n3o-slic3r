//! The `Session` superstruct: the app's live document = **persisted
//! content** ([`Project`]) + **runtime state** ([`SessionRuntime`]).
//!
//! Tauri manages `Arc<Mutex<Session>>`. `Project` and its `Plate`s are pure
//! serializable content — nothing ephemeral lives inside them. Everything
//! that exists only for the session lives in `SessionRuntime`:
//!   - project-level: the on-disk location (`source_path`), the crash
//!     recovery Save-As hint (`recovery_origin`), and the primitive-mesh
//!     dedup cache;
//!   - per-plate (keyed by [`PlateId`]): the live `selection` and the
//!     derived `bed` visualization.
//!
//! **Derived state follows the persisted structure via [`Session::reconcile`]**:
//! `bed` is a pure function of a plate's printer binding, so it's never
//! persisted or snapshotted — `reconcile` re-derives it (and drops runtime
//! for plates that no longer exist) after any load, undo/redo, or plate /
//! binding change. `selection` is genuine session state that undo tracks
//! (see [`super::history`]); `reconcile` preserves it for surviving plates.
//!
//! **Layering:** `impl Project` holds pure-persisted operations; operations
//! that also touch runtime (selection, bed, primitive cache) live in
//! `impl Session` and delegate the persisted half down to `Project`.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::core::scene::bed::BedMesh;
use crate::core::scene::primitives::{PrimitiveKind, PrimitiveParams};
use crate::core::scene::state::{MeshId, ObjectId};

use super::model::{Plate, PlateId, Project};

/// The live document: persisted [`Project`] + ephemeral [`SessionRuntime`].
#[derive(Debug, Clone, Default)]
pub struct Session {
    pub project: Project,
    pub runtime: SessionRuntime,
}

/// Session-scoped state, never serialized. Project-level fields plus a
/// per-plate map keyed by the persisted plate's [`PlateId`].
#[derive(Debug, Clone, Default)]
pub struct SessionRuntime {
    /// Filesystem path the project came from / saves to. `None` for an
    /// unsaved project. Populated by the load path / Save-As; the autosave
    /// worker and snapshot read it. Never persisted (it would leak an
    /// absolute path into a shared `.n3o`).
    pub source_path: Option<PathBuf>,

    /// Where a crash-recovered project should be saved back to (its
    /// pre-crash `source_path`). Set by `project_recover` from the recovery
    /// file's envelope; a recovered project's Save prompts a Save-As here.
    pub recovery_origin: Option<PathBuf>,

    /// Primitive-mesh dedup cache: each (kind, params) resolves to one
    /// `MeshId` so re-instancing the same procedural primitive shares
    /// geometry. Linear scan (stays small).
    pub(crate) primitive_cache: Vec<(PrimitiveKind, PrimitiveParams, MeshId)>,

    /// Per-plate runtime, keyed by [`PlateId`]. Maintained by
    /// [`Session::reconcile`]; an entry exists for exactly the plates in
    /// `project.plates` after any reconcile.
    pub plates: HashMap<PlateId, PlateRuntime>,
}

impl SessionRuntime {
    /// The origin to embed in an autosave recovery file: the current save
    /// target, else the origin a recovered-but-unsaved project carries.
    pub fn autosave_origin(&self) -> Option<&Path> {
        self.source_path
            .as_deref()
            .or(self.recovery_origin.as_deref())
    }
}

/// Runtime state owned by one plate.
#[derive(Debug, Clone, Default)]
pub struct PlateRuntime {
    /// Live selection — transient UI, reset when the plate is (re)created.
    /// Undo tracks it (snapshotted alongside the project).
    pub selection: HashSet<ObjectId>,

    /// Derived bed visualization + bounds, a pure function of the plate's
    /// printer binding. `None` when unbound. Re-derived by `reconcile`;
    /// its `exclusion_zones` field is the authoritative exclusion source.
    pub bed: Option<BedMesh>,
}

/// Derive a plate's bed from its printer binding, resolving the profile
/// through the registry. `None` when unbound or the profile no longer
/// resolves. The one place bed derivation happens (was `Plate::set_printer`).
pub fn derive_bed(plate: &Plate) -> Option<BedMesh> {
    let instance_id = plate.printer_instance_id()?;
    let instance = crate::core::printer::lookup_instance(instance_id)?;
    let profile = crate::core::printer::lookup(&instance.vendor_profile_ref)?;
    Some(crate::core::scene::bed::bed_for_printer(&profile))
}

impl Session {
    /// Wrap a freshly-built or loaded `Project`, seeding runtime (beds
    /// derived, selection empty). `runtime` project-level fields
    /// (source_path/recovery_origin) start empty — the caller sets them.
    pub fn new(project: Project) -> Self {
        let mut session = Self {
            project,
            runtime: SessionRuntime::default(),
        };
        session.reconcile();
        session
    }

    /// Make runtime follow the persisted structure: one [`PlateRuntime`]
    /// per plate, `bed` re-derived from each plate's binding, entries for
    /// vanished plates dropped, `selection` preserved for surviving plates
    /// (a newly-appearing plate — bootstrap, add, or undo-resurrected —
    /// starts with an empty selection).
    ///
    /// Call after anything that changes the plate set or a binding: load /
    /// new / recover, undo / redo, add / remove plate, printer (re)bind.
    /// Cheap — a handful of plates, each a registry lookup.
    pub fn reconcile(&mut self) {
        let live: HashSet<PlateId> = self.project.plates.iter().map(|p| p.id).collect();
        // Drop runtime for plates that no longer exist. Mandatory: PlateIds
        // can be reused (`next_plate_id` = max+1), so a stale entry would be
        // silently inherited by a new plate.
        self.runtime.plates.retain(|id, _| live.contains(id));
        for plate in &self.project.plates {
            let bed = derive_bed(plate);
            self.runtime.plates.entry(plate.id).or_default().bed = bed;
        }
    }

    /// Runtime for the active plate. `reconcile` guarantees an entry exists
    /// for every plate, so this never returns a phantom.
    pub fn active_plate_runtime(&self) -> &PlateRuntime {
        let id = self.project.active_plate().id;
        self.runtime
            .plates
            .get(&id)
            .expect("reconcile keeps a PlateRuntime per plate")
    }

    /// Runtime for `id`, or `None` if the plate doesn't exist.
    pub fn plate_runtime(&self, id: PlateId) -> Option<&PlateRuntime> {
        self.runtime.plates.get(&id)
    }

    /// Mutable runtime for `id`, creating an empty entry if absent (a
    /// caller mutating a just-added plate before the next reconcile).
    pub fn plate_runtime_mut(&mut self, id: PlateId) -> &mut PlateRuntime {
        self.runtime.plates.entry(id).or_default()
    }

    /// Mutable runtime for the active plate.
    pub fn active_plate_runtime_mut(&mut self) -> &mut PlateRuntime {
        let id = self.project.active_plate().id;
        self.plate_runtime_mut(id)
    }

    /// Document title for sliced-output naming — the project's title resolved
    /// against the runtime save path (its file stem is the metadata fallback).
    pub fn title(&self) -> String {
        self.project.title(self.runtime.source_path.as_deref())
    }

    /// Resolve the active plate's bound `PrinterInstance` from the registry
    /// (`None` when unbound or the id no longer resolves). The registry access
    /// lives here at the command boundary so the mutation methods stay pure —
    /// they take the resolved instance as a parameter (e.g. `add_from_primitive`,
    /// `set_material_slot`).
    pub fn active_plate_instance(&self) -> Option<crate::core::printer::PrinterInstance> {
        self.plate_instance(self.project.active_plate().id)
    }

    /// Resolve plate `id`'s bound `PrinterInstance` from the registry.
    pub fn plate_instance(&self, id: PlateId) -> Option<crate::core::printer::PrinterInstance> {
        self.project
            .plate(id)?
            .printer_instance_id()
            .and_then(crate::core::printer::lookup_instance)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::printer::instance_registry::RegistryGuard;
    use crate::core::scene::state::ObjectId;

    #[test]
    fn reconcile_derives_bed_from_binding() {
        let _guard = RegistryGuard::acquire();
        // `Project::default` binds the bootstrap plate to the first registered
        // instance (the bundled a1-mini `bambi`); `Session::new` reconciles.
        let session = Session::new(Project::default());
        assert!(
            session.active_plate_runtime().bed.is_some(),
            "a bound plate's bed derives on reconcile",
        );
    }

    #[test]
    fn reconcile_clears_bed_for_an_unbound_plate() {
        let mut project = Project::default();
        project.plates[0].set_printer(None); // unbind
        let session = Session::new(project);
        assert!(session.active_plate_runtime().bed.is_none());
    }

    #[test]
    fn reconcile_drops_runtime_for_vanished_plates() {
        let mut session = Session::new(Project::default());
        // A stale entry for a plate id that isn't in the project.
        let ghost = PlateId(999);
        session.runtime.plates.entry(ghost).or_default();
        assert!(session.runtime.plates.contains_key(&ghost));
        session.reconcile();
        assert!(
            !session.runtime.plates.contains_key(&ghost),
            "reconcile drops runtime for plates the project no longer has \
             (PlateIds can be reused, so a stale entry must not linger)",
        );
    }

    #[test]
    fn reconcile_preserves_selection_for_surviving_plates() {
        let mut session = Session::new(Project::default());
        let id = session.project.active_plate().id;
        session.plate_runtime_mut(id).selection.insert(ObjectId(7));
        session.reconcile();
        assert!(
            session
                .active_plate_runtime()
                .selection
                .contains(&ObjectId(7)),
            "a surviving plate keeps its selection across reconcile",
        );
    }
}
