//! Slice orchestrator worker.
//!
//! Spawns a worker thread that walks the [`SliceJobInput`]'s plate
//! list, resolves the cascade + adapts to a libslic3r `Config` per
//! plate, fires `slic3r_ffi::slice` with the progress callback
//! routed through Tauri events, builds a `PlateSummary` on success
//! (or classifies the error on failure), emits per-plate +
//! per-job lifecycle events.
//!
//! Off the UI thread per FR-SL-2. Sequential plates per FR-SL-1.
//! Errors typed + attributed per FR-SL-3. Summary attached per
//! FR-SL-4. Output paths per FR-SL-5.
//!
//! ## Threading
//!
//! - The orchestrator's start path allocates a [`JobId`],
//!   builds a [`ResolvedJob`] (cascade lookup + context conversion),
//!   inserts a [`JobHandle`] into the registry, spawns the worker,
//!   and returns the id immediately.
//! - The worker owns the slicing thread for the job's lifetime. It
//!   checks the cancel flag between plates, and `slice_cancel` also
//!   aborts the plate currently slicing mid-`process()` via
//!   `slic3r_ffi::cancel_active_slice` (libslic3r's `throw_if_canceled`):
//!   that surfaces as an `Err`, which the worker reports as
//!   `slice:cancelled` (not a failure) because the flag is set.
//! - Progress events are throttled: at most one per 50 ms per
//!   plate, plus an immediate event on every stage transition.
//!   Libslic3r emits hundreds of ticks per second on large plates
//!   and we'd saturate the Tauri event channel without this.
//!
//! ## FFI progress callback ownership
//!
//! The progress callback is passed *into* `slic3r_ffi::slice` per
//! call — no global registration, no cross-slice contamination. The
//! orchestrator's worker thread owns the closure for the lifetime of
//! one `slice` call; concurrent jobs (whenever they land) would each
//! carry their own.
//!
//! Slice serialization happens inside `slic3r_ffi::slice` itself
//! (process-wide mutex around the call) — see the docstring there.
//! From the orchestrator's perspective each slice runs in isolation;
//! multi-job-parallelism later just queues on the FFI's mutex until
//! libslic3r is verified concurrent-safe.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::errors::{classify_libslic3r_error, SliceError};
use super::events::SliceEvent;
use super::job::{JobHandle, JobId, JobRegistry, JobStatus, ResolvedJob, SliceJobInput};
use super::summary::build_summary;
use crate::core::cascade::commands::{ContextJson, OverrideFileSpec};
use crate::core::cascade::{
    parse_override_str, resolve_with_overrides, to_resolved, types::Cascade, FlatOverrides,
    OverrideTiers, Resolved, ResolvedValue, SourceLocation,
};
use crate::core::cascade_adapter::adapt;
use crate::core::gcode::{parse_str, to_string};
use crate::core::plugin::{
    DispatchGate, FilamentLoadout, HookKind, PlateMeta, PluginHost, PostSliceHook, PreSliceContext,
    PreSliceHook,
};
use crate::core::printer::lookup_instance;
use crate::core::profile_library::{compose_cascade, with_quality_profile};
use crate::core::project::SlicingContext;
use slic3r_ffi::{slice_outcome, Model};
use std::collections::BTreeMap;

/// Shared plugin host the worker dispatches the post-slice hook
/// through. `None` when no host is wired (tests / tooling) — the
/// post-slice step is then skipped entirely.
pub type PluginHostRef = Arc<Mutex<PluginHost>>;

/// Errors the orchestrator returns synchronously (before the
/// worker thread spawns). Post-spawn errors flow out via the
/// `slice:job_failed` event.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data")]
pub enum SliceStartError {
    NoPlatesRequested,
    OutputDirInvalid(String),
    /// `printer_instance_id` doesn't match any bundled PrinterInstance,
    /// or the instance's fragment slugs don't resolve to bundled
    /// cascade fragments.
    PrinterInstanceCompose(String),
    /// Pre-slice validation gate refused the job. The
    /// frontend renders the failure on the binding panel.
    SliceBlocked(super::pre_slice_gate::PlateValidationFailure),
}

impl std::fmt::Display for SliceStartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoPlatesRequested => write!(f, "no plates in job"),
            Self::OutputDirInvalid(p) => write!(f, "output_dir not usable: {p}"),
            Self::PrinterInstanceCompose(s) => {
                write!(f, "printer-instance cascade compose failed: {s}")
            }
            Self::SliceBlocked(fail) => write!(
                f,
                "plate {} has {} pre-slice issue(s); fix in the binding panel before slicing",
                fail.plate_id,
                fail.issues.len(),
            ),
        }
    }
}

impl std::error::Error for SliceStartError {}

/// Event-sink callback the worker thread uses to surface every
/// lifecycle transition. The production path (`slice::commands`) emits
/// each event on its Tauri channel; tests inject a `Vec`-pushing
/// closure to inspect the stream.
pub type EventSink = Box<dyn Fn(SliceEvent) + Send + Sync + 'static>;

