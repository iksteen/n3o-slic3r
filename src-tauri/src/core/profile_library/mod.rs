//! Hierarchical vendor profile library, loaded from disk at startup.
//!
//! On-disk layout (root = `profiles/vendor/`):
//!
//! ```text
//! <root>/<vendor>/
//! ├── printer/
//! │   ├── <slug>.toml                  ← cascade fragment (machine globals)
//! │   └── <slug>/
//! │       ├── printer.toml             ← PrinterProfile metadata (n3o shape)
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
//!   `printer.toml`, falls back to the directory name).

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::Deserialize;

use crate::core::cascade::loader::{parse_cascade_str, CascadeLoadError};
use crate::core::cascade::types::Cascade;
use crate::core::printer::profile::PrinterProfile;

pub mod composer;
pub use composer::{compose_cascade, ComposeError};

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
}

impl std::fmt::Display for LibraryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingRoot(p) => write!(f, "profile root `{}` does not exist", p.display()),
            Self::Io(p, e) => write!(f, "io error reading `{}`: {e}", p.display()),
            Self::Toml(p, e) => write!(f, "parse error in `{}`: {e}", p.display()),
            Self::Cascade(p, e) => write!(f, "cascade load error in `{}`: {e}", p.display()),
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
    #[allow(dead_code)]
    source_path: String,
}

/// One printer catalog entry, parsed from `printer.toml` next to the
/// printer's fragment.
#[derive(Debug, Clone)]
pub struct PrinterCatalogEntry {
    pub identity: String,
    pub profile: PrinterProfile,
    /// Fragment slug = directory name. Equal to identity when no
    /// `identity` override is declared in `printer.toml`.
    pub fragment_slug: String,
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
    /// Filament slug declaration order (UI presentation).
    filament_order: Vec<String>,

    /// Picker catalog — one entry per `<printer_dir>/printer.toml`.
    catalog: Vec<PrinterCatalogEntry>,
}

static LIBRARY: OnceLock<ProfileLibrary> = OnceLock::new();

/// Explicit init. The Tauri runtime calls this from `setup()` with
/// the bundled-resources `profiles/vendor` path. Subsequent calls
/// are no-ops (`OnceLock` only initializes the first time).
pub fn init_from(root: PathBuf) {
    let _ = LIBRARY.get_or_init(|| {
        ProfileLibrary::load(&root)
            .unwrap_or_else(|e| panic!("profile library load failed: {e}"))
    });
}

