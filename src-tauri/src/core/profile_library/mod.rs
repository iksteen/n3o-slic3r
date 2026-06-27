//! Hierarchical vendor profile library, loaded from disk at startup.
//!
//! On-disk layout (root = `resources/profiles/`):
//!
//! ```text
//! <root>/<vendor>/
//! ├── printer/
//! │   └── <slug>/
//! │       ├── machine.toml             ← cascade fragment (machine globals)
//! │       ├── model.toml               ← PrinterProfile metadata (n3o shape)
//! │       ├── nozzles/<sku>.toml       ← per-extruder scalars
//! │       ├── beds/<name>.toml         ← thin metadata (identity, curr_bed_type)
//! │       └── processes/<slug>.toml    ← printer-bound process preset
//! └── filament/<slug>.toml
//! ```
//!
//! No `include_str!`. The vendor tree is loaded once into a process-
//! wide `OnceLock<ProfileLibrary>`; subsequent lookups borrow from
//! that cache. Tauri's `setup()` hook calls [`init_from`] with the
//! bundled-resources path so packaged builds find the profiles next
//! to the binary. Tests lazy-init from the workspace path via
//! `env!("CARGO_MANIFEST_DIR")`.
//!
//! Lookups are keyed by:
//! - printer cascade fragment: `<slug>` (file stem, e.g.
//!   `"bambu-lab-a1-mini"`).
//! - nozzle fragment: `(<printer_slug>, <sku>)` (sku = file stem).
//! - bed fragment: `(<printer_slug>, <identity>)` — identity is the
//!   libslic3r `curr_bed_type` enum value carried inside the file.
//! - process fragment: `(<printer_slug>, <slug>)` (process presets
//!   live under each printer because they're printer-tuned — e.g.
//!   layer-height sets, speeds, calibration).
//! - filament fragment: `<slug>` (file stem). Filaments are cross-
//!   printer.
//! - printer catalog entry: `<identity>` (declared inside
//!   `model.toml`, falls back to the directory name).

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::Deserialize;

use crate::core::cascade::loader::{parse_cascade_str, CascadeLoadError};
use crate::core::cascade::types::Cascade;
use crate::core::printer::profile::PrinterProfile;

pub mod composer;
pub use composer::{
    compose_cascade, join_for_key, resolve_base_scalars, split_for_key, with_quality_profile,
    ComposeError,
};

/// Errors emitted by [`ProfileLibrary::load`]. The Tauri setup hook
/// panics on failure — a packaged binary without a parseable profile
/// tree shouldn't have shipped — so the error type only needs to be
/// clearly describable in a panic message.
#[derive(Debug)]
pub enum LibraryError {
    MissingRoot(PathBuf),
    Io(PathBuf, std::io::Error),
    Toml(PathBuf, toml::de::Error),
    Cascade(PathBuf, CascadeLoadError),
    /// `machine.toml` exists for a printer but doesn't carry the
    /// `printer_model` scalar that drives every cascade
    /// `when.printer.model = …` predicate. Fail fast so a malformed
    /// import doesn't silently empty the process/filament filters.
    MissingPrinterModel {
        printer_dir: PathBuf,
    },
    /// A catalog printer's `machine.toml` doesn't carry the
    /// `default_bed_type` scalar that `PrinterProfile.default_bed`
    /// hydrates from. Fail fast so a malformed import doesn't
    /// silently fall back to the first supported plate as the
    /// instance default.
    MissingDefaultBedType {
        printer_dir: PathBuf,
    },
}

impl std::fmt::Display for LibraryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingRoot(p) => write!(f, "profile root `{}` does not exist", p.display()),
            Self::Io(p, e) => write!(f, "io error reading `{}`: {e}", p.display()),
            Self::Toml(p, e) => write!(f, "parse error in `{}`: {e}", p.display()),
            Self::Cascade(p, e) => write!(f, "cascade load error in `{}`: {e}", p.display()),
            Self::MissingPrinterModel { printer_dir } => write!(
                f,
                "`{}`: machine.toml does not declare a `printer_model` scalar — \
                 every cascade `when.printer.model = …` predicate keys off it",
                printer_dir.display(),
            ),
            Self::MissingDefaultBedType { printer_dir } => write!(
                f,
                "`{}`: machine.toml does not declare a `default_bed_type` scalar — \
                 `PrinterProfile.default_bed` hydrates from it; without it a new \
                 instance would silently default to the first supported plate",
                printer_dir.display(),
            ),
        }
    }
}

impl std::error::Error for LibraryError {}

/// One parsed cascade fragment + the workspace-relative path the
/// resolver embeds in `SourceLocation` for trace UI.
#[derive(Debug, Clone)]
struct CascadeAsset {
    cascade: Cascade,
    /// Workspace-relative source path string (only used for human-
    /// readable display; once a fragment is parsed we don't refer
    /// back to the file).
    source_path: String,
}

/// One printer catalog entry, parsed from `model.toml` next to the
/// printer's `machine.toml` cascade fragment.
#[derive(Debug, Clone)]
pub struct PrinterCatalogEntry {
    pub identity: String,
    pub profile: PrinterProfile,
    /// Fragment slug = directory name. Equal to identity when no
    /// `identity` override is declared in `model.toml`.
    pub fragment_slug: String,
}

/// One `(printer, nozzle)` pair a process fragment applies to,
/// derived at picker time from the fragment's `[[rule]]` predicates.
/// Each rule with `when.printer.model = …` + `when.nozzle.diameter = …`
/// contributes one entry per (model, nozzle) combination (the OR-list
/// form `when.printer.model = ["A", "B"]` expands).
///
/// Surfaced on the wire so a future UI can show "also fits …" hints;
/// the picker itself uses it to filter by the active installed-nozzle
/// set (union rule, with composite specs like `"0.4+0.6"` splitting
/// on `+`).
#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct ProcessAvailability {
    pub printer: String,
    pub nozzle: String,
}