/// Resolve the [`Cascade`] this job slices against.
///
/// Looks the named PrinterInstance up in the bundled library and
/// composes a fresh authored cascade from its per-bucket vendor fragments
/// (plus the instance's own machine overrides). Composition happens per
/// job, not against a shared registry; there's no caching.
///
/// The user / project / object override tiers are NOT folded here — the
/// worker applies them as the second phase via
/// [`override_tiers_from_context`] + `cascade::resolve_with_overrides`.
fn resolve_cascade(input: &SliceJobInput) -> Result<Cascade, SliceStartError> {
    let instance = lookup_instance(&input.printer_instance_id).ok_or_else(|| {
        SliceStartError::PrinterInstanceCompose(format!(
            "unknown printer instance id `{}`",
            input.printer_instance_id,
        ))
    })?;
    // The plate's process/quality profile overrides the instance's
    // (per-plate binding); `with_quality_profile` swaps it in only when
    // set, so the composer picks the plate's process fragment.
    let effective = with_quality_profile(&instance, input.quality_profile.as_deref());
    compose_cascade(&effective, &input.material_layout)
        .map_err(|e| SliceStartError::PrinterInstanceCompose(e.to_string()))
}

/// Build the two-phase override tiers from the slice context. Each tier
/// arrives as a list of TOML override specs (`user_overrides`,
/// `project_overrides`) plus the per-object map; `cascade::resolve_with_
/// overrides` applies them on top of the authored cascade in
/// user → project → object precedence. `plugin.*` keys are dropped — they
/// drive the plugin gate ([`plugin_overrides_for_tier`]), never the
/// libslic3r adapter.
fn override_tiers_from_context(ctx: &ContextJson) -> OverrideTiers {
    OverrideTiers {
        user: flat_overrides_from_specs(&ctx.user_overrides),
        project: flat_overrides_from_specs(&ctx.project_overrides),
        object: {
            let entries: BTreeMap<String, String> = ctx
                .object_overrides
                .iter()
                .filter(|(k, _)| !k.starts_with("plugin."))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            (!entries.is_empty()).then(|| FlatOverrides {
                source: SourceLocation {
                    path: PathBuf::from("<object-overrides>"),
                    line: 1,
                },
                entries,
            })
        },
    }
}

/// Parse each override spec into a `FlatOverrides`, dropping `plugin.*`
/// keys and any spec that fails to parse or carries no slicer keys.
fn flat_overrides_from_specs(specs: &[OverrideFileSpec]) -> Vec<FlatOverrides> {
    specs
        .iter()
        .filter_map(|spec| {
            let flat = parse_override_str(&spec.content, Path::new(&spec.label)).ok()?;
            let entries: BTreeMap<String, String> = flat
                .entries
                .into_iter()
                .filter(|(k, _)| !k.starts_with("plugin."))
                .collect();
            (!entries.is_empty()).then(|| FlatOverrides {
                source: flat.source,
                entries,
            })
        })
        .collect()
}

/// Shared pre-flight: validate, resolve the cascade + context,
/// materialize the output dir, allocate the job, and register its
/// handle. Both the spawning and blocking entries build on this.
fn prepare_job(
    input: SliceJobInput,
    registry: &JobRegistry,
) -> Result<(JobId, ResolvedJob, Arc<JobHandle>), SliceStartError> {
    if input.plate_ids.is_empty() {
        return Err(SliceStartError::NoPlatesRequested);
    }
    let cascade = resolve_cascade(&input)?;
    let context = SlicingContext {
        printer: Arc::new(input.context.printer.clone()),
        plate: Arc::new(input.context.plate.clone()),
        filaments: input
            .context
            .filaments
            .clone()
            .into_iter()
            .map(Arc::new)
            .collect(),
        active_slot: input.context.active_slot,
    };
    // Snapshot the bound filament loadout from the instance for the
    // plugin hooks. `resolve_cascade` already proved the instance
    // resolves; re-looking-up here is cheap (bundled-library read) and
    // keeps the snapshot logic out of the cascade path. Empty on the
    // (now-unreachable) miss so plugins still run with no slots.
    let filament = lookup_instance(&input.printer_instance_id)
        .map(|inst| {
            FilamentLoadout::from_instance(
                &inst,
                input.context.printer.model.clone(),
                input.context.printer.toolheads.len(),
            )
        })
        .unwrap_or_default();
    let output_dir = PathBuf::from(&input.output_dir);
    // Materialize the output directory now so the worker can write
    // its first file without dancing around `mkdir -p`. If the path
    // is unusable the user finds out before we spawn the thread.
    std::fs::create_dir_all(&output_dir)
        .map_err(|e| SliceStartError::OutputDirInvalid(format!("{}: {e}", output_dir.display())))?;

    let job_id = registry.alloc_id();
    let handle = JobHandle::new();
    registry.insert(job_id, handle.clone());

    // Per-level plugin overrides for the dispatch gate. The plugin levels
    // are global / printer-instance / project / plate: printer-instance =
    // the bound instance's `config_overrides` (a per-printer default in the
    // user library), project = cascade *user* tier
    // (`Project.user_overrides`), plate = cascade *project* tier
    // (`Plate.project_overrides`). The object tier is not a plugin level.
    let plugin_instance = lookup_instance(&input.printer_instance_id)
        .map(|inst| {
            inst.config_overrides
                .iter()
                .filter(|(k, _)| k.starts_with("plugin."))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect()
        })
        .unwrap_or_default();
    let plugin_project = plugin_overrides_for_tier(&input.context.user_overrides);
    let plugin_plate = plugin_overrides_for_tier(&input.context.project_overrides);
    let override_tiers = override_tiers_from_context(&input.context);

    let resolved = ResolvedJob {
        objects: input.objects,
        output_dir,
        plate_ids: input.plate_ids,
        cascade,
        override_tiers,
        context,
        filament,
        plugin_instance,
        plugin_project,
        plugin_plate,
        paint_filament_remap: input.paint_filament_remap,
    };
    Ok((job_id, resolved, handle))
}

