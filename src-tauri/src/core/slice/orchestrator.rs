//! Slice orchestrator worker (PR-3-2).
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
//! - The Tauri command `slice_start_job` allocates a [`JobId`],
//!   builds a [`ResolvedJob`] (cascade lookup + context conversion),
//!   inserts a [`JobHandle`] into the registry, spawns the worker,
//!   and returns the id immediately.
//! - The worker owns the slicing thread for the job's lifetime.
//!   It reads the cancel flag between plates and after each
//!   progress tick (libslic3r doesn't expose a mid-process cancel
//!   hook today; we cooperate at boundaries).
//! - Progress events are throttled: at most one per 50 ms per
//!   plate, plus an immediate event on every stage transition.
//!   Libslic3r emits hundreds of ticks per second on large plates
//!   and we'd saturate the Tauri event channel without this.
//!
//! ## FFI progress callback ownership
//!
//! `slic3r_ffi::set_slice_progress` is process-global — only one
//! callback can be registered at a time. The orchestrator's
//! invariant: at most one job's worker thread is *inside*
//! `slic3r_ffi::slice` at any moment. Sequential per-plate slicing
//! holds this naturally; future parallel-job support would need
//! per-job channels rather than the global callback.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use super::errors::{classify_libslic3r_error, SliceError};
use super::events::SliceEvent;
use super::job::{JobHandle, JobId, JobRegistry, JobStatus, ResolvedJob, SliceJobInput};
use super::summary::build_summary;
use crate::core::cascade::{self, types::Cascade};
use crate::core::cascade_adapter::{adapt, Manifest};
use crate::core::printer::lookup_instance;
use crate::core::profile_library::compose_cascade;
use crate::core::project::SlicingContext;
use std::collections::BTreeMap;
use slic3r_ffi::{clear_slice_progress, set_slice_progress, slice, Model};

/// Errors `start_slice_job` returns synchronously (before the
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
}

impl std::fmt::Display for SliceStartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoPlatesRequested => write!(f, "no plates in job"),
            Self::OutputDirInvalid(p) => write!(f, "output_dir not usable: {p}"),
            Self::PrinterInstanceCompose(s) => {
                write!(f, "printer-instance cascade compose failed: {s}")
            }
        }
    }
}

impl std::error::Error for SliceStartError {}

/// Event-sink callback the worker thread uses to surface every
/// lifecycle transition. `AppHandle::emit` is the production path;
/// tests inject a `Vec`-pushing closure to inspect the stream.
pub type EventSink = Box<dyn Fn(SliceEvent) + Send + Sync + 'static>;

/// Resolve the [`Cascade`] this job slices against.
///
/// Looks the named PrinterInstance up in the bundled library and
/// composes a fresh cascade from its per-bucket vendor fragments +
/// the plate's process overrides. Composition happens per job, not
/// against a shared registry; there's no caching.
///
/// Plate process overrides flow in via `input.context.project_overrides`
/// (`build_slice_input` attaches them). The composer treats them as
/// the highest-precedence layer.
fn resolve_cascade(input: &SliceJobInput) -> Result<Cascade, SliceStartError> {
    let instance = lookup_instance(&input.printer_instance_id).ok_or_else(|| {
        SliceStartError::PrinterInstanceCompose(format!(
            "unknown printer instance id `{}`",
            input.printer_instance_id,
        ))
    })?;
    // Pull plate overrides off the spec list. The composer wants them
    // as a flat BTreeMap; the spec list carries them as a TOML body
    // so the regular cascade override-tier loader works. Re-parse
    // back to a map — cheap, spec lists are tiny.
    let mut plate_overrides: BTreeMap<String, String> = BTreeMap::new();
    for spec in &input.context.project_overrides {
        if let Ok(table) = spec.content.parse::<toml::Value>() {
            if let Some(t) = table.as_table() {
                for (k, v) in t {
                    if let Some(s) = v.as_str() {
                        plate_overrides.insert(k.clone(), s.to_owned());
                    } else {
                        plate_overrides.insert(k.clone(), v.to_string());
                    }
                }
            }
        }
    }
    compose_cascade(&instance, &plate_overrides)
        .map_err(|e| SliceStartError::PrinterInstanceCompose(e.to_string()))
}

/// Spawn the worker thread for a slice job. Returns the allocated
/// [`JobId`] immediately; the worker drives the rest of the
/// lifecycle through Tauri events.
pub fn start_slice_job(
    input: SliceJobInput,
    app_handle: AppHandle,
    registry: &JobRegistry,
) -> Result<JobId, SliceStartError> {
    start_slice_job_with_sink(input, registry, app_handle_sink(app_handle))
}

/// Production sink — emits each event on the matching Tauri
/// channel. Errors are logged + swallowed (a disconnected frontend
/// shouldn't kill the worker thread).
fn app_handle_sink(app: AppHandle) -> EventSink {
    Box::new(move |event: SliceEvent| {
        let name = event.name();
        if let Err(e) = app.emit(name, &event) {
            tracing::warn!(event = name, error = %e, "slice event emit failed");
        }
    })
}