/// Snapshot of every parseable profile fragment on disk, plus the
/// printer catalog. Built once at startup; lookups borrow from this.
pub struct ProfileLibrary {
    printer_fragments: HashMap<String, CascadeAsset>,
    nozzle_fragments: HashMap<(String, String), CascadeAsset>,
    bed_fragments: HashMap<(String, String), CascadeAsset>,
    filament_fragments: HashMap<String, CascadeAsset>,
    process_fragments: HashMap<(String, String), CascadeAsset>,

    /// Per-printer nozzle SKU declaration order (UI presentation).
    nozzle_order: BTreeMap<String, Vec<String>>,
    /// Per-printer bed identity declaration order (UI presentation).
    bed_order: BTreeMap<String, Vec<String>>,
    /// Per-printer process-slug declaration order (UI presentation).
    process_order: BTreeMap<String, Vec<String>>,
    /// Filament slug declaration order (UI presentation).
    filament_order: Vec<String>,

    /// Picker catalog — one entry per `<printer_dir>/model.toml`.
    catalog: Vec<PrinterCatalogEntry>,
}

static LIBRARY: OnceLock<ProfileLibrary> = OnceLock::new();

/// Explicit init. The Tauri runtime calls this from `setup()` with
/// the bundled-resources `profiles` path. Subsequent calls
/// are no-ops (`OnceLock` only initializes the first time).
pub fn init_from(root: PathBuf) {
    let _ = LIBRARY.get_or_init(|| {
        ProfileLibrary::load(&root).unwrap_or_else(|e| panic!("profile library load failed: {e}"))
    });
}

fn library() -> &'static ProfileLibrary {
    LIBRARY.get_or_init(|| {
        // Test/dev fallback: walk up from the manifest dir to find
        // `resources/profiles` in the workspace. A packaged binary
        // *must* explicitly call `init_from` before any lookup; if
        // it doesn't, this fallback will pick up a stale build-time
        // path that doesn't exist post-install and `load` will panic
        // with a clear "missing root" message.
        let resources_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("workspace root above manifest dir")
            .join("resources");
        let root = resources_root.join("profiles");
        ProfileLibrary::load(&root)
            .unwrap_or_else(|e| panic!("profile library load (workspace fallback) failed: {e}"))
    })
}

// ---- Walker --------------------------------------------------------

#[derive(Debug, Deserialize)]
struct PrinterProfileEnvelope {
    /// Catalog identity. Defaults to the directory name if omitted.
    #[serde(default)]
    identity: Option<String>,
    #[serde(flatten)]
    profile: PrinterProfile,
}

#[derive(Debug, Deserialize)]
struct BedIdentityProbe {
    identity: String,
}

impl ProfileLibrary {
    fn load(root: &Path) -> Result<Self, LibraryError> {
        if !root.exists() {
            return Err(LibraryError::MissingRoot(root.to_owned()));
        }

        let mut lib = ProfileLibrary {
            printer_fragments: HashMap::new(),
            nozzle_fragments: HashMap::new(),
            bed_fragments: HashMap::new(),
            filament_fragments: HashMap::new(),
            process_fragments: HashMap::new(),
            nozzle_order: BTreeMap::new(),
            bed_order: BTreeMap::new(),
            process_order: BTreeMap::new(),
            filament_order: Vec::new(),
            catalog: Vec::new(),
        };

        // Iterate vendors in stable sorted order so the catalog &
        // picker presentation are deterministic.
        for vendor_dir in read_sorted_subdirs(root)? {
            lib.load_vendor(root, &vendor_dir)?;
        }

        Ok(lib)
    }

    fn load_vendor(&mut self, root: &Path, vendor_dir: &Path) -> Result<(), LibraryError> {
        let printer_root = vendor_dir.join("printer");
        if printer_root.is_dir() {
            self.load_printers(root, &printer_root)?;
        }
        let filament_root = vendor_dir.join("filament");
        if filament_root.is_dir() {
            for f in read_sorted_files(&filament_root)? {
                let slug = file_stem(&f);
                let asset = read_cascade(root, &f)?;
                insert_fragment(
                    &mut self.filament_fragments,
                    slug.clone(),
                    asset,
                    "filament",
                );
                if !self.filament_order.contains(&slug) {
                    self.filament_order.push(slug);
                }
            }
        }
        Ok(())
    }