/// The production slice path entry: spawns the worker, capturing events
/// through `sink` and dispatching the post-slice hook through `host`.
/// The integration test under `src-tauri/tests/slice_orchestrator.rs`
/// uses the blocking variants below to capture events without a Tauri
/// runtime.
pub fn start_slice_job_with_sink_and_plugins(
    input: SliceJobInput,
    registry: &Arc<JobRegistry>,
    sink: EventSink,
    host: Option<PluginHostRef>,
) -> Result<JobId, SliceStartError> {
    spawn_worker(input, registry, sink, host)
}

/// How long a finished job's handle lingers in the registry before the
/// worker prunes it — long enough that a `slice_status` poll racing the
/// terminal event still resolves, short enough that handles don't pile up
/// across a long slicing session. The UI is event-driven, so this is
/// belt-and-suspenders, not the primary completion signal.
const JOB_RETENTION_AFTER_TERMINAL: Duration = Duration::from_secs(30);

fn spawn_worker(
    input: SliceJobInput,
    registry: &Arc<JobRegistry>,
    sink: EventSink,
    host: Option<PluginHostRef>,
) -> Result<JobId, SliceStartError> {
    let (job_id, resolved, handle) = prepare_job(input, registry)?;
    let sink = Arc::new(sink);
    let registry = Arc::clone(registry);
    thread::Builder::new()
        .name(format!("n3o-slice-{}", job_id.0))
        .spawn(move || {
            run_worker(job_id, resolved, sink, handle, host);
            // Prune the completed handle so the registry doesn't grow one
            // entry per slice for the process lifetime.
            std::thread::sleep(JOB_RETENTION_AFTER_TERMINAL);
            registry.remove(job_id);
        })
        .expect("spawn slice worker");
    Ok(job_id)
}

/// Synchronous variant for tests + tooling: runs the worker on the
/// calling thread instead of spawning. No plugin host.
pub fn run_slice_job_blocking(
    input: SliceJobInput,
    registry: &JobRegistry,
    sink: EventSink,
) -> Result<JobId, SliceStartError> {
    let (job_id, resolved, handle) = prepare_job(input, registry)?;
    run_worker(job_id, resolved, Arc::new(sink), handle, None);
    Ok(job_id)
}

/// Blocking variant with a plugin host — used by the post-slice
/// integration test to drive a real slice through a real plugin.
pub fn run_slice_job_blocking_with_plugins(
    input: SliceJobInput,
    registry: &JobRegistry,
    sink: EventSink,
    host: PluginHostRef,
) -> Result<JobId, SliceStartError> {
    let (job_id, resolved, handle) = prepare_job(input, registry)?;
    run_worker(job_id, resolved, Arc::new(sink), handle, Some(host));
    Ok(job_id)
}

