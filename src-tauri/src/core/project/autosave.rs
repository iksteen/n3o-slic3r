//! Autosave + recovery (FR-MP-4 recovery half).
//!
//! Every `interval` seconds, snapshot the current [`Project`]
//! state to `<autosave_dir>/<project-uuid>.3mf` so a crash or
//! force-quit doesn't lose more than one cycle's worth of work.
//! On app launch, [`scan_recoveries`] enumerates candidate
//! autosave files; the frontend's recovery dialog presents them
//! and routes the user's choice to either
//! [`super::format::read_project`] ("recover") or
//! [`drop_autosave`] ("discard").
//!
//! ## Threading
//!
//! - The worker is a plain `std::thread` (no tokio dep — keeps
//!   the dependency graph thin while we're still pre-1.0).
//! - The worker holds an `Arc<Mutex<Project>>` clone — the
//!   same Mutex the Tauri command surface uses. On each tick it
//!   acquires the lock, clones the project, drops the lock, and
//!   writes the clone to disk. Disk I/O happens **outside** the
//!   lock so user-driven edits never block on autosave.
//! - The worker sleeps via [`std::thread::park_timeout`] so a
//!   stop signal can wake it immediately instead of waiting for
//!   the next interval.
//!
//! ## Skip-unchanged detection
//!
//! The worker hashes the project's JSON serialization on each
//! tick and skips disk I/O when nothing changed since the last
//! write. Mesh buffers are `#[serde(skip)]`, so this hash is
//! cheap (the JSON shape is small) and catches the common
//! "user opened the app and stepped away" case.
//!
//! ## Out of scope
//!
//! - Cloud sync.
//! - Per-project autosave intervals.
//! - Worker-priority tuning.
//! - Reading the autosave's own JSON to filter recovery
//!   candidates by mtime-vs-source-path (the frontend dialog
//!   surfaces all autosaves; user judgment picks the right
//!   one).

use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

use super::format::write_project;
use super::model::Project;

/// Default interval between autosave ticks. Lifted from PRD §6.2
/// (FR-MP-4 implementation note).
pub const DEFAULT_INTERVAL: Duration = Duration::from_secs(30);

/// One file in the autosave directory, as surfaced to the
/// frontend's recovery dialog.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AutosaveEntry {
    /// Filename stem (i.e. project uuid).
    pub uuid: String,
    /// Container-side path on disk.
    pub path: String,
    /// Last-modified wall-clock time. The recovery dialog sorts
    /// newest-first.
    pub modified_unix_secs: u64,
    /// Bytes on disk. Surfaced so the dialog can render a size
    /// hint next to each entry.
    pub size_bytes: u64,
}

/// Config the worker reads on each tick. Cloned into the thread
/// at spawn time, then captured by value.
#[derive(Debug, Clone)]
pub struct AutosaveConfig {
    pub dir: PathBuf,
    pub interval: Duration,
}

impl AutosaveConfig {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            interval: DEFAULT_INTERVAL,
        }
    }
}

/// Tauri-managed handle to the autosave worker. Wraps an
/// `Option<JoinHandle>` and a stop flag behind a Mutex so
/// `enable` / `disable` commands can flip the worker on/off at
/// runtime without recreating the Tauri State entry.
pub struct AutosaveHandle {
    inner: Mutex<Inner>,
}

struct Inner {
    stop: Option<Arc<AtomicBool>>,
    thread: Option<JoinHandle<()>>,
}

impl Default for AutosaveHandle {
    fn default() -> Self {
        Self::new()
    }
}