    fn load_printers(&mut self, root: &Path, printer_root: &Path) -> Result<(), LibraryError> {
        // Each printer lives in its own directory under <printer_root>:
        //   <slug>/machine.toml — cascade fragment (the bulk of the
        //                         imported libslic3r config).
        //   <slug>/model.toml   — PrinterProfile metadata (n3o shape).
        //   <slug>/nozzles/<sku>.toml
        //   <slug>/beds/<id>.toml
        //   <slug>/processes/<slug>.toml
        for printer_dir in read_sorted_subdirs(printer_root)? {
            let slug = printer_dir
                .file_name()
                .and_then(|n| n.to_str())
                .expect("directory name is valid utf-8")
                .to_owned();
            // machine.toml — cascade fragment (libslic3r config).
            let machine_path = printer_dir.join("machine.toml");
            if machine_path.is_file() {
                let asset = read_cascade(root, &machine_path)?;
                insert_fragment(&mut self.printer_fragments, slug.clone(), asset, "printer");
            }
            // model.toml — catalog metadata. `model` is no longer
            // authored here; we hydrate it from the machine cascade's
            // `printer_model` scalar (the single source of truth that
            // every `when.printer.model = …` predicate keys off).
            // Same for `Toolhead.hotend_type` — derived later in
            // `registry::hydrate_profile` from the per-nozzle profile.
            let meta_path = printer_dir.join("model.toml");
            if meta_path.is_file() {
                let raw = std::fs::read_to_string(&meta_path)
                    .map_err(|e| LibraryError::Io(meta_path.clone(), e))?;
                let mut envelope: PrinterProfileEnvelope =
                    toml::from_str(&raw).map_err(|e| LibraryError::Toml(meta_path.clone(), e))?;
                let machine_printer_model = self
                    .printer_fragments
                    .get(&slug)
                    .and_then(|a| fragment_set_value(&a.cascade, "printer_model"))
                    .ok_or_else(|| LibraryError::MissingPrinterModel {
                        printer_dir: printer_dir.clone(),
                    })?;
                envelope.profile.model = machine_printer_model;
                // `default_bed` is hydrated (in `registry::hydrate_profile`)
                // from the machine cascade's `default_bed_type` scalar. Fail
                // fast at load if it's absent rather than letting
                // `create_instance` silently seed the first supported plate.
                if self
                    .printer_fragments
                    .get(&slug)
                    .and_then(|a| fragment_set_value(&a.cascade, "default_bed_type"))
                    .is_none()
                {
                    return Err(LibraryError::MissingDefaultBedType {
                        printer_dir: printer_dir.clone(),
                    });
                }
                let identity = envelope.identity.unwrap_or_else(|| slug.clone());
                self.catalog.push(PrinterCatalogEntry {
                    identity,
                    profile: envelope.profile,
                    fragment_slug: slug.clone(),
                });
            }
            // nozzles/<sku>.toml
            let nozzles_dir = printer_dir.join("nozzles");
            if nozzles_dir.is_dir() {
                for f in read_sorted_files(&nozzles_dir)? {
                    let sku = file_stem(&f);
                    let asset = read_cascade(root, &f)?;
                    insert_fragment(
                        &mut self.nozzle_fragments,
                        (slug.clone(), sku.clone()),
                        asset,
                        "nozzle",
                    );
                    self.nozzle_order.entry(slug.clone()).or_default().push(sku);
                }
            }
            // processes/<slug>.toml — printer-bound process presets.
            let processes_dir = printer_dir.join("processes");
            if processes_dir.is_dir() {
                for f in read_sorted_files(&processes_dir)? {
                    let process_slug = file_stem(&f);
                    let asset = read_cascade(root, &f)?;
                    insert_fragment(
                        &mut self.process_fragments,
                        (slug.clone(), process_slug.clone()),
                        asset,
                        "process",
                    );
                    self.process_order
                        .entry(slug.clone())
                        .or_default()
                        .push(process_slug);
                }
            }
            // beds/<name>.toml — identity comes from inside the file.
            let beds_dir = printer_dir.join("beds");
            if beds_dir.is_dir() {
                for f in read_sorted_files(&beds_dir)? {
                    let raw =
                        std::fs::read_to_string(&f).map_err(|e| LibraryError::Io(f.clone(), e))?;
                    let probe: BedIdentityProbe =
                        toml::from_str(&raw).map_err(|e| LibraryError::Toml(f.clone(), e))?;
                    let identity = probe.identity;
                    let asset = parse_cascade(root, &f, &raw)?;
                    insert_fragment(
                        &mut self.bed_fragments,
                        (slug.clone(), identity.clone()),
                        asset,
                        "bed",
                    );
                    self.bed_order
                        .entry(slug.clone())
                        .or_default()
                        .push(identity);
                }
            }
        }
        Ok(())
    }
}

/// Insert a fragment into one of the library's HashMaps, warning on
/// collision. Both source paths land in the warning so a stale
/// resource-dir leftover (or a genuine same-slug collision across
/// vendors) is visible at startup rather than silently winning by
/// alphabetical order.
///
/// **Why:** Tauri copies `bundle.resources` into the target dir on
/// build and doesn't prune directories that have since vanished from
/// source. A stale `target/<profile>/profiles/<old-vendor>/`
/// leftover whose name sorts later than a current vendor's will
/// silently overwrite the right fragment — same-slug collisions are
/// load-bearing for slice correctness (plate-temp keys vanished into
/// libslic3r defaults for one such case; bed temp emitted as the
/// engine's 45 °C instead of the cascade-resolved value).
fn insert_fragment<K>(
    map: &mut HashMap<K, CascadeAsset>,
    key: K,
    asset: CascadeAsset,
    kind: &'static str,
) where
    K: Eq + std::hash::Hash + std::fmt::Debug,
{
    let new_source = asset.source_path.clone();
    if let Some(prior) = map.insert(key, asset) {
        tracing::warn!(
            kind = kind,
            prior_source = %prior.source_path,
            new_source = %new_source,
            "profile_library: {kind} fragment slug collision — later file silently \
             overwrites the earlier one. Check for stale leftovers in the resource dir \
             (`target/<profile>/profiles/`) or remove the duplicate from source.",
        );
    }
}

fn read_sorted_files(dir: &Path) -> Result<Vec<PathBuf>, LibraryError> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| LibraryError::Io(dir.to_owned(), e))?
        .filter_map(|entry| entry.ok())
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().and_then(|s| s.to_str()) == Some("toml"))
        .collect();
    out.sort();
    Ok(out)
}

fn read_sorted_subdirs(dir: &Path) -> Result<Vec<PathBuf>, LibraryError> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| LibraryError::Io(dir.to_owned(), e))?
        .filter_map(|entry| entry.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    out.sort();
    Ok(out)
}

fn file_stem(p: &Path) -> String {
    p.file_stem()
        .and_then(|s| s.to_str())
        .expect("toml file has a utf-8 stem")
        .to_owned()
}

fn read_cascade(root: &Path, path: &Path) -> Result<CascadeAsset, LibraryError> {
    let raw = std::fs::read_to_string(path).map_err(|e| LibraryError::Io(path.to_owned(), e))?;
    parse_cascade(root, path, &raw)
}

fn parse_cascade(root: &Path, path: &Path, raw: &str) -> Result<CascadeAsset, LibraryError> {
    // Trace UI uses this path verbatim; show it relative to the
    // vendor root so traces stay readable across machines.
    let relative = path
        .strip_prefix(root)
        .map(Path::to_path_buf)
        .unwrap_or_else(|_| path.to_path_buf());
    let rules =
        parse_cascade_str(raw, &relative).map_err(|e| LibraryError::Cascade(path.to_owned(), e))?;
    Ok(CascadeAsset {
        cascade: Cascade { rules },
        source_path: relative.display().to_string(),
    })
}