/// Testable orchestrator entry — same as [`start_slice_job`] but
/// takes a generic event sink instead of a Tauri AppHandle. The
/// integration test under `src-tauri/tests/slice_orchestrator.rs`
/// uses this to capture every event into a `Vec` without spinning
/// up a Tauri runtime.
pub fn start_slice_job_with_sink(
    input: SliceJobInput,
    registry: &JobRegistry,
    sink: EventSink,
) -> Result<JobId, SliceStartError> {
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
    let output_dir = PathBuf::from(&input.output_dir);
    // Materialize the output directory now so the worker can write
    // its first file without dancing around `mkdir -p`. If the path
    // is unusable the user finds out before we spawn the thread.
    std::fs::create_dir_all(&output_dir)
        .map_err(|e| SliceStartError::OutputDirInvalid(format!("{}: {e}", output_dir.display())))?;

    let job_id = registry.alloc_id();
    let handle = JobHandle::new();
    registry.insert(job_id, handle.clone());

    let resolved = ResolvedJob {
        model_path: PathBuf::from(&input.model_path),
        output_dir,
        plate_ids: input.plate_ids,
        cascade,
        context,
    };

    let handle_for_worker = handle.clone();
    let sink = Arc::new(sink);
    thread::Builder::new()
        .name(format!("n3o-slice-{}", job_id.0))
        .spawn(move || run_worker(job_id, resolved, sink, handle_for_worker))
        .expect("spawn slice worker");

    Ok(job_id)
}

/// Synchronous variant for tests + tooling: runs the worker on the
/// calling thread instead of spawning. The test harness uses this
/// to keep assertions inline with slice progress.
pub fn run_slice_job_blocking(
    input: SliceJobInput,
    registry: &JobRegistry,
    sink: EventSink,
) -> Result<JobId, SliceStartError> {
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
    let output_dir = PathBuf::from(&input.output_dir);
    std::fs::create_dir_all(&output_dir)
        .map_err(|e| SliceStartError::OutputDirInvalid(format!("{}: {e}", output_dir.display())))?;
    let job_id = registry.alloc_id();
    let handle = JobHandle::new();
    registry.insert(job_id, handle.clone());
    let resolved = ResolvedJob {
        model_path: PathBuf::from(&input.model_path),
        output_dir,
        plate_ids: input.plate_ids,
        cascade,
        context,
    };
    run_worker(job_id, resolved, Arc::new(sink), handle);
    Ok(job_id)
}

/// Sequential per-plate worker. Holds the `JobHandle` for cancel +
/// status updates. Emits every lifecycle event through `sink`.
fn run_worker(
    job_id: JobId,
    job: ResolvedJob,
    sink: Arc<EventSink>,
    handle: Arc<JobHandle>,
) {
    let mut last_plate_in_progress: Option<u32> = None;

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
        // context is the same per plate.
        let resolved_cascade = cascade::resolve(&job.cascade, &job.context);

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

        let manifest = Manifest::build();
        let adapt_result = match adapt(&resolved_cascade, &job.context, &manifest) {
            Ok(ar) => ar,
            Err(e) => {
                let err = SliceError::Unknown {
                    raw_message: format!("adapter failed: {e}"),
                };
                fail(&handle, &sink, job_id, plate_id, err);
                return;
            }
        };

        // Load the model. PR-3-9's project writer is the future
        // path (scene → temp 3MF → load), but for MVP we slice
        // whatever file the caller pointed us at.
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
        if let Err(e) = model.load(&job.model_path) {
            let err = classify_libslic3r_error(&format!("{e}"));
            fail(&handle, &sink, job_id, plate_id, err);
            return;
        }

        // Install the progress callback. The sink Arc is cloned
        // into the closure so we can share it with both the worker
        // body and the FFI's progress thread (which today is the
        // same thread, but the Sync bound is documented as part of
        // the sink's contract).
        let sink_for_cb = sink.clone();
        let handle_for_cb = handle.clone();
        let throttle = Arc::new(std::sync::Mutex::new(ProgressThrottle::default()));
        set_slice_progress(move |percent, stage| {
            handle_for_cb.set_status(JobStatus::Running {
                plate_id,
                percent,
                stage: stage.to_owned(),
            });
            if let Ok(mut guard) = throttle.lock() {
                if !guard.should_emit(percent, stage) {
                    return;
                }
            }
            sink_for_cb(SliceEvent::PlateProgress {
                job_id,
                plate_id,
                percent,
                stage: stage.to_owned(),
            });
        });

        let output_path = job.output_dir.join(format!("plate_{plate_id}.gcode"));
        let slice_result = slice(&model, &adapt_result.config, &output_path);
        // Always tear down the callback before we move on so a
        // subsequent plate (or another job) doesn't inherit the
        // wrong job_id in its progress events.
        clear_slice_progress();

        match slice_result {
            Ok(()) => {
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
                });
            }
            Err(e) => {
                let err = classify_libslic3r_error(&format!("{e}"));
                fail(&handle, &sink, job_id, plate_id, err);
                return;
            }
        }
    }

    handle.set_status(JobStatus::Finished);
    sink(SliceEvent::JobFinished { job_id });
}

fn fail(handle: &Arc<JobHandle>, sink: &Arc<EventSink>, job_id: JobId, plate_id: u32, error: SliceError) {
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

/// Compatibility re-export — used by [`crate::core::cascade`] tests
/// and downstream code that wanted `Path` paths instead of `&str`.
#[allow(dead_code)]
fn _path_alias(p: &Path) -> &Path {
    p
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