fn library() -> &'static ProfileLibrary {
    LIBRARY.get_or_init(|| {
        // Test/dev fallback: walk up from the manifest dir to find
        // `profiles/vendor` in the workspace. A packaged binary
        // *must* explicitly call `init_from` before any lookup; if
        // it doesn't, this fallback will pick up a stale build-time
        // path that doesn't exist post-install and `load` will panic
        // with a clear "missing root" message.
        let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("workspace root above manifest dir")
            .to_path_buf();
        let root = workspace_root.join("profiles/vendor");
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
                self.filament_fragments.insert(slug.clone(), asset);
                if !self.filament_order.contains(&slug) {
                    self.filament_order.push(slug);
                }
            }
        }
        Ok(())
    }

    fn load_printers(&mut self, root: &Path, printer_root: &Path) -> Result<(), LibraryError> {
        // Top-level cascade fragments: <printer_root>/<slug>.toml
        for f in read_sorted_files(printer_root)? {
            let slug = file_stem(&f);
            let asset = read_cascade(root, &f)?;
            self.printer_fragments.insert(slug, asset);
        }
        // Sub-directories carry the per-printer breakdown (nozzles/,
        // beds/, printer.toml). Iterate in stable order.
        for printer_dir in read_sorted_subdirs(printer_root)? {
            let slug = printer_dir
                .file_name()
                .and_then(|n| n.to_str())
                .expect("directory name is valid utf-8")
                .to_owned();
            // printer.toml — catalog metadata.
            let meta_path = printer_dir.join("printer.toml");
            if meta_path.is_file() {
                let raw = std::fs::read_to_string(&meta_path)
                    .map_err(|e| LibraryError::Io(meta_path.clone(), e))?;
                let envelope: PrinterProfileEnvelope =
                    toml::from_str(&raw).map_err(|e| LibraryError::Toml(meta_path.clone(), e))?;
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
                    self.nozzle_fragments
                        .insert((slug.clone(), sku.clone()), asset);
                    self.nozzle_order
                        .entry(slug.clone())
                        .or_default()
                        .push(sku);
                }
            }
            // processes/<slug>.toml — printer-bound process presets.
            let processes_dir = printer_dir.join("processes");
            if processes_dir.is_dir() {
                for f in read_sorted_files(&processes_dir)? {
                    let process_slug = file_stem(&f);
                    let asset = read_cascade(root, &f)?;
                    self.process_fragments
                        .insert((slug.clone(), process_slug), asset);
                }
            }
            // beds/<name>.toml — identity comes from inside the file.
            let beds_dir = printer_dir.join("beds");
            if beds_dir.is_dir() {
                for f in read_sorted_files(&beds_dir)? {
                    let raw = std::fs::read_to_string(&f)
                        .map_err(|e| LibraryError::Io(f.clone(), e))?;
                    let probe: BedIdentityProbe = toml::from_str(&raw)
                        .map_err(|e| LibraryError::Toml(f.clone(), e))?;
                    let identity = probe.identity;
                    let asset = parse_cascade(root, &f, &raw)?;
                    self.bed_fragments
                        .insert((slug.clone(), identity.clone()), asset);
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

fn parse_cascade(
    root: &Path,
    path: &Path,
    raw: &str,
) -> Result<CascadeAsset, LibraryError> {
    // Trace UI uses this path verbatim; show it relative to the
    // vendor root so traces stay readable across machines.
    let relative = path
        .strip_prefix(root)
        .map(Path::to_path_buf)
        .unwrap_or_else(|_| path.to_path_buf());
    let rules = parse_cascade_str(raw, &relative)
        .map_err(|e| LibraryError::Cascade(path.to_owned(), e))?;
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

/// One bundled vendor filament's identity + display label, surfaced
/// to the frontend slot-binding panel. `identity` is the slug
/// (matches the wire form stored in `SlotBinding.filament_identity`);
/// `display_name` is the `filament_settings_id` field a human will
/// recognize ("Bambu PLA Basic @BBL A1M"); `base_type` drives the
/// swatch color in the picker.
#[derive(Debug, Clone, serde::Serialize)]
pub struct FilamentFragmentSummary {
    pub identity: String,
    pub display_name: String,
    pub base_type: String,
}

/// Enumerate every bundled vendor filament fragment. Parses the
/// `filament_settings_id` + `filament_type` fields out of each
/// fragment (stamped by the vendor converter, stable across regens).
pub fn list_filament_fragments() -> Vec<FilamentFragmentSummary> {
    library()
        .filament_order
        .iter()
        .map(|slug| {
            let cascade = library()
                .filament_fragments
                .get(slug)
                .expect("filament_order index always present in fragments map");
            // The cascade.rules vec carries one unconditional rule
            // for converter output; read `filament_settings_id` /
            // `filament_type` out of its set.
            let set = cascade
                .cascade
                .rules
                .first()
                .map(|r| &r.set)
                .expect("filament fragment carries at least one rule");
            let display_name = set
                .get("filament_settings_id")
                .cloned()
                .unwrap_or_else(|| slug.clone());
            let base_type = set
                .get("filament_type")
                .cloned()
                .unwrap_or_else(|| "PLA".to_owned());
            FilamentFragmentSummary {
                identity: slug.clone(),
                display_name,
                base_type,
            }
        })
        .collect()
}

/// Printer catalog — one entry per `printer.toml` found on disk.
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
    fn every_bundled_fragment_parses() {
        // Smoke: walk the library and ensure each registered slug
        // resolves back through the public loaders.
        for slug in library().printer_fragments.keys() {
            assert!(load_printer_fragment(slug).is_some(), "printer {slug}");
        }
        for (printer, sku) in library().nozzle_fragments.keys() {
            assert!(load_nozzle_fragment(printer, sku).is_some(), "nozzle {printer}/{sku}");
        }
        for (printer, identity) in library().bed_fragments.keys() {
            assert!(load_bed_fragment(printer, identity).is_some(), "bed {printer}/{identity}");
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
            "nozzle_diameter must NOT be in printer.toml (lives in nozzles/<sku>.toml)",
        );
    }

    #[test]
    fn a1_mini_0_4_nozzle_carries_scalar_diameter() {
        let cascade = load_nozzle_fragment("bambu-lab-a1-mini", "0.4").expect("0.4 nozzle");
        let rule = &cascade.rules[0];
        let diameter = rule.set.get("nozzle_diameter").expect("nozzle_diameter present");
        assert_eq!(diameter, "0.4");
    }

    #[test]
    fn u1_0_4_nozzle_is_also_scalar_despite_4_extruders() {
        let cascade = load_nozzle_fragment("snapmaker-u1", "0.4").expect("U1 0.4 nozzle");
        let rule = &cascade.rules[0];
        let diameter = rule.set.get("nozzle_diameter").expect("nozzle_diameter present");
        assert_eq!(diameter, "0.4");
    }

    #[test]
    fn supertack_bed_carries_curr_bed_type_enum_value() {
        let cascade = load_bed_fragment("bambu-lab-a1-mini", "Supertack Plate")
            .expect("supertack bed");
        let rule = &cascade.rules[0];
        assert_eq!(rule.set.get("curr_bed_type").map(String::as_str), Some("Supertack Plate"));
        assert_eq!(rule.set.get("identity").map(String::as_str), Some("Supertack Plate"));
    }

    #[test]
    fn bundled_beds_for_printer_lists_full_a1_mini_range() {
        // Order is file-system sort order (alphabetical by file
        // name), not authoring intent. Don't pin to a specific
        // ordering — pin to set membership instead.
        let beds: std::collections::BTreeSet<&str> =
            bundled_beds_for_printer("bambu-lab-a1-mini").into_iter().collect();
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
        assert_eq!(bundled_beds_for_printer("snapmaker-u1"), vec!["Textured PEI Plate"]);
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
        assert!(load_process_fragment("ghost", "0.20mm-standard-bbl-a1m").is_none());
        assert!(load_process_fragment("bambu-lab-a1-mini", "ghost").is_none());
    }

    #[test]
    fn printer_catalog_carries_both_bundled_printers() {
        let cat = printer_catalog();
        assert!(cat.iter().any(|e| e.identity == "bambu-lab-a1-mini"));
        assert!(cat.iter().any(|e| e.identity == "snapmaker-u1"));
        let bambu = printer_catalog_lookup("bambu-lab-a1-mini").expect("a1 mini");
        assert_eq!(bambu.profile.model, "Bambu A1 mini");
        assert_eq!(bambu.fragment_slug, "bambu-lab-a1-mini");
    }
}