// ---- Public loaders (preserve the legacy free-function surface) ----

/// Load the printer cascade fragment for `slug`. Returns `None` if
/// unknown.
pub fn load_printer_fragment(slug: &str) -> Option<Cascade> {
    library()
        .printer_fragments
        .get(slug)
        .map(|a| a.cascade.clone())
}

/// Load the nozzle cascade fragment for `(printer_slug, sku)`. Both
/// strings must match the loaded library; returns `None` otherwise.
pub fn load_nozzle_fragment(printer_slug: &str, sku: &str) -> Option<Cascade> {
    library()
        .nozzle_fragments
        .get(&(printer_slug.to_owned(), sku.to_owned()))
        .map(|a| a.cascade.clone())
}

/// Load the bed cascade fragment for `(printer_slug, identity)`.
/// Identity is the libslic3r `curr_bed_type` enum value carried in
/// the bed.toml's own `identity` field.
pub fn load_bed_fragment(printer_slug: &str, identity: &str) -> Option<Cascade> {
    library()
        .bed_fragments
        .get(&(printer_slug.to_owned(), identity.to_owned()))
        .map(|a| a.cascade.clone())
}

/// Peek a single config key out of a printer fragment's cascade. The
/// machine fragment is a flat TOML in practice — every key is set by
/// some rule's `set` map — so the first hit wins. Returns `None` when
/// no rule sets `key`, or when the printer slug is unknown.
fn fragment_set_value(cascade: &Cascade, key: &str) -> Option<String> {
    for rule in &cascade.rules {
        if let Some(v) = rule.set.get(key) {
            return Some(v.clone());
        }
    }
    None
}

/// The machine cascade's `default_bed_type` scalar — libslic3r's
/// documented home for the printer's picker-default bed
/// (PrintConfig.cpp:1072 registers it as a `coString` config key;
/// Preset::get_default_bed_type reads it off the resolved
/// `DynamicPrintConfig`). The registry hydrates
/// `PrinterProfile.default_bed` from this so model.toml doesn't
/// duplicate the value.
pub fn default_bed_type_for(printer_slug: &str) -> Option<String> {
    let cascade = load_printer_fragment(printer_slug)?;
    fragment_set_value(&cascade, "default_bed_type")
}

/// Parse libslic3r's `printable_area` + `printable_height` for the
/// printer into a 3-D AABB. `printable_area` is a BBS-style polygon
/// (`"x1xy1,x2xy2,..."` corner list) — we take the axis-aligned XY
/// bounds. Returns `None` when either key is missing or unparseable.
pub fn build_volume_for_printer(
    printer_slug: &str,
) -> Option<crate::core::printer::profile::BoundingBox> {
    let cascade = load_printer_fragment(printer_slug)?;
    let area = fragment_set_value(&cascade, "printable_area")?;
    let height = fragment_set_value(&cascade, "printable_height")?;
    parse_build_volume(&area, &height)
}

fn parse_build_volume(
    area: &str,
    height: &str,
) -> Option<crate::core::printer::profile::BoundingBox> {
    let max_z: f64 = height.trim().parse().ok()?;
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    let mut saw_corner = false;
    for corner in area.split(',') {
        let (x, y) = corner.trim().split_once('x')?;
        let x: f64 = x.trim().parse().ok()?;
        let y: f64 = y.trim().parse().ok()?;
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
        saw_corner = true;
    }
    if !saw_corner {
        return None;
    }
    Some(crate::core::printer::profile::BoundingBox {
        min: [min_x, min_y, 0.0],
        max: [max_x, max_y, max_z],
    })
}

/// libslic3r `default_filament_profile` for the given printer + nozzle
/// SKU — the filament preset Orca picks when a fresh instance lands.
/// Returns the preset's `filament_settings_id` string (e.g.
/// `"Bambu PLA Basic @BBL A1M"`); callers map this to a fragment slug
/// via [`filament_slug_by_display_name`].
pub fn default_filament_profile_for(printer_slug: &str, sku: &str) -> Option<String> {
    let cascade = load_nozzle_fragment(printer_slug, sku)?;
    fragment_set_value(&cascade, "default_filament_profile")
}

/// Look up a filament fragment slug by its human-readable
/// `filament_settings_id` (the same name libslic3r's
/// `default_filament_profile` uses). Returns `None` when no fragment
/// matches — callers should fall back to a generic filament slug.
pub fn filament_slug_by_display_name(display_name: &str) -> Option<String> {
    list_filament_fragments()
        .into_iter()
        .find(|f| f.display_name == display_name)
        .map(|f| f.identity)
}

/// Every bed identity bundled for the named printer, in declaration
/// order (file-system sort order, deterministic).
pub fn bundled_beds_for_printer(printer_slug: &str) -> Vec<&'static str> {
    library()
        .bed_order
        .get(printer_slug)
        .map(|v| v.iter().map(String::as_str).collect())
        .unwrap_or_default()
}

/// All nozzle SKUs bundled for the named printer.
pub fn nozzle_skus_for(printer_slug: &str) -> Vec<&'static str> {
    library()
        .nozzle_order
        .get(printer_slug)
        .map(|v| v.iter().map(String::as_str).collect())
        .unwrap_or_default()
}

/// Load the filament cascade fragment for `slug`.
pub fn load_filament_fragment(slug: &str) -> Option<Cascade> {
    library()
        .filament_fragments
        .get(slug)
        .map(|a| a.cascade.clone())
}

/// Load the process cascade fragment for `(printer_slug, process_slug)`.
/// Process presets are printer-bound (each lives under
/// `printer/<slug>/processes/<process_slug>.toml`); a process slug
/// alone is ambiguous if two printers reuse a name.
pub fn load_process_fragment(printer_slug: &str, process_slug: &str) -> Option<Cascade> {
    library()
        .process_fragments
        .get(&(printer_slug.to_owned(), process_slug.to_owned()))
        .map(|a| a.cascade.clone())
}