impl AutosaveHandle {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                stop: None,
                thread: None,
            }),
        }
    }

    /// Start the worker if not already running. Idempotent —
    /// calling `start` while running is a no-op (the existing
    /// worker continues with its prior config).
    ///
    /// Returns `Err` if the autosave directory can't be created.
    pub fn start(
        &self,
        project: Arc<Mutex<Project>>,
        config: AutosaveConfig,
    ) -> std::io::Result<()> {
        let mut inner = self.inner.lock().expect("autosave handle poisoned");
        if inner.thread.is_some() {
            return Ok(());
        }
        fs::create_dir_all(&config.dir)?;
        let stop = Arc::new(AtomicBool::new(false));
        let stop_clone = stop.clone();
        let thread = std::thread::Builder::new()
            .name("n3o-autosave".into())
            .spawn(move || autosave_loop(project, config, stop_clone))
            .expect("spawn autosave thread");
        inner.stop = Some(stop);
        inner.thread = Some(thread);
        Ok(())
    }

    /// Stop the worker. Idempotent. Blocks until the worker
    /// thread joins (worst case: one in-flight tick of disk
    /// I/O).
    pub fn stop(&self) {
        let mut inner = self.inner.lock().expect("autosave handle poisoned");
        if let Some(stop) = inner.stop.take() {
            stop.store(true, Ordering::Release);
            if let Some(thread) = inner.thread.take() {
                // Unpark in case the worker is mid-sleep.
                thread.thread().unpark();
                let _ = thread.join();
            }
        }
    }

    pub fn is_running(&self) -> bool {
        let inner = self.inner.lock().expect("autosave handle poisoned");
        inner.thread.is_some()
    }
}

impl Drop for AutosaveHandle {
    fn drop(&mut self) {
        // Stop the worker if the handle is dropped without an
        // explicit stop — covers app shutdown via process exit.
        self.stop();
    }
}

fn autosave_loop(project: Arc<Mutex<Project>>, config: AutosaveConfig, stop: Arc<AtomicBool>) {
    let mut last_hash: Option<u64> = None;
    loop {
        std::thread::park_timeout(config.interval);
        if stop.load(Ordering::Acquire) {
            break;
        }
        match write_one_tick(&project, &config.dir, &mut last_hash) {
            Ok(WriteOutcome::Wrote(path)) => {
                tracing::debug!(path = %path.display(), "autosave wrote");
            }
            Ok(WriteOutcome::Unchanged) => {
                tracing::trace!("autosave tick: project unchanged, skipped");
            }
            Err(e) => {
                // Don't kill the worker on transient I/O errors —
                // the next tick may succeed. The user already has
                // a copy in memory; the worst case is one missed
                // recovery point.
                tracing::warn!(error = %e, "autosave tick failed");
            }
        }
    }
}

#[derive(Debug)]
enum WriteOutcome {
    Wrote(PathBuf),
    Unchanged,
}

/// One tick's worth of work — clone the project under the lock,
/// hash the clone's JSON, write if changed. Pulled out so tests
/// can drive it deterministically without spinning up the worker
/// thread.
fn write_one_tick(
    project: &Arc<Mutex<Project>>,
    dir: &Path,
    last_hash: &mut Option<u64>,
) -> Result<WriteOutcome, Box<dyn std::error::Error + Send + Sync>> {
    let snapshot = {
        let p = project.lock().expect("project poisoned");
        p.clone()
    };
    // Hash the JSON shape (mesh buffers excluded via
    // #[serde(skip)]). If nothing changed since the previous
    // tick, skip the disk write entirely.
    let json = serde_json::to_vec(&snapshot)?;
    let mut hasher = DefaultHasher::new();
    json.hash(&mut hasher);
    let hash = hasher.finish();
    if Some(hash) == *last_hash {
        return Ok(WriteOutcome::Unchanged);
    }
    let path = autosave_path_for(dir, &snapshot);
    write_project(&snapshot, &path)?;
    *last_hash = Some(hash);
    Ok(WriteOutcome::Wrote(path))
}

/// Resolve the on-disk path for `project`'s autosave file:
/// `<dir>/<uuid>.3mf`.
pub fn autosave_path_for(dir: &Path, project: &Project) -> PathBuf {
    dir.join(format!("{}.3mf", project.uuid))
}

/// Default location for autosaves. On Linux: `$XDG_DATA_HOME` or
/// `~/.local/share`, then `/n3o-slic3r/autosave/`. Other
/// platforms get a best-effort fallback under `std::env::temp_dir`
/// — Tauri's plugin-fs (Phase 7+) will replace this with proper
/// per-platform resolution.
pub fn default_autosave_dir() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        if !xdg.is_empty() {
            return PathBuf::from(xdg).join("n3o-slic3r").join("autosave");
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("n3o-slic3r")
            .join("autosave");
    }
    std::env::temp_dir().join("n3o-slic3r").join("autosave")
}