/// Run the post-slice plugin hook over a plate's freshly-written
/// G-code, rewriting the file in place if any plugin changed it.
///
/// Skips all work (no parse, no read) when no host is wired or no
/// enabled plugin declares the hook. On the no-mutation path the
/// re-serialized bytes equal the original, so the file is left
/// untouched — the output stays byte-identical to libslic3r's.
fn apply_post_slice(
    host: &Option<PluginHostRef>,
    output_path: &Path,
    plate_id: u32,
    ctx: &SlicingContext,
    filament: &FilamentLoadout,
    gate: &DispatchGate,
) {
    let Some(host) = host else {
        return;
    };

    // Cheap gate, held only for the check: nothing to do unless an
    // active plugin (this printer + activated) declares the hook.
    if !lock_host(host).any_active_hook(HookKind::PostSlice, gate) {
        return;
    }

    // Read + parse OUTSIDE the host lock (only the Lua dispatch below
    // needs it), so plugin UI commands aren't blocked by the file I/O.
    //
    // Cost note: this reads the whole plate to a String, parses it to a
    // typed `Vec<Line>`, and re-serializes below — a full round-trip
    // even for a one-line edit, and `PostSliceHook` clones the lines
    // per plugin. Fine at MVP scale; revisit if large multi-material
    // jobs feel the transient allocation.
    let src = match std::fs::read_to_string(output_path) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(
                error = %e,
                path = %output_path.display(),
                "post-slice plugins skipped: G-code output is not readable UTF-8 text",
            );
            return;
        }
    };
    let hook = PostSliceHook {
        plate: PlateMeta {
            plate_id,
            printer_model: ctx.printer.model.clone(),
            bed_type: Some(ctx.plate.identity.clone()),
            // The orchestrator slices a model file and doesn't count
            // objects today; surface it as unknown rather than wrong.
            object_count: None,
        },
        filament: filament.clone(),
    };

    // Dispatch runs untrusted plugin Lua; hold the host lock only for
    // that. The lock is poison-tolerant so a panicking plugin can't
    // wedge the host for the rest of the process.
    let edited = lock_host(host).dispatch_gated(&hook, parse_str(&src), gate);

    let new_src = to_string(&edited);
    if new_src != src {
        // Atomic replace (write sibling temp + rename) so a failed or
        // partial write can't leave a truncated .gcode that the summary
        // and preview then read as if it were the finished slice. On
        // failure the original stays intact.
        if let Err(e) = crate::core::paths::atomic_write(output_path, new_src.as_bytes()) {
            tracing::warn!(
                error = %e,
                path = %output_path.display(),
                "post-slice: write failed; G-code left as sliced",
            );
        }
    }
}

/// Add the plate's [`SliceObject`]s to `model` in-memory (the default,
/// no-temp-file path). A solo object rides [`Model::add_object`] (world
/// transform on the instance); a multi-volume group (members sharing a
/// `GroupId`) becomes one [`Model::add_group`] + one [`Model::add_volume`] per
/// member (world transform on each volume). Build units emit in first-
/// appearance order, and a one-member group is a solo — mirroring the `.3mf`
/// writer's `Layout` so this path stays byte-identical to a `.3mf` round-trip.
fn build_model_objects(
    model: &mut Model,
    objects: &[super::input::SliceObject],
) -> std::result::Result<(), String> {
    // Borrow the per-triangle paint straight from the object's `Arc` — no clone.
    fn paint(o: &super::input::SliceObject) -> &[String] {
        o.paint.as_deref().map(|p| p.as_slice()).unwrap_or(&[])
    }
    fn support(o: &super::input::SliceObject) -> &[String] {
        o.support_paint
            .as_deref()
            .map(|p| p.as_slice())
            .unwrap_or(&[])
    }

    for unit in build_units(objects) {
        // A solo object with no connector volumes is one single-volume object
        // (matches the writer's Layout). A group, or a solo carrying cut
        // connectors, becomes one multi-volume ModelObject: the part(s) plus a
        // peg (MODEL_PART) / hole (NEGATIVE_VOLUME) volume per connector, which
        // libslic3r subtracts/fuses per-layer in 2D — no baked boolean.
        if let [i] = unit[..] {
            if objects[i].modifiers.is_empty() {
                let o = &objects[i];
                model
                    .add_object(
                        &o.name,
                        &o.vertices,
                        &o.indices,
                        &o.transform,
                        o.extruder,
                        paint(o),
                        support(o),
                        &o.overrides,
                    )
                    .map_err(|e| format!("add_object({}) failed: {e}", o.name))?;
                continue;
            }
        }
        let obj_idx = model
            .add_group(&objects[unit[0]].name, &objects[unit[0]].group_overrides)
            .map_err(|e| format!("add_group({}) failed: {e}", objects[unit[0]].name))?;
        for &i in &unit {
            let o = &objects[i];
            model
                .add_volume(
                    obj_idx,
                    &o.name,
                    &o.vertices,
                    &o.indices,
                    &o.transform,
                    o.extruder,
                    slic3r_ffi::VolumeType::Part,
                    paint(o),
                    support(o),
                    &o.overrides,
                )
                .map_err(|e| format!("add_volume({}) failed: {e}", o.name))?;
            for m in &o.modifiers {
                let vt = if m.negative {
                    slic3r_ffi::VolumeType::Negative
                } else {
                    slic3r_ffi::VolumeType::Part
                };
                model
                    .add_volume(
                        obj_idx,
                        &format!("{} connector", o.name),
                        &m.vertices,
                        &m.indices,
                        &o.transform,
                        o.extruder,
                        vt,
                        &[],
                        &[],
                        &[],
                    )
                    .map_err(|e| format!("add_volume(connector of {}) failed: {e}", o.name))?;
            }
        }
    }
    Ok(())
}