/// Every bundled process slug for `printer_slug`, in declaration
/// order. First entry is a reasonable default when seeding a
/// fresh PrinterInstance.
pub fn bundled_process_slugs_for_printer(printer_slug: &str) -> Vec<&'static str> {
    // Use `process_order` (declaration-ordered) rather than the
    // `process_fragments` HashMap keys, whose iteration order is
    // non-deterministic — the first entry seeds a fresh instance's
    // default process, which must be stable run-to-run.
    library()
        .process_order
        .get(printer_slug)
        .map(|order| order.iter().map(String::as_str).collect())
        .unwrap_or_default()
}

/// One bundled vendor filament's identity + display label, surfaced
/// to the frontend slot-binding panel. `identity` is the slug
/// (matches the wire form stored in `SlotBinding.filament_identity`);
/// `display_name` is the `filament_settings_id` field a human will
/// recognize ("Bambu PLA Basic @BBL A1M"); `base_type` drives the
/// material tag in the picker. `vendor` groups products under a
/// brand rail; `nozzle_temp` / `bed_temp` seed the per-product meta
/// row in the filament picker. `filament_id` is the vendor SKU
/// stamped into the fragment (e.g. "GFA00" for Bambu PLA Basic) —
/// driver-side sync matches AMS-reported `tray_info_idx`
/// against this to resolve a tray to a bundled fragment.
#[derive(Debug, Clone, serde::Serialize)]
pub struct FilamentFragmentSummary {
    pub identity: String,
    pub display_name: String,
    pub base_type: String,
    pub vendor: String,
    pub nozzle_temp: u32,
    pub bed_temp: u32,
    pub filament_id: Option<String>,
    /// True when the user has edited this filament in place (a non-empty
    /// override profile exists for its slug). The picker shows a Revert
    /// affordance for edited filaments. Filled in by `filament_profile_list`
    /// (this layer can't see the user library); `false` here.
    #[serde(default)]
    pub edited: bool,
}

/// Enumerate every bundled vendor filament fragment. Parses the
/// `filament_settings_id` + `filament_type` + `filament_vendor` +
/// temperature fields out of each fragment (stamped by the vendor
/// converter, stable across regens).
pub fn list_filament_fragments() -> Vec<FilamentFragmentSummary> {
    library()
        .filament_order
        .iter()
        .filter_map(|slug| {
            let cascade = library()
                .filament_fragments
                .get(slug)
                .expect("filament_order index always present in fragments map");
            // The cascade.rules vec carries one unconditional rule
            // for converter output; read the surfaced fields out of
            // its set. A fragment with no rules (e.g. metadata-only
            // file that all the picker scalars happened to skip) is
            // skipped here rather than panicking the picker IPC.
            let set = match cascade.cascade.rules.first() {
                Some(r) => &r.set,
                None => {
                    tracing::warn!(
                        slug = %slug,
                        "filament fragment carries no rules; skipping in picker list",
                    );
                    return None;
                }
            };
            let display_name = set
                .get("filament_settings_id")
                .cloned()
                .unwrap_or_else(|| slug.clone());
            let base_type = set
                .get("filament_type")
                .cloned()
                .unwrap_or_else(|| "PLA".to_owned());
            let vendor = set
                .get("filament_vendor")
                .cloned()
                .unwrap_or_else(|| "Generic".to_owned());
            let nozzle_temp = set
                .get("nozzle_temperature")
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(210);
            // BBS fragments expose per-bed-type temps (hot_plate_temp,
            // textured_plate_temp, etc.) but no single "bed_temp"
            // field. The picker only wants a representative value, so
            // prefer the generic `hot_plate_temp` (PEI / smooth) and
            // fall back through textured / cool / supertack.
            let bed_temp = [
                "hot_plate_temp",
                "textured_plate_temp",
                "cool_plate_temp",
                "supertack_plate_temp",
            ]
            .iter()
            .find_map(|k| set.get(*k).and_then(|s| s.parse::<u32>().ok()))
            .unwrap_or(60);
            // Vendor SKU — stamped by the converter for fragments
            // that have one (Bambu / Generic carry it; bespoke
            // user-imported profiles may not). Empty strings are
            // treated as absent.
            let filament_id = set
                .get("filament_id")
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_owned());
            Some(FilamentFragmentSummary {
                identity: slug.clone(),
                display_name,
                base_type,
                vendor,
                nozzle_temp,
                bed_temp,
                filament_id,
                edited: false,
            })
        })
        .collect()
}

/// The `(min, max)` nozzle temperature range a filament fragment
/// declares, for writing back to a Bambu AMS tray. Reads the BBS
/// `nozzle_temperature_range_low` / `_high` scalars, falling back to
/// the single `nozzle_temperature` for either bound when a range
/// field is absent. `None` if the fragment is unknown or carries no
/// temperature at all.
pub fn filament_nozzle_range(identity: &str) -> Option<(u32, u32)> {
    let asset = library().filament_fragments.get(identity)?;
    let set = &asset.cascade.rules.first()?.set;
    let one = set
        .get("nozzle_temperature")
        .and_then(|s| s.parse::<u32>().ok());
    let lo = set
        .get("nozzle_temperature_range_low")
        .and_then(|s| s.parse::<u32>().ok())
        .or(one)?;
    let hi = set
        .get("nozzle_temperature_range_high")
        .and_then(|s| s.parse::<u32>().ok())
        .or(one)?;
    Some((lo, hi))
}

/// The `default_process_profile` slug declared in a nozzle.toml
/// fragment, if any. Drives the Quality picker's rule-1 default —
/// each nozzle profile registers its preferred process; the picker
/// uses that when seeding a fresh instance and when the user's
/// current process becomes incompatible after a nozzle swap.
///
/// Read from the nozzle cascade's unconditional default rule
/// (cascade load already places top-level scalars there). Returns
/// `None` when the nozzle fragment is unknown or doesn't declare
/// a default.
pub fn nozzle_default_process(printer_slug: &str, sku: &str) -> Option<String> {
    library()
        .nozzle_fragments
        .get(&(printer_slug.to_owned(), sku.to_owned()))
        .and_then(|asset| asset.cascade.rules.iter().find(|r| r.is_default()))
        .and_then(|rule| rule.set.get("default_process_profile"))
        .cloned()
}