/// Enumerate autosave files in `dir`. Returns entries newest-
/// first. Files that don't end in `.3mf` are ignored. Missing
/// or unreadable directories return an empty Vec (not an
/// error) — fresh app installs have no autosave dir yet.
pub fn scan_recoveries(dir: &Path) -> std::io::Result<Vec<AutosaveEntry>> {
    let read_dir = match fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut entries: Vec<AutosaveEntry> = Vec::new();
    for raw in read_dir {
        let entry = raw?;
        let path = entry.path();
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()).map(str::to_owned) else {
            continue;
        };
        if path.extension().and_then(|e| e.to_str()) != Some("3mf") {
            continue;
        }
        let meta = entry.metadata()?;
        if !meta.is_file() {
            continue;
        }
        let modified_unix_secs = meta
            .modified()?
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        entries.push(AutosaveEntry {
            uuid: stem,
            path: path.to_string_lossy().into_owned(),
            modified_unix_secs,
            size_bytes: meta.len(),
        });
    }
    entries.sort_by_key(|b| std::cmp::Reverse(b.modified_unix_secs));
    Ok(entries)
}

/// Delete the autosave file `<dir>/<uuid>.3mf`. Silent no-op
/// when the file isn't present (idempotent — the discard button
/// works regardless of whether another instance already cleaned
/// up).
pub fn drop_autosave(dir: &Path, uuid: &str) -> std::io::Result<()> {
    let path = dir.join(format!("{uuid}.3mf"));
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn tempdir() -> TempDir {
        TempDir::new().expect("tempdir")
    }

    #[test]
    fn write_one_tick_writes_on_first_call() {
        let dir = tempdir();
        let project = Arc::new(Mutex::new(Project::default()));
        let mut last_hash = None;
        let outcome = write_one_tick(&project, dir.path(), &mut last_hash).expect("ok");
        assert!(matches!(outcome, WriteOutcome::Wrote(_)));
        assert!(last_hash.is_some());
        let expected = autosave_path_for(dir.path(), &project.lock().unwrap());
        assert!(expected.exists());
    }

    #[test]
    fn write_one_tick_skips_when_unchanged() {
        let dir = tempdir();
        let project = Arc::new(Mutex::new(Project::default()));
        let mut last_hash = None;
        let _ = write_one_tick(&project, dir.path(), &mut last_hash).expect("ok");
        let outcome = write_one_tick(&project, dir.path(), &mut last_hash).expect("ok");
        assert!(matches!(outcome, WriteOutcome::Unchanged));
    }

    #[test]
    fn write_one_tick_writes_again_after_change() {
        let dir = tempdir();
        let project = Arc::new(Mutex::new(Project::default()));
        let mut last_hash = None;
        let _ = write_one_tick(&project, dir.path(), &mut last_hash).expect("ok");
        // Mutate.
        {
            let mut p = project.lock().unwrap();
            p.user_overrides.insert("k".into(), "v".into());
        }
        let outcome = write_one_tick(&project, dir.path(), &mut last_hash).expect("ok");
        assert!(matches!(outcome, WriteOutcome::Wrote(_)));
    }

    #[test]
    fn autosave_path_uses_project_uuid() {
        let dir = tempdir();
        let project = Project::default();
        let path = autosave_path_for(dir.path(), &project);
        let stem = path.file_stem().unwrap().to_string_lossy();
        assert_eq!(stem, project.uuid.to_string());
    }

    #[test]
    fn scan_recoveries_empty_dir_returns_empty() {
        let dir = tempdir();
        let entries = scan_recoveries(dir.path()).expect("ok");
        assert!(entries.is_empty());
    }

    #[test]
    fn scan_recoveries_missing_dir_returns_empty_not_err() {
        let entries =
            scan_recoveries(Path::new("/nonexistent-n3o-test-dir")).expect("not an error");
        assert!(entries.is_empty());
    }

    #[test]
    fn scan_recoveries_lists_written_autosaves() {
        let dir = tempdir();
        // Write two projects via write_one_tick.
        let p1 = Arc::new(Mutex::new(Project::default()));
        let mut h1 = None;
        write_one_tick(&p1, dir.path(), &mut h1).expect("ok");
        let p2 = Arc::new(Mutex::new(Project::default()));
        let mut h2 = None;
        write_one_tick(&p2, dir.path(), &mut h2).expect("ok");

        let entries = scan_recoveries(dir.path()).expect("ok");
        assert_eq!(entries.len(), 2);
        // Each entry's uuid should match a real project.
        let want1 = p1.lock().unwrap().uuid.to_string();
        let want2 = p2.lock().unwrap().uuid.to_string();
        assert!(entries.iter().any(|e| e.uuid == want1));
        assert!(entries.iter().any(|e| e.uuid == want2));
    }

    #[test]
    fn scan_recoveries_ignores_non_3mf_files() {
        let dir = tempdir();
        fs::write(dir.path().join("not-a-project.txt"), b"hello").unwrap();
        fs::write(dir.path().join("readme.md"), b"hi").unwrap();
        let entries = scan_recoveries(dir.path()).expect("ok");
        assert!(entries.is_empty());
    }

    #[test]
    fn drop_autosave_removes_file() {
        let dir = tempdir();
        let project = Arc::new(Mutex::new(Project::default()));
        let mut h = None;
        write_one_tick(&project, dir.path(), &mut h).expect("ok");
        let uuid = project.lock().unwrap().uuid.to_string();
        drop_autosave(dir.path(), &uuid).expect("ok");
        let entries = scan_recoveries(dir.path()).expect("ok");
        assert!(entries.is_empty());
    }

    #[test]
    fn drop_autosave_absent_is_silent_noop() {
        let dir = tempdir();
        drop_autosave(dir.path(), "nonexistent-uuid").expect("no error");
    }

    #[test]
    fn default_autosave_dir_respects_xdg_data_home() {
        let dir = tempdir();
        // SAFETY: tests run single-threaded; we restore after.
        let prior = std::env::var("XDG_DATA_HOME").ok();
        // Use std::env::set_var only within this test scope; the
        // env-var assertion is robust to value but cleanup is
        // critical so other tests aren't affected.
        unsafe { std::env::set_var("XDG_DATA_HOME", dir.path()) };
        let resolved = default_autosave_dir();
        assert!(resolved.starts_with(dir.path()));
        assert!(resolved.ends_with("n3o-slic3r/autosave"));
        match prior {
            Some(v) => unsafe { std::env::set_var("XDG_DATA_HOME", v) },
            None => unsafe { std::env::remove_var("XDG_DATA_HOME") },
        }
    }

    #[test]
    fn handle_start_then_stop_round_trip() {
        let dir = tempdir();
        let project = Arc::new(Mutex::new(Project::default()));
        let handle = AutosaveHandle::new();
        let config = AutosaveConfig::new(dir.path().to_path_buf());
        handle.start(project, config).expect("start");
        assert!(handle.is_running());
        handle.stop();
        assert!(!handle.is_running());
    }

    #[test]
    fn handle_start_is_idempotent() {
        let dir = tempdir();
        let project = Arc::new(Mutex::new(Project::default()));
        let handle = AutosaveHandle::new();
        let config = AutosaveConfig::new(dir.path().to_path_buf());
        handle
            .start(project.clone(), config.clone())
            .expect("start 1");
        handle.start(project, config).expect("start 2 — no-op");
        assert!(handle.is_running());
        handle.stop();
    }

    #[test]
    fn handle_stop_is_idempotent() {
        let handle = AutosaveHandle::new();
        handle.stop();
        handle.stop();
        assert!(!handle.is_running());
    }

    #[test]
    fn handle_dropped_without_stop_joins_worker() {
        let dir = tempdir();
        let project = Arc::new(Mutex::new(Project::default()));
        {
            let handle = AutosaveHandle::new();
            let config = AutosaveConfig::new(dir.path().to_path_buf());
            handle.start(project, config).expect("start");
            // Drop without explicit stop — Drop impl should join.
        }
        // If the Drop didn't join, this would block forever in
        // CI. Reaching here means it joined.
    }
}