/// Bucket object indices into build units, preserving first-appearance order: a
/// solo unit per ungrouped object, one unit per `GroupId` (members in encounter
/// order). Mirrors the `.3mf` writer's `Layout` — a one-member group is just a
/// solo downstream. Pure over `o.group` so it's unit-testable without a `Model`.
fn build_units(objects: &[super::input::SliceObject]) -> Vec<Vec<usize>> {
    use crate::core::scene::state::GroupId;
    use std::collections::BTreeMap;

    let mut units: Vec<Vec<usize>> = Vec::new();
    let mut group_pos: BTreeMap<GroupId, usize> = BTreeMap::new();
    for (i, o) in objects.iter().enumerate() {
        match o.group {
            Some(g) => match group_pos.get(&g) {
                Some(&pos) => units[pos].push(i),
                None => {
                    group_pos.insert(g, units.len());
                    units.push(vec![i]);
                }
            },
            None => units.push(vec![i]),
        }
    }
    units
}

/// Run the pre-slice plugin hook over the resolved cascade, applying
/// any plugin edits back into `resolved` before the adapter + safety
/// gate run. No-op when no host is wired or no plugin declares the hook.
fn apply_pre_slice(
    host: &Option<PluginHostRef>,
    resolved: &mut Resolved,
    ctx: &SlicingContext,
    filament: &FilamentLoadout,
    gate: &DispatchGate,
) {
    let Some(host) = host else {
        return;
    };
    if !lock_host(host).any_active_hook(HookKind::PreSlice, gate) {
        return;
    }

    let settings: BTreeMap<String, String> = resolved
        .iter()
        .map(|(k, v)| (k.clone(), v.value.clone()))
        .collect();
    let hook = PreSliceHook {
        context: PreSliceContext {
            printer_model: ctx.printer.model.clone(),
            plate: ctx.plate.identity.clone(),
            toolhead_count: ctx.printer.toolheads.len(),
        },
        filament: filament.clone(),
    };
    // Dispatch (untrusted Lua) is the panic-prone step; it runs before
    // `resolved` is touched, so a panic leaves the cascade unchanged.
    let edited = lock_host(host).dispatch_gated(&hook, settings, gate);
    apply_pre_slice_result(resolved, edited);
}

/// Fold a plugin-edited settings map back into the resolved cascade:
/// update existing values and insert new keys (attributed to a
/// synthetic plugin source). Plugins can't remove keys (settings are
/// modify/add only), so nothing is dropped. The adapter only reads
/// key + value, so the synthetic trace is cosmetic.
fn apply_pre_slice_result(resolved: &mut Resolved, edited: BTreeMap<String, String>) {
    for (key, value) in edited {
        match resolved.get_mut(&key) {
            Some(rv) => rv.value = value,
            None => {
                resolved.insert(
                    key,
                    ResolvedValue {
                        value,
                        winning_rule: SourceLocation {
                            path: PathBuf::from("<plugin:pre_slice>"),
                            line: 0,
                        },
                        winning_specificity: 0,
                        matching_rules: Vec::new(),
                    },
                );
            }
        }
    }
}

/// Lock the plugin host, recovering the guard if a plugin panic
/// poisoned it (the buffer's edits are per-call, so recovery is safe).
fn lock_host(host: &PluginHostRef) -> std::sync::MutexGuard<'_, PluginHost> {
    host.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Collect the flat `plugin.*` override entries (both `.enabled` flags
/// and `.<key>` settings) for one cascade tier — one of the job's
/// override-spec lists. The host resolves each plugin's activation +
/// settings from these via [`crate::core::plugin::resolve`]; plugin keys
/// never reach the libslic3r adapter (which only knows real slicer
/// keys). Non-plugin keys are dropped here.
fn plugin_overrides_for_tier(specs: &[OverrideFileSpec]) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for spec in specs {
        if let Ok(flat) = parse_override_str(&spec.content, Path::new(&spec.label)) {
            for (k, v) in flat.entries {
                if k.starts_with("plugin.") {
                    out.insert(k, v);
                }
            }
        }
    }
    out
}