/// The `nozzle_type` scalar from a nozzle.toml fragment — the
/// hotend material descriptor (`"stainless_steel"`,
/// `"hardened_steel"`, …). Used by `registry::hydrate_profile` to
/// populate `Toolhead.hotend_type` so model.toml doesn't duplicate
/// the same string the nozzle SKU profile already carries.
pub fn nozzle_type_for(printer_slug: &str, sku: &str) -> Option<String> {
    library()
        .nozzle_fragments
        .get(&(printer_slug.to_owned(), sku.to_owned()))
        .and_then(|asset| asset.cascade.rules.iter().find(|r| r.is_default()))
        .and_then(|rule| rule.set.get("nozzle_type"))
        .cloned()
}

/// One row in the Quality picker's dropdown for the active
/// (printer, nozzle). `slug` is the wire identity the frontend
/// writes back via `printer_instance_set_quality_profile`;
/// `display_name` and `layer_height_mm` are picker presentation;
/// `available_for` carries the full set of (printer, nozzle) combos
/// the fragment supports so a future UI can show "also fits …" hints.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ProcessFragmentSummary {
    pub slug: String,
    pub display_name: String,
    pub layer_height_mm: Option<f32>,
    pub available_for: Vec<ProcessAvailability>,
}

/// Enumerate process fragments available for the active
/// (printer, set-of-installed-nozzles). `printer_fragment_slug` is
/// the printer directory slug (e.g. `"bambu-lab-a1-mini"`);
/// `printer_model` is the human printer name from `machine.toml`
/// (e.g. `"Bambu Lab A1 mini"`) — the metadata's `available_for`
/// rows key off the latter, while the on-disk fragments live under
/// the former.
///
/// `installed_nozzle_diameters` lists the unique nozzle diameters
/// currently installed across the printer's extruders (e.g.
/// `["0.4"]` for an A1 mini, `["0.4", "0.6"]` for a mixed-nozzle
/// U1). A fragment matches when any nozzle in its `available_for`
/// entry (split on `+` for composite specs like `"0.4+0.6"`) shares
/// at least one diameter with the installed set. Composite profiles
/// surface alongside single-nozzle ones whenever any of their
/// constituent nozzles is installed.
pub fn list_process_fragments(
    printer_fragment_slug: &str,
    printer_model: &str,
    installed_nozzle_diameters: &[String],
) -> Vec<ProcessFragmentSummary> {
    let lib = library();
    let order = match lib.process_order.get(printer_fragment_slug) {
        Some(order) => order,
        None => return Vec::new(),
    };
    let installed: std::collections::HashSet<&str> = installed_nozzle_diameters
        .iter()
        .map(String::as_str)
        .collect();
    order
        .iter()
        .filter_map(|process_slug| {
            let asset = lib
                .process_fragments
                .get(&(printer_fragment_slug.to_owned(), process_slug.clone()))?;
            let available_for = derive_process_availability(&asset.cascade);
            let matches = available_for.iter().any(|a| {
                a.printer == printer_model && a.nozzle.split('+').any(|n| installed.contains(n))
            });
            if !matches {
                return None;
            }
            let default_rule = asset.cascade.rules.iter().find(|r| r.is_default());
            let display_name = default_rule
                .and_then(|r| r.set.get("print_settings_id"))
                .cloned()
                .unwrap_or_else(|| process_slug.clone());
            let layer_height_mm = default_rule
                .and_then(|r| r.set.get("layer_height"))
                .and_then(|s| s.parse::<f32>().ok());
            Some(ProcessFragmentSummary {
                slug: process_slug.clone(),
                display_name,
                layer_height_mm,
                available_for,
            })
        })
        .collect()
}

/// Walk a process fragment's `[[rule]]` blocks and collect every
/// `(printer.model, nozzle.diameter)` combination they target. OR-list
/// predicates (`when.printer.model = ["A", "B"]`) expand into one
/// availability entry per printer; rules without both dimensions are
/// skipped (a fragment without a printer.model predicate isn't a
/// printer-bound process and can't surface for any printer).
fn derive_process_availability(cascade: &Cascade) -> Vec<ProcessAvailability> {
    use crate::core::cascade::types::ConditionValue;
    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    for rule in &cascade.rules {
        let mut printers: Vec<&str> = Vec::new();
        let mut nozzle: Option<&str> = None;
        for cond in &rule.when.conditions {
            match cond.dimension.as_str() {
                "printer.model" => match &cond.value {
                    ConditionValue::Scalar(s) => printers.push(s.as_str()),
                    ConditionValue::Array(xs) => printers.extend(xs.iter().map(String::as_str)),
                },
                "nozzle.diameter" => {
                    if let ConditionValue::Scalar(s) = &cond.value {
                        nozzle = Some(s.as_str());
                    }
                }
                _ => {}
            }
        }
        let Some(n) = nozzle else { continue };
        for p in printers {
            if seen.insert((p.to_owned(), n.to_owned())) {
                out.push(ProcessAvailability {
                    printer: p.to_owned(),
                    nozzle: n.to_owned(),
                });
            }
        }
    }
    out
}

/// Printer catalog — one entry per `model.toml` found on disk.
/// `core::printer::registry` re-exposes this for the picker.
pub fn printer_catalog() -> &'static [PrinterCatalogEntry] {
    &library().catalog
}

/// Look up a printer catalog entry by its identity slug.
pub fn printer_catalog_lookup(identity: &str) -> Option<&'static PrinterCatalogEntry> {
    library().catalog.iter().find(|e| e.identity == identity)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_process_fragments_returns_a1m_04_set() {
        // The A1 mini + 0.4 nozzle pairing should match every
        // bundled process whose `[meta] available_for` contains
        // (Bambu Lab A1 mini, 0.4). Upstream BBL ships 10 such
        // leaves; the consolidator preserves them as named slugs
        // with the same available_for, so this list is stable.
        let summaries = list_process_fragments(
            "bambu-lab-a1-mini",
            "Bambu Lab A1 mini",
            &["0.4".to_string()],
        );
        assert_eq!(
            summaries.len(),
            10,
            "expected 10 process fragments for A1 mini + 0.4 nozzle, got {} ({:?})",
            summaries.len(),
            summaries.iter().map(|s| &s.slug).collect::<Vec<_>>(),
        );
        assert!(summaries.iter().any(|s| s.slug == "0.20mm-standard"));
        assert!(summaries.iter().any(|s| s.slug == "0.20mm-strength"));
    }

    #[test]
    fn list_process_fragments_unions_multi_nozzle_u1() {
        // U1 mixed-nozzle setup ([0.4, 0.6]) — the picker should
        // surface every process whose `available_for` includes any
        // installed nozzle (union rule). That covers single-nozzle
        // "0.4" and "0.6" processes PLUS upstream's composite
        // "0.4+0.6" profile (the filter splits on `+`).
        let summaries = list_process_fragments(
            "snapmaker-u1",
            "Snapmaker U1",
            &["0.4".to_string(), "0.6".to_string()],
        );
        let slugs: Vec<&str> = summaries.iter().map(|s| s.slug.as_str()).collect();
        assert!(
            slugs.contains(&"0.20-standard"),
            "U1 mixed should see 0.20-standard (covers single 0.4, \
             single 0.6, and composite 0.4+0.6 contexts); got {slugs:?}",
        );
        let has_06_anywhere = summaries.iter().any(|s| {
            s.available_for
                .iter()
                .any(|a| a.printer == "Snapmaker U1" && a.nozzle.split('+').any(|n| n == "0.6"))
        });
        assert!(
            has_06_anywhere,
            "U1 mixed should include at least one fragment whose \
             available_for mentions 0.6 nozzle; got {slugs:?}",
        );
    }

    #[test]
    fn list_process_fragments_filters_by_active_nozzle_set() {
        // Single-nozzle setup: A1 mini with just 0.4 only sees
        // fragments whose `available_for` for this printer mentions
        // 0.4 somewhere (either as a single-nozzle "0.4" or as a
        // constituent of a composite like "0.4+x"). Fragments
        // targeting other nozzles only (e.g. "0.6", "0.8") must not
        // surface.
        let summaries = list_process_fragments(
            "bambu-lab-a1-mini",
            "Bambu Lab A1 mini",
            &["0.4".to_string()],
        );
        for s in &summaries {
            let has_04 = s.available_for.iter().any(|a| {
                a.printer == "Bambu Lab A1 mini" && a.nozzle.split('+').any(|n| n == "0.4")
            });
            assert!(
                has_04,
                "fragment `{}` surfaced for A1 mini 0.4 but its \
                 available_for never mentions 0.4: {:?}",
                s.slug, s.available_for,
            );
        }
    }

    #[test]
    fn every_bundled_fragment_parses() {
        // Smoke: walk the library and ensure each registered slug
        // resolves back through the public loaders.
        for slug in library().printer_fragments.keys() {
            assert!(load_printer_fragment(slug).is_some(), "printer {slug}");
        }
        for (printer, sku) in library().nozzle_fragments.keys() {
            assert!(
                load_nozzle_fragment(printer, sku).is_some(),
                "nozzle {printer}/{sku}"
            );
        }
        for (printer, identity) in library().bed_fragments.keys() {
            assert!(
                load_bed_fragment(printer, identity).is_some(),
                "bed {printer}/{identity}"
            );
        }
        for slug in library().filament_fragments.keys() {
            assert!(load_filament_fragment(slug).is_some(), "filament {slug}");
        }
        for (printer, slug) in library().process_fragments.keys() {
            assert!(
                load_process_fragment(printer, slug).is_some(),
                "process {printer}/{slug}",
            );
        }
    }

    #[test]
    fn bambi_printer_fragment_carries_machine_envelope_not_nozzle_keys() {
        let cascade = load_printer_fragment("bambu-lab-a1-mini").expect("bambi printer");
        let rule = &cascade.rules[0];
        assert!(rule.set.contains_key("printable_height"));
        assert!(rule.set.contains_key("machine_max_acceleration_x"));
        assert!(
            !rule.set.contains_key("nozzle_diameter"),
            "nozzle_diameter must NOT be in machine.toml (lives in nozzles/<sku>.toml)",
        );
    }

    #[test]
    fn a1_mini_0_4_nozzle_carries_scalar_diameter() {
        let cascade = load_nozzle_fragment("bambu-lab-a1-mini", "0.4").expect("0.4 nozzle");
        let rule = &cascade.rules[0];
        let diameter = rule
            .set
            .get("nozzle_diameter")
            .expect("nozzle_diameter present");
        assert_eq!(diameter, "0.4");
    }

    #[test]
    fn u1_0_4_nozzle_is_also_scalar_despite_4_extruders() {
        let cascade = load_nozzle_fragment("snapmaker-u1", "0.4").expect("U1 0.4 nozzle");
        let rule = &cascade.rules[0];
        let diameter = rule
            .set
            .get("nozzle_diameter")
            .expect("nozzle_diameter present");
        assert_eq!(diameter, "0.4");
    }

    #[test]
    fn supertack_bed_carries_curr_bed_type_enum_value() {
        let cascade =
            load_bed_fragment("bambu-lab-a1-mini", "Supertack Plate").expect("supertack bed");
        let rule = &cascade.rules[0];
        assert_eq!(
            rule.set.get("curr_bed_type").map(String::as_str),
            Some("Supertack Plate")
        );
        assert_eq!(
            rule.set.get("identity").map(String::as_str),
            Some("Supertack Plate")
        );
    }

    #[test]
    fn bundled_beds_for_printer_lists_full_a1_mini_range() {
        // Order is file-system sort order (alphabetical by file
        // name), not authoring intent. Don't pin to a specific
        // ordering — pin to set membership instead.
        let beds: std::collections::BTreeSet<&str> = bundled_beds_for_printer("bambu-lab-a1-mini")
            .into_iter()
            .collect();
        assert_eq!(
            beds,
            [
                "Cool Plate",
                "Engineering Plate",
                "High Temp Plate",
                "Supertack Plate",
                "Textured PEI Plate",
            ]
            .into_iter()
            .collect(),
        );
        assert_eq!(
            bundled_beds_for_printer("snapmaker-u1"),
            vec!["Textured PEI Plate"]
        );
        assert!(bundled_beds_for_printer("ghost-printer").is_empty());
    }

    #[test]
    fn nozzle_skus_for_returns_declaration_order() {
        let bambu = nozzle_skus_for("bambu-lab-a1-mini");
        assert_eq!(bambu, vec!["0.2", "0.4", "0.6", "0.8"]);
        let u1 = nozzle_skus_for("snapmaker-u1");
        assert_eq!(u1, vec!["0.4", "0.6"]);
        assert!(nozzle_skus_for("ghost-printer").is_empty());
    }

    #[test]
    fn unknown_slugs_return_none() {
        assert!(load_printer_fragment("ghost").is_none());
        assert!(load_nozzle_fragment("bambu-lab-a1-mini", "9.9").is_none());
        assert!(load_bed_fragment("ghost", "Cool Plate").is_none());
        assert!(load_bed_fragment("bambu-lab-a1-mini", "Ghost Plate").is_none());
        assert!(load_filament_fragment("ghost").is_none());
        assert!(load_process_fragment("ghost", "0.20mm-standard").is_none());
        assert!(load_process_fragment("bambu-lab-a1-mini", "ghost").is_none());
    }

    #[test]
    fn printer_catalog_carries_both_bundled_printers() {
        let cat = printer_catalog();
        assert!(cat.iter().any(|e| e.identity == "bambu-lab-a1-mini"));
        assert!(cat.iter().any(|e| e.identity == "snapmaker-u1"));
        let bambu = printer_catalog_lookup("bambu-lab-a1-mini").expect("a1 mini");
        assert_eq!(bambu.profile.model, "Bambu Lab A1 mini");
        assert_eq!(bambu.fragment_slug, "bambu-lab-a1-mini");
    }

    #[test]
    fn machine_toml_missing_printer_model_fails_load() {
        // `printer_model` in machine.toml is the single source of
        // truth for the cascade's `when.printer.model = …`
        // predicates; without it every picker filter silently
        // empties. The loader hydrates `PrinterProfile.model` from
        // this scalar, so its absence is fatal.
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        let printer_dir = root.join("acme").join("printer").join("foo");
        std::fs::create_dir_all(&printer_dir).expect("mkdir printer dir");
        std::fs::write(
            printer_dir.join("machine.toml"),
            "printable_area = \"0x0,1x0,1x1,0x1\"\n",
        )
        .expect("write machine.toml");
        std::fs::write(
            printer_dir.join("model.toml"),
            "brand = \"Acme\"\ntoolheads = []\n",
        )
        .expect("write model.toml");

        let result = ProfileLibrary::load(root);
        match result {
            Err(LibraryError::MissingPrinterModel { .. }) => {}
            Err(other) => panic!("expected MissingPrinterModel, got {other:?}"),
            Ok(_) => panic!("expected MissingPrinterModel, got Ok"),
        }
    }

    #[test]
    fn printer_profile_model_is_hydrated_from_machine_toml() {
        // model.toml no longer carries `model`; the loader populates
        // it from the sibling machine.toml's `printer_model` scalar.
        let a1 = printer_catalog_lookup("bambu-lab-a1-mini").expect("a1 mini present");
        assert_eq!(a1.profile.model, "Bambu Lab A1 mini");
        let u1 = printer_catalog_lookup("snapmaker-u1").expect("u1 present");
        assert_eq!(u1.profile.model, "Snapmaker U1");
    }

    #[test]
    fn same_slug_in_two_vendors_warns_about_silent_overwrite() {
        // Repro for the resource-dir-leftover incident: a stale vendor
        // dir whose name sorts later than the current vendor silently
        // overwrote the correct filament fragment. The loader must now
        // emit a warning so the next such leftover surfaces at startup.
        use std::sync::{Arc, Mutex};
        use tracing::subscriber::with_default;

        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        for vendor in ["alpha", "zulu"] {
            let path = root.join(vendor).join("filament");
            std::fs::create_dir_all(&path).unwrap();
            std::fs::write(
                path.join("generic-pla.toml"),
                format!("filament_settings_id = \"Generic PLA ({vendor})\"\n"),
            )
            .unwrap();
        }

        let captured: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        let writer = TestWriter {
            buf: captured.clone(),
        };
        let subscriber = tracing_subscriber::fmt()
            .with_writer(move || writer.clone())
            .with_max_level(tracing::Level::WARN)
            .finish();
        let lib = with_default(subscriber, || ProfileLibrary::load(root)).expect("load");

        // Last-wins: zulu beats alpha alphabetically.
        let frag = lib.filament_fragments.get("generic-pla").expect("present");
        assert!(
            frag.cascade.rules[0]
                .set
                .get("filament_settings_id")
                .unwrap()
                .contains("zulu"),
            "later vendor still wins (preserves prior behavior)",
        );

        let log = String::from_utf8(captured.lock().unwrap().clone()).unwrap();
        assert!(
            log.contains("filament fragment slug collision"),
            "got: {log}"
        );
        assert!(
            log.contains("alpha"),
            "prior source named in warning: {log}"
        );
        assert!(log.contains("zulu"), "new source named in warning: {log}");
    }

    /// Tracing-capture sink — mirrors the pattern in
    /// `cascade::resolver` tests. Lets us assert on warn-level output
    /// without pulling in a heavier tracing-test dep.
    #[derive(Clone)]
    struct TestWriter {
        buf: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
    }
    impl std::io::Write for TestWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.buf.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
}