/// Sequential per-plate worker. Holds the `JobHandle` for cancel +
/// status updates. Emits every lifecycle event through `sink`.
fn run_worker(
    job_id: JobId,
    job: ResolvedJob,
    sink: Arc<EventSink>,
    handle: Arc<JobHandle>,
    host: Option<PluginHostRef>,
) {
    let mut last_plate_in_progress: Option<u32> = None;

    // The dispatch gate is the same for every plate in this job — one
    // bound printer model + one resolved activation set. Build it once
    // and hand it to both slice hooks; it gates which plugins run
    // (printer-compatible + activated, on top of host health).
    let gate = DispatchGate {
        printer_model: Some(job.context.printer.model.clone()),
        printer_instance: job.plugin_instance.clone(),
        project: job.plugin_project.clone(),
        plate: job.plugin_plate.clone(),
    };

    for &plate_id in &job.plate_ids {
        if handle.is_cancelled() {
            sink(SliceEvent::Cancelled {
                job_id,
                plate_id_in_progress: last_plate_in_progress,
            });
            return;
        }
        last_plate_in_progress = Some(plate_id);

        handle.set_status(JobStatus::Running {
            plate_id,
            percent: 0,
            stage: "Starting".into(),
        });
        sink(SliceEvent::PlateStarted { job_id, plate_id });

        // Resolve + adapt fresh per plate. Multi-plate projects
        // (Phase 5) may want per-plate cascade overrides; today the
        // context is the same per plate. Two-phase resolution: the authored
        // cascade, then the user/project/object override tiers on top
        // (`to_resolved` flattens to the effective value the hook + safety
        // gate + adapter consume; trace keeps the un-flattened map).
        let mut resolved_cascade = to_resolved(&resolve_with_overrides(
            &job.cascade,
            &job.override_tiers,
            &job.context,
        ));

        // Pre-slice plugin hook: let plugins read/modify the resolved
        // settings before the adapter + safety gate see them. Guarded
        // by catch_unwind for the same reason as post-slice — a panic
        // must not silently kill the worker thread.
        let pre = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            apply_pre_slice(
                &host,
                &mut resolved_cascade,
                &job.context,
                &job.filament,
                &gate,
            );
        }));
        if pre.is_err() {
            tracing::error!(
                plate_id,
                "pre-slice plugin hook panicked; using resolved settings"
            );
        }

        // Safety gate (cascade_safety.rs): refuses slice when the
        // resolved cascade is missing machine_start_gcode /
        // change_filament_gcode, has an empty acceleration envelope,
        // or asks for a nozzle temp above the printer's max. Catches
        // the demonstration-cascade class of failure before we feed
        // empty start-of-print to libslic3r + ship the result to a
        // real printer.
        if let Err(issues) = super::cascade_safety::validate_resolved_cascade(
            &resolved_cascade,
            &job.context.printer,
        ) {
            tracing::warn!(
                plate_id = plate_id,
                issue_count = issues.len(),
                "cascade safety gate refused slice",
            );
            let err = SliceError::UnsafeCascade { issues };
            fail(&handle, &sink, job_id, plate_id, err);
            return;
        }

        let adapt_result = match adapt(&resolved_cascade, &job.context) {
            Ok(ar) => ar,
            Err(e) => {
                let err = SliceError::Unknown {
                    raw_message: format!("adapter failed: {e}"),
                };
                fail(&handle, &sink, job_id, plate_id, err);
                return;
            }
        };

        // Build the libslic3r model from the plate's in-memory geometry:
        // `build_model_objects` hands each object's mesh buffers straight to
        // libslic3r (solos via `add_object`, multi-volume groups via `add_group`
        // + `add_volume`) — no temp file, no XML round-trip.
        let mut model = match Model::new() {
            Ok(m) => m,
            Err(e) => {
                let err = SliceError::Unknown {
                    raw_message: format!("Model::new failed: {e}"),
                };
                fail(&handle, &sink, job_id, plate_id, err);
                return;
            }
        };
        if let Err(raw_message) = build_model_objects(&mut model, &job.objects) {
            fail(
                &handle,
                &sink,
                job_id,
                plate_id,
                SliceError::Unknown { raw_message },
            );
            return;
        }
        // Toolchanger MMU paint routing: remap each painted filament
        // state to the libslic3r filament index its base material binds
        // to, so painted faces follow their material onto the right
        // toolhead (AMS printers pass `None` — identity is implicit).
        if let Some(perm) = &job.paint_filament_remap {
            if let Err(e) = model.remap_paint_filaments(perm) {
                let err = SliceError::Unknown {
                    raw_message: format!("paint filament remap failed: {e}"),
                };
                fail(&handle, &sink, job_id, plate_id, err);
                return;
            }
        }

        // Build the progress closure. Passed into slice() per-call
        // so no global state to clear afterwards.
        //
        // libslic3r overloads the progress callback to also carry
        // warnings: percent = -1 with the warning text in `stage`
        // (e.g. "It seems object X has floating regions..."). When the
        // slice subsequently aborts, the FFI surface only returns the
        // opaque string "Slice: Errors" — the real diagnosis is the
        // last warning. Capture it here so the failure path can
        // substitute it into `SliceError::Unknown.raw_message` instead
        // of the unhelpful cryptic summary.
        let sink_for_cb = sink.clone();
        let handle_for_cb = handle.clone();
        let mut throttle = ProgressThrottle::default();
        let last_warning: Arc<std::sync::Mutex<Option<String>>> =
            Arc::new(std::sync::Mutex::new(None));
        let last_warning_for_cb = last_warning.clone();
        let progress_cb = move |percent: i32, stage: &str| {
            handle_for_cb.set_status(JobStatus::Running {
                plate_id,
                percent,
                stage: stage.to_owned(),
            });
            if percent < 0 {
                if let Ok(mut guard) = last_warning_for_cb.lock() {
                    *guard = Some(stage.to_owned());
                }
            }
            if !throttle.should_emit(percent, stage) {
                return;
            }
            sink_for_cb(SliceEvent::PlateProgress {
                job_id,
                plate_id,
                percent,
                stage: stage.to_owned(),
            });
        };

        // Cancel requested during prepare (cascade resolve + Model build — all
        // before process() exists to abort): skip the slice rather than run it
        // to completion. Without this, a cancel in this window is swallowed (the
        // mid-process abort only catches cancels once process() is running).
        if handle.is_cancelled() {
            sink(SliceEvent::Cancelled {
                job_id,
                plate_id_in_progress: Some(plate_id),
            });
            return;
        }

        let output_path = job.output_dir.join(format!("plate_{plate_id}.gcode"));
        let outcome = slice_outcome(&model, &adapt_result.config, &output_path, progress_cb);

        // Surface advisory warnings ahead of the terminal event — whether
        // the slice then succeeds OR fails — so the error console shows them
        // either way.
        for message in outcome.warnings {
            sink(SliceEvent::PlateWarning {
                job_id,
                plate_id,
                message,
            });
        }

        match outcome.result {
            Ok(tower_mesh) => {
                // A cancel raced the slice to completion (requested after the
                // pre-slice check, but process() finished before aborting, or it
                // landed during the quick G-code export). Honor it — report
                // cancelled and drop the finished plate.
                if handle.is_cancelled() {
                    sink(SliceEvent::Cancelled {
                        job_id,
                        plate_id_in_progress: Some(plate_id),
                    });
                    return;
                }
                // Post-slice plugin hook: let plugins read/modify the
                // plate's G-code before the summary + preview see it.
                // No-op (and near-zero cost) when no host is wired or
                // no plugin declares the hook.
                //
                // Guarded by catch_unwind: a panic inside untrusted
                // plugin Lua must not unwind the worker thread (that
                // would silently lose the slice — no terminal event, UI
                // stuck "Running", temp file leaked). On a panic the
                // plate keeps libslic3r's unmodified G-code and the
                // slice completes normally.
                let post = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    apply_post_slice(
                        &host,
                        &output_path,
                        plate_id,
                        &job.context,
                        &job.filament,
                        &gate,
                    );
                }));
                if post.is_err() {
                    tracing::error!(
                        plate_id,
                        "post-slice plugin hook panicked; using unmodified G-code",
                    );
                }

                let summary = build_summary(&output_path).unwrap_or_else(|e| {
                    tracing::warn!(
                        error = %e,
                        path = %output_path.display(),
                        "could not build PlateSummary; emitting default",
                    );
                    super::PlateSummary {
                        output_path: output_path.clone(),
                        ..Default::default()
                    }
                });
                sink(SliceEvent::PlateFinished {
                    job_id,
                    plate_id,
                    output_path: output_path.display().to_string(),
                    summary,
                    tower_mesh: tower_mesh.map(|m| super::events::TowerMesh {
                        vertices: m.vertices,
                        indices: m.indices,
                    }),
                });
            }
            Err(e) => {
                // A cancel landed mid-slice: the cancel command flipped the flag
                // and aborted process() (slic3r_cancel → throw_if_canceled), so
                // the engine returned an error. Report it as cancelled, not a
                // failure.
                if handle.is_cancelled() {
                    sink(SliceEvent::Cancelled {
                        job_id,
                        plate_id_in_progress: Some(plate_id),
                    });
                    return;
                }
                let raw = format!("{e}");
                let mut err = classify_libslic3r_error(&raw);
                // The FFI surface returns an opaque "Slice: Errors"
                // (and similar) when libslic3r aborts mid-slice with a
                // diagnosis it only logged through the progress
                // callback. Swap in the last warning text so the UI
                // shows the actual reason instead of the unhelpful
                // summary.
                if let SliceError::Unknown { raw_message } = &mut err {
                    if let Ok(guard) = last_warning.lock() {
                        if let Some(warning) = guard.as_deref() {
                            tracing::info!(
                                raw = %raw_message,
                                warning,
                                "substituting libslic3r progress-warning into opaque slice error",
                            );
                            *raw_message = warning.to_owned();
                        }
                    }
                }
                fail(&handle, &sink, job_id, plate_id, err);
                return;
            }
        }
    }

    handle.set_status(JobStatus::Finished);
    sink(SliceEvent::JobFinished { job_id });
}

fn fail(
    handle: &Arc<JobHandle>,
    sink: &Arc<EventSink>,
    job_id: JobId,
    plate_id: u32,
    error: SliceError,
) {
    handle.set_status(JobStatus::Failed {
        plate_id,
        error: error.to_string(),
    });
    sink(SliceEvent::JobFailed {
        job_id,
        plate_id,
        error,
    });
}

/// Rate-limit progress event emission. Allows one event per
/// 50 ms per plate plus an immediate event whenever the stage
/// label changes (so the user sees phase transitions instantly).
/// Without this libslic3r's "Generating G-code: layer 247" ticks
/// would saturate the Tauri event channel.
#[derive(Default)]
struct ProgressThrottle {
    last_emit_at: Option<Instant>,
    last_stage: String,
}

const PROGRESS_MIN_INTERVAL: Duration = Duration::from_millis(50);

impl ProgressThrottle {
    fn should_emit(&mut self, _percent: i32, stage: &str) -> bool {
        let now = Instant::now();
        let stage_changed = stage != self.last_stage;
        let interval_ok = self
            .last_emit_at
            .map(|t| now.duration_since(t) >= PROGRESS_MIN_INTERVAL)
            .unwrap_or(true);
        if stage_changed || interval_ok {
            self.last_emit_at = Some(now);
            self.last_stage = stage.to_owned();
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A SliceObject with dummy geometry — `build_units` only reads `.group`.
    fn so(
        name: &str,
        group: Option<crate::core::scene::state::GroupId>,
    ) -> crate::core::slice::input::SliceObject {
        use std::sync::Arc;
        crate::core::slice::input::SliceObject {
            name: name.into(),
            vertices: Arc::new(vec![]),
            indices: Arc::new(vec![]),
            paint: None,
            support_paint: None,
            transform: [0.0; 16],
            extruder: 1,
            overrides: vec![],
            group,
            group_overrides: vec![],
            modifiers: vec![],
        }
    }

    #[test]
    fn build_units_buckets_groups_in_first_appearance_order() {
        use crate::core::scene::state::GroupId;
        let g1 = GroupId::fresh();
        let g2 = GroupId::fresh();
        // A(g1) B(solo) C(g1) D(g2) E(g2): g1 first appears before B, so its
        // unit holds that slot; non-contiguous members fold into it.
        let objs = vec![
            so("A", Some(g1)),
            so("B", None),
            so("C", Some(g1)),
            so("D", Some(g2)),
            so("E", Some(g2)),
        ];
        assert_eq!(build_units(&objs), vec![vec![0, 2], vec![1], vec![3, 4]]);
    }

    #[test]
    fn build_units_one_member_group_is_a_solo_unit() {
        use crate::core::scene::state::GroupId;
        let objs = vec![so("only", Some(GroupId::fresh()))];
        // Single-index unit → build_model_objects dispatches it via add_object.
        assert_eq!(build_units(&objs), vec![vec![0]]);
    }

    #[test]
    fn throttle_emits_first_tick_immediately() {
        let mut t = ProgressThrottle::default();
        assert!(t.should_emit(0, "Slicing"));
    }

    #[test]
    fn throttle_suppresses_dense_same_stage_ticks() {
        let mut t = ProgressThrottle::default();
        assert!(t.should_emit(0, "Slicing"));
        // Two more ticks within the 50 ms window with the same stage —
        // both suppressed.
        assert!(!t.should_emit(10, "Slicing"));
        assert!(!t.should_emit(20, "Slicing"));
    }

    #[test]
    fn throttle_emits_immediately_on_stage_change() {
        let mut t = ProgressThrottle::default();
        assert!(t.should_emit(0, "Slicing"));
        // Different stage within the 50 ms window — emits.
        assert!(t.should_emit(15, "Generating perimeters"));
    }

    #[test]
    fn throttle_emits_after_interval_elapses() {
        let mut t = ProgressThrottle::default();
        assert!(t.should_emit(0, "Slicing"));
        assert!(!t.should_emit(5, "Slicing"));
        // Pretend time has passed by reaching into the field. The
        // real path waits for the OS clock; this just exercises the
        // duration check.
        t.last_emit_at = Some(Instant::now() - Duration::from_millis(100));
        assert!(t.should_emit(10, "Slicing"));
    }

    #[test]
    fn plugin_overrides_for_tier_keeps_only_plugin_keys() {
        use crate::core::cascade::commands::OverrideFileSpec;
        let spec = |s: &str| OverrideFileSpec {
            label: "<test>".into(),
            content: s.into(),
        };
        // Dotted keys must be quoted in TOML to stay flat.
        let specs = vec![spec(
            "\"plugin.platecycler.enabled\" = false\n\
             \"plugin.platecycler.swap\" = \"M400\"\n\
             bed_temperature = 60",
        )];
        let flat = plugin_overrides_for_tier(&specs);
        assert_eq!(
            flat.get("plugin.platecycler.enabled").map(String::as_str),
            Some("false")
        );
        assert_eq!(
            flat.get("plugin.platecycler.swap").map(String::as_str),
            Some("M400")
        );
        assert_eq!(flat.get("bed_temperature"), None, "non-plugin keys dropped");
    }
}
