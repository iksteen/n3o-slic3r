//! Safe Rust bindings for libslic3r via the OrcaSlicer FFI shim.
//!
//! Call [`init`] once before anything else. Then build a [`Config`], load a
//! [`Model`], and call [`slice`]. Option metadata for building UIs is available
//! through [`option_defs`].

#![allow(non_upper_case_globals, non_camel_case_types, non_snake_case)]

pub mod sys {
    include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
}

use std::ffi::{c_char, CStr, CString};
use std::fmt;
use std::path::Path;
use std::ptr;
use std::sync::{Mutex, Once};

// ---- Errors ----

#[derive(Debug, Clone)]
pub struct Error {
    pub kind: ErrorKind,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    InvalidArg,
    NotInitialized,
    UnknownKey,
    ParseValue,
    Io,
    Validate,
    Slice,
    Internal,
}

impl ErrorKind {
    fn from_status(s: sys::slic3r_status) -> Option<Self> {
        match s {
            sys::SLIC3R_OK => None,
            sys::SLIC3R_ERR_INVALID_ARG => Some(Self::InvalidArg),
            sys::SLIC3R_ERR_NOT_INIT => Some(Self::NotInitialized),
            sys::SLIC3R_ERR_UNKNOWN_KEY => Some(Self::UnknownKey),
            sys::SLIC3R_ERR_PARSE_VALUE => Some(Self::ParseValue),
            sys::SLIC3R_ERR_IO => Some(Self::Io),
            sys::SLIC3R_ERR_VALIDATE => Some(Self::Validate),
            sys::SLIC3R_ERR_SLICE => Some(Self::Slice),
            _ => Some(Self::Internal),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.message {
            Some(msg) => write!(f, "{:?}: {msg}", self.kind),
            None => write!(f, "{:?}", self.kind),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

fn check(status: sys::slic3r_status) -> Result<()> {
    // SAFETY: null err_ptr is always valid for check_with_err.
    unsafe { check_with_err(status, ptr::null_mut()) }
}

/// Convert a status code, taking ownership of a heap message pointer if non-null.
/// Safety: `err_ptr` must be a value previously written by the shim, or null.
unsafe fn check_with_err(status: sys::slic3r_status, err_ptr: *mut c_char) -> Result<()> {
    match ErrorKind::from_status(status) {
        None => Ok(()),
        Some(kind) => {
            let message = if err_ptr.is_null() {
                None
            } else {
                let s = CStr::from_ptr(err_ptr).to_string_lossy().into_owned();
                sys::slic3r_string_free(err_ptr);
                Some(s)
            };
            Err(Error { kind, message })
        }
    }
}

// ---- Library init ----

static INIT_GUARD: Once = Once::new();

/// One-time process init. Multiple calls collapse into one (subsequent calls
/// are silently ignored). `resources_dir` is optional and only required for
/// STEP import and font embossing. `log_level` follows boost::log severity:
/// 0=trace, 1=debug, 2=info, 3=warning, 4=error, 5=fatal.
pub fn init(resources_dir: Option<&Path>, log_level: u32) -> Result<()> {
    let mut result = Ok(());
    INIT_GUARD.call_once(|| {
        let cstr = resources_dir
            .map(|p| CString::new(p.to_string_lossy().as_bytes()).expect("resources_dir has NUL"));
        let raw = cstr.as_ref().map_or(ptr::null(), |c| c.as_ptr());
        // SAFETY: pointer either null or valid for the duration of this call.
        let status = unsafe { sys::slic3r_init(raw, log_level) };
        result = check(status);
    });
    result
}

/// Returns the FFI shim's version banner.
pub fn version() -> String {
    // SAFETY: slic3r_version returns a process-lifetime static string.
    unsafe {
        CStr::from_ptr(sys::slic3r_version())
            .to_string_lossy()
            .into_owned()
    }
}

/// Auto-orient a triangle mesh for minimal support material — the engine behind
/// OrcaSlicer's "Auto orient".
///
/// `vertices` is flattened xyz (length a non-zero multiple of 3); `indices` is
/// flattened triangle vertex indices (length a non-zero multiple of 3), all in
/// object-local coordinates. `overhang_angle` is the support threshold in
/// degrees; pass `None` for the engine default. Returns the rotation to apply to
/// the object as a unit quaternion `[x, y, z, w]`.
///
/// Pure computation — no init or model handle required. May run for a noticeable
/// time on large meshes (it runs an optimizer), so call it off any UI lock.
pub fn orient_mesh(vertices: &[f32], indices: &[u32], overhang_angle: Option<f32>) -> Result<[f32; 4]> {
    if vertices.is_empty() || vertices.len() % 3 != 0 {
        return Err(Error {
            kind: ErrorKind::InvalidArg,
            message: Some("vertices must be a non-empty multiple of 3".into()),
        });
    }
    if indices.is_empty() || indices.len() % 3 != 0 {
        return Err(Error {
            kind: ErrorKind::InvalidArg,
            message: Some("indices must be a non-empty multiple of 3".into()),
        });
    }
    // Bounds-check the indices before they reach libslic3r — it indexes the
    // vertex array unchecked (its_face_normals / facet_area), so an out-of-range
    // index would be an out-of-bounds read inside the engine.
    let vertex_count = vertices.len() / 3;
    if let Some(&max_index) = indices.iter().max() {
        if max_index as usize >= vertex_count {
            return Err(Error {
                kind: ErrorKind::InvalidArg,
                message: Some(format!(
                    "triangle index {max_index} out of range for {vertex_count} vertices"
                )),
            });
        }
    }
    let mut quat = [0.0f32; 4];
    let mut err: *mut c_char = ptr::null_mut();
    // SAFETY: slices are non-empty and length-validated above; the out pointers
    // (quat, err) are valid for the call.
    let status = unsafe {
        sys::slic3r_orient_mesh(
            vertices.as_ptr(),
            vertices.len() / 3,
            indices.as_ptr(),
            indices.len() / 3,
            overhang_angle.unwrap_or(0.0),
            quat.as_mut_ptr(),
            &mut err,
        )
    };
    // SAFETY: err is either null or a shim-owned message pointer.
    unsafe { check_with_err(status, err) }?;
    Ok(quat)
}

/// Where the nester placed one item (see [`arrange`]). `translation` (mm) and
/// `rotation` (radians) are applied to the item's footprint; `bed_idx` is the
/// logical bed it landed on: `0` = the given bed, `> 0` = spilled onto an extra
/// bed, `-1` = could not be placed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ArrangePlacement {
    pub translation: [f64; 2],
    pub rotation: f64,
    pub bed_idx: i32,
}

/// 2D auto-arrange — the engine behind OrcaSlicer's "Arrange" (libnest2d).
///
/// `contours` is one **convex** footprint per item (>= 3 points, mm); `bed` is
/// `[width, height]` (mm, origin at 0,0); `min_dist` is the minimum gap between
/// items (mm); `allow_rotations` lets the nester try discrete rotations.
/// Returns a placement per item in the same order. Pure computation — no init
/// or model handle required; may run multithreaded (TBB).
pub fn arrange(
    contours: &[Vec<[f64; 2]>],
    bed: [f64; 2],
    min_dist: f64,
    allow_rotations: bool,
) -> Result<Vec<ArrangePlacement>> {
    if contours.is_empty() {
        return Err(Error {
            kind: ErrorKind::InvalidArg,
            message: Some("no items to arrange".into()),
        });
    }
    let mut flat: Vec<f64> = Vec::new();
    let mut lengths: Vec<usize> = Vec::with_capacity(contours.len());
    for c in contours {
        if c.len() < 3 {
            return Err(Error {
                kind: ErrorKind::InvalidArg,
                message: Some("each item needs at least 3 contour points".into()),
            });
        }
        lengths.push(c.len());
        for p in c {
            flat.push(p[0]);
            flat.push(p[1]);
        }
    }
    let n = contours.len();
    let mut out_dxdy = vec![0.0f64; n * 2];
    let mut out_rot = vec![0.0f64; n];
    let mut out_bed = vec![0i32; n];
    let mut err: *mut c_char = ptr::null_mut();
    // SAFETY: all input slices outlive the call; the out buffers are sized n /
    // 2n and the out_err pointer is valid.
    let status = unsafe {
        sys::slic3r_arrange(
            flat.as_ptr(),
            lengths.as_ptr(),
            n,
            bed[0],
            bed[1],
            min_dist,
            allow_rotations as i32,
            out_dxdy.as_mut_ptr(),
            out_rot.as_mut_ptr(),
            out_bed.as_mut_ptr(),
            &mut err,
        )
    };
    // SAFETY: err is either null or a shim-owned message pointer.
    unsafe { check_with_err(status, err) }?;
    Ok((0..n)
        .map(|i| ArrangePlacement {
            translation: [out_dxdy[i * 2], out_dxdy[i * 2 + 1]],
            rotation: out_rot[i],
            bed_idx: out_bed[i],
        })
        .collect())
}

// ---- Option introspection ----

/// Mirrors `slic3r_opt_type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptType {
    None,
    Float,
    Floats,
    Int,
    Ints,
    String,
    Strings,
    Percent,
    Percents,
    FloatOrPercent,
    FloatsOrPercents,
    Point,
    Points,
    Point3,
    Bool,
    Bools,
    Enum,
    Enums,
    Unknown(u32),
}

impl OptType {
    fn from_raw(v: sys::slic3r_opt_type) -> Self {
        match v {
            sys::SLIC3R_OPT_NONE => Self::None,
            sys::SLIC3R_OPT_FLOAT => Self::Float,
            sys::SLIC3R_OPT_FLOATS => Self::Floats,
            sys::SLIC3R_OPT_INT => Self::Int,
            sys::SLIC3R_OPT_INTS => Self::Ints,
            sys::SLIC3R_OPT_STRING => Self::String,
            sys::SLIC3R_OPT_STRINGS => Self::Strings,
            sys::SLIC3R_OPT_PERCENT => Self::Percent,
            sys::SLIC3R_OPT_PERCENTS => Self::Percents,
            sys::SLIC3R_OPT_FLOAT_OR_PERCENT => Self::FloatOrPercent,
            sys::SLIC3R_OPT_FLOATS_OR_PERCENTS => Self::FloatsOrPercents,
            sys::SLIC3R_OPT_POINT => Self::Point,
            sys::SLIC3R_OPT_POINTS => Self::Points,
            sys::SLIC3R_OPT_POINT3 => Self::Point3,
            sys::SLIC3R_OPT_BOOL => Self::Bool,
            sys::SLIC3R_OPT_BOOLS => Self::Bools,
            sys::SLIC3R_OPT_ENUM => Self::Enum,
            sys::SLIC3R_OPT_ENUMS => Self::Enums,
            // The raw enum value is u32 under GCC (Linux) but i32 under MSVC
            // (windows-msvc), since C leaves an all-non-negative enum's underlying
            // type to the compiler. Normalize to the wrapper's u32.
            other => Self::Unknown(other as u32),
        }
    }

    pub fn is_vector(&self) -> bool {
        matches!(
            self,
            Self::Floats
                | Self::Ints
                | Self::Strings
                | Self::Percents
                | Self::FloatsOrPercents
                | Self::Points
                | Self::Bools
                | Self::Enums
        )
    }
}

/// Mirrors `slic3r_opt_mode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptMode {
    Simple,
    Advanced,
    Expert,
    Develop,
}

impl OptMode {
    fn from_raw(v: sys::slic3r_opt_mode) -> Self {
        match v {
            sys::SLIC3R_MODE_SIMPLE => Self::Simple,
            sys::SLIC3R_MODE_ADVANCED => Self::Advanced,
            sys::SLIC3R_MODE_DEVELOP => Self::Develop,
            _ => Self::Expert,
        }
    }
}

/// Bitmask of the scopes (libslic3r config classes) an option can be set at.
///
/// Mirrors `slic3r_opt_scope`. An option may belong to multiple scopes — most
/// commonly when an FFF and an SLA class both declare the same key (e.g.
/// `layer_height` lives in both `PrintObjectConfig` and `SLAPrintObjectConfig`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OptScope(pub u32);

impl OptScope {
    // `as u32`: the SLIC3R_SCOPE_* enum constants are u32 under GCC (Linux) but
    // i32 under MSVC (windows-msvc) — C leaves an all-non-negative enum's
    // underlying type to the compiler. The values are small positive bitflags,
    // so the cast is lossless on both.
    pub const PRINT: Self = Self(sys::SLIC3R_SCOPE_PRINT as u32);
    pub const OBJECT: Self = Self(sys::SLIC3R_SCOPE_OBJECT as u32);
    pub const REGION: Self = Self(sys::SLIC3R_SCOPE_REGION as u32);
    pub const SLA_PRINT: Self = Self(sys::SLIC3R_SCOPE_SLA_PRINT as u32);
    pub const SLA_OBJECT: Self = Self(sys::SLIC3R_SCOPE_SLA_OBJECT as u32);
    pub const SLA_MATERIAL: Self = Self(sys::SLIC3R_SCOPE_SLA_MATERIAL as u32);
    pub const SLA_PRINTER: Self = Self(sys::SLIC3R_SCOPE_SLA_PRINTER as u32);

    /// True if `other`'s bits are all set on `self`.
    pub fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0 && other.0 != 0
    }

    pub fn is_print(self) -> bool {
        self.contains(Self::PRINT)
    }
    pub fn is_object(self) -> bool {
        self.contains(Self::OBJECT)
    }
    pub fn is_region(self) -> bool {
        self.contains(Self::REGION)
    }
    pub fn is_sla_print(self) -> bool {
        self.contains(Self::SLA_PRINT)
    }
    pub fn is_sla_object(self) -> bool {
        self.contains(Self::SLA_OBJECT)
    }
    pub fn is_sla_material(self) -> bool {
        self.contains(Self::SLA_MATERIAL)
    }
    pub fn is_sla_printer(self) -> bool {
        self.contains(Self::SLA_PRINTER)
    }

    /// True for any FFF scope (Print / Object / Region).
    pub fn is_fff(self) -> bool {
        self.0 & (Self::PRINT.0 | Self::OBJECT.0 | Self::REGION.0) != 0
    }

    /// True for any SLA scope.
    pub fn is_sla(self) -> bool {
        self.0
            & (Self::SLA_PRINT.0 | Self::SLA_OBJECT.0 | Self::SLA_MATERIAL.0 | Self::SLA_PRINTER.0)
            != 0
    }
}

/// Preset-bucket classification, scraped from OrcaSlicer's `Preset.cpp`.
///
/// Every FFF config-key belongs to exactly one bucket — Printer, Filament, or
/// Process. The partitioning comes from `Preset::printer_options()` /
/// `filament_options()` / `print_options()`; extruder-keyed printer options
/// (per-extruder vectors) fall under `Printer` because that's where their
/// vendor preset stores them. See `docs/dev/orcaslicer-settings-classification.md`
/// for the upstream rationale.
///
/// Some metadata keys (`compatible_printers`, `inherits`, …) appear in all
/// three buckets upstream; they're omitted from the table, and
/// [`bucket_of`] returns `None` for them — correct UX since they're not
/// user-editable settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptBucket {
    Printer,
    Filament,
    Process,
}

mod option_buckets;
pub use option_buckets::bucket_of;

mod option_display_order;
pub use option_display_order::display_order_of;

/// An owned, allocated copy of a `slic3r_option_def_t` view, decoded into Rust types.
/// The original C struct's strings are process-lifetime so we _could_ borrow them,
/// but copying keeps the consumer ergonomics simple.
#[derive(Debug, Clone)]
pub struct OptionDef {
    pub key: String,
    pub ty: OptType,
    pub label: Option<String>,
    pub full_label: Option<String>,
    pub tooltip: Option<String>,
    pub category: Option<String>,
    pub sidetext: Option<String>,
    pub default_serialized: Option<String>,
    pub mode: OptMode,
    pub readonly: bool,
    pub multiline: bool,
    pub enum_values: Vec<String>,
    pub enum_labels: Vec<String>,
    pub min: f64,
    pub max: f64,
    pub scope: OptScope,
    /// Preset bucket (Printer / Filament / Process). `None` for metadata
    /// keys that span all buckets (`compatible_printers`, `inherits`, …)
    /// or keys outside the FFF preset universe (SLA-only, internal scratch).
    pub bucket: Option<OptBucket>,
}

unsafe fn maybe_cstr(p: *const c_char) -> Option<String> {
    if p.is_null() {
        None
    } else {
        Some(CStr::from_ptr(p).to_string_lossy().into_owned())
    }
}

impl OptionDef {
    fn from_raw(raw: &sys::slic3r_option_def_t) -> Self {
        // SAFETY: all pointers either null or point to process-lifetime strings.
        unsafe {
            let mut enum_values = Vec::with_capacity(raw.enum_value_count);
            let mut enum_labels = Vec::with_capacity(raw.enum_value_count);
            if !raw.enum_values.is_null() {
                for i in 0..raw.enum_value_count {
                    let p = *raw.enum_values.add(i);
                    enum_values.push(CStr::from_ptr(p).to_string_lossy().into_owned());
                }
            }
            if !raw.enum_labels.is_null() {
                for i in 0..raw.enum_value_count {
                    let p = *raw.enum_labels.add(i);
                    enum_labels.push(CStr::from_ptr(p).to_string_lossy().into_owned());
                }
            }
            Self {
                key: CStr::from_ptr(raw.key).to_string_lossy().into_owned(),
                ty: OptType::from_raw(raw.type_),
                label: maybe_cstr(raw.label),
                full_label: maybe_cstr(raw.full_label),
                tooltip: maybe_cstr(raw.tooltip),
                category: maybe_cstr(raw.category),
                sidetext: maybe_cstr(raw.sidetext),
                default_serialized: maybe_cstr(raw.default_serialized),
                mode: OptMode::from_raw(raw.mode),
                readonly: raw.readonly != 0,
                multiline: raw.multiline != 0,
                enum_values,
                enum_labels,
                min: raw.min,
                max: raw.max,
                scope: OptScope(raw.scope),
                bucket: None, // populated by `option_defs()` from the scraped table
            }
        }
    }
}

/// All registered options. Call after [`init`].
pub fn option_defs() -> Vec<OptionDef> {
    // SAFETY: shim guarantees thread-safe read after init.
    let count = unsafe { sys::slic3r_option_def_count() };
    let mut out = Vec::with_capacity(count);
    let mut raw: sys::slic3r_option_def_t = unsafe { std::mem::zeroed() };
    for i in 0..count {
        let status = unsafe { sys::slic3r_option_def_at(i, &mut raw) };
        if status == sys::SLIC3R_OK {
            let mut def = OptionDef::from_raw(&raw);
            def.bucket = bucket_of(&def.key);
            out.push(def);
        }
    }
    out
}

/// Look up a single option by key.
pub fn option_def(key: &str) -> Result<OptionDef> {
    let ckey = CString::new(key).map_err(|_| Error {
        kind: ErrorKind::InvalidArg,
        message: Some("key contains NUL".into()),
    })?;
    let mut raw: sys::slic3r_option_def_t = unsafe { std::mem::zeroed() };
    // SAFETY: ckey lives through the call; raw is an out-param.
    let status = unsafe { sys::slic3r_option_def_lookup(ckey.as_ptr(), &mut raw) };
    check(status)?;
    Ok(OptionDef::from_raw(&raw))
}

// ---- Config ----

pub struct Config {
    raw: *mut sys::slic3r_config_t,
}

unsafe impl Send for Config {}

impl Config {
    /// Allocate a fresh config seeded with FullPrintConfig defaults.
    pub fn new() -> Result<Self> {
        // SAFETY: shim handles the allocation; null means OOM or NotInitialized.
        let raw = unsafe { sys::slic3r_config_new() };
        if raw.is_null() {
            return Err(Error {
                kind: ErrorKind::NotInitialized,
                message: Some("did you call init()?".into()),
            });
        }
        Ok(Self { raw })
    }

    /// Set an option by key, using libslic3r's serialized value form.
    pub fn set(&mut self, key: &str, value: &str) -> Result<()> {
        let k = CString::new(key).map_err(|_| Error {
            kind: ErrorKind::InvalidArg,
            message: Some("key has NUL".into()),
        })?;
        let v = CString::new(value).map_err(|_| Error {
            kind: ErrorKind::InvalidArg,
            message: Some("value has NUL".into()),
        })?;
        // SAFETY: self.raw is a valid handle; k and v live through the call.
        let status = unsafe { sys::slic3r_config_set(self.raw, k.as_ptr(), v.as_ptr()) };
        check(status)
    }

    /// Read the current serialized value of an option.
    pub fn get(&self, key: &str) -> Result<String> {
        let k = CString::new(key).map_err(|_| Error {
            kind: ErrorKind::InvalidArg,
            message: Some("key has NUL".into()),
        })?;
        let mut out: *mut c_char = ptr::null_mut();
        // SAFETY: out is an out-param the shim writes; we free it via slic3r_string_free.
        let status = unsafe { sys::slic3r_config_get(self.raw, k.as_ptr(), &mut out) };
        check(status)?;
        if out.is_null() {
            return Ok(String::new());
        }
        let s = unsafe { CStr::from_ptr(out).to_string_lossy().into_owned() };
        unsafe {
            sys::slic3r_string_free(out);
        }
        Ok(s)
    }

    /// Run libslic3r's cross-option validator. Ok(()) means the config is sliceable.
    pub fn validate(&self) -> Result<()> {
        let mut err: *mut c_char = ptr::null_mut();
        // SAFETY: err is an out-param; if non-null on return, we own it.
        let status = unsafe { sys::slic3r_config_validate(self.raw, &mut err) };
        unsafe { check_with_err(status, err) }
    }
}

impl Drop for Config {
    fn drop(&mut self) {
        // SAFETY: raw was returned by slic3r_config_new and we have unique ownership.
        unsafe { sys::slic3r_config_free(self.raw) };
    }
}

// ---- Model ----

pub struct Model {
    raw: *mut sys::slic3r_model_t,
}

unsafe impl Send for Model {}

impl Model {
    pub fn new() -> Result<Self> {
        // SAFETY: shim handles allocation.
        let raw = unsafe { sys::slic3r_model_new() };
        if raw.is_null() {
            return Err(Error {
                kind: ErrorKind::Internal,
                message: Some("slic3r_model_new returned null".into()),
            });
        }
        Ok(Self { raw })
    }

    /// Load a model file. Format detected from extension.
    pub fn load<P: AsRef<Path>>(&mut self, path: P) -> Result<()> {
        let p = CString::new(path.as_ref().to_string_lossy().as_bytes()).map_err(|_| Error {
            kind: ErrorKind::InvalidArg,
            message: Some("path has NUL".into()),
        })?;
        let mut err: *mut c_char = ptr::null_mut();
        // SAFETY: p lives through the call; err is an out-param we own on non-null return.
        let status = unsafe { sys::slic3r_model_load(self.raw, p.as_ptr(), &mut err) };
        unsafe { check_with_err(status, err) }
    }

    /// Load a model file and fold any settings embedded in the file into
    /// `config`. For 3MFs, picks up the printer/print/filament settings
    /// stored in `Metadata/project_settings.config`. Pre-existing values in
    /// `config` are preserved unless overridden by the file.
    ///
    /// For STL/OBJ/STEP files (which carry no embedded config) this is
    /// equivalent to [`Model::load`] and leaves `config` untouched.
    pub fn load_with_config<P: AsRef<Path>>(&mut self, path: P, config: &mut Config) -> Result<()> {
        let p = CString::new(path.as_ref().to_string_lossy().as_bytes()).map_err(|_| Error {
            kind: ErrorKind::InvalidArg,
            message: Some("path has NUL".into()),
        })?;
        let mut err: *mut c_char = ptr::null_mut();
        // SAFETY: handles are valid; p lives through the call; err is an out-param we own on non-null return.
        let status = unsafe {
            sys::slic3r_model_load_with_config(self.raw, config.raw, p.as_ptr(), &mut err)
        };
        unsafe { check_with_err(status, err) }
    }

    /// Remap MMU color-painting (`paint_color`) filament states in place.
    ///
    /// Each painted face's filament state `s` is replaced with `perm[s]`
    /// (states `>= perm.len()`, and unpainted faces, are left unchanged).
    /// Index 0 is the object's own extruder and should map to itself. Used to
    /// make painted filament indices follow n3o's per-object extruder remap on
    /// toolchanger printers, where the base extruder is rewritten to a
    /// flat-slot index and the paint must follow. No-op on an unpainted model.
    pub fn remap_paint_filaments(&mut self, perm: &[i32]) -> Result<()> {
        let status =
            unsafe { sys::slic3r_model_remap_paint_filaments(self.raw, perm.as_ptr(), perm.len()) };
        check(status)
    }
}

impl Drop for Model {
    fn drop(&mut self) {
        // SAFETY: raw was returned by slic3r_model_new and we have unique ownership.
        unsafe { sys::slic3r_model_free(self.raw) };
    }
}

// ---- Slicing + per-slice progress callback ----
//
// `slice` takes a closure that fires on every libslic3r status tick
// for the duration of *this* call. The closure is captured per-slice:
// no global registration, no cross-slice contamination at the FFI
// layer. See the per-fn docs on `slice` below for the libslic3r-
// level thread-safety caveat (TL;DR: serialize at the application
// layer; concurrent Print::process() calls SIGSEGV on heavier
// workloads).
//
// Rust↔C bridge: pin the closure as a `&mut dyn FnMut` on the
// stack and hand its address to C as opaque user_data. The
// trampoline reconstructs the reference and calls it. No heap
// allocation, no global state, no leak — the closure drops when
// the `slice` call's stack frame unwinds.

/// Trampoline registered with the C side per `slice` call. Treats
/// `user_data` as a `*mut &mut dyn FnMut(i32, &str)` and invokes it.
///
/// SAFETY:
/// - `user_data` must be the `&mut &mut dyn FnMut` address passed to
///   `slic3r_slice` from `slice` below; nothing else writes to that
///   pointer. The closure outlives the slice call.
/// - `stage` is NUL-terminated and valid for the duration of the
///   call (guaranteed by the C side).
extern "C" fn progress_trampoline(
    percent: i32,
    stage: *const c_char,
    user_data: *mut std::ffi::c_void,
) {
    if user_data.is_null() {
        return;
    }
    let stage_str = if stage.is_null() {
        ""
    } else {
        // SAFETY: stage is NUL-terminated for the call's duration.
        unsafe { CStr::from_ptr(stage) }
            .to_str()
            .unwrap_or_default()
    };
    // SAFETY: user_data is the `&mut &mut dyn FnMut` slot owned by
    // the calling `slice` invocation; the reference is exclusive
    // for the lifetime of the slice call.
    let cb: &mut &mut dyn FnMut(i32, &str) =
        unsafe { &mut *(user_data as *mut &mut dyn FnMut(i32, &str)) };
    cb(percent, stage_str);
}

/// Process-wide serialization mutex for [`slice`]. libslic3r isn't
/// safe to drive concurrently at the `Print::process()` level —
/// heavier multi-material workloads SIGSEGV when two slices race
/// (fourcolor benchy + cube-halves + snappy 4-color, observed in
/// CI May 2026). The lock is held for the duration of the slice
/// call so libslic3r runs one slice at a time across the whole
/// process — independent of thread count, job count, or test
/// binary count.
///
/// Poison recovery: a previous slice that panicked leaves the lock
/// poisoned, but the inner `()` carries no state, so we recover the
/// guard and keep slicing.
static SLICE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Slice `model` with `config`, writing G-code to `out_gcode_path`.
///
/// Calls libslic3r through the FFI under the process-wide
/// [`SLICE_LOCK`]; concurrent callers queue rather than race. The
/// progress callback fires synchronously on the slicing thread,
/// already serialized by libslic3r's C++-side per-slice mutex (see
/// `crates/slic3r-ffi/ffi/slic3r_ffi.cpp` — `print.set_status_callback`
/// lambda). Together the two locks give the `FnMut` callback the
/// exclusive-access guarantee Rust requires, even though libslic3r
/// fans `set_status` calls across many TBB worker threads inside a
/// single slice.
/// The prime/wipe tower's exact mesh as libslic3r built it during the
/// slice (a box for AMS purge towers, the rib/cone solid for
/// toolchangers), in tower-local millimetres. `vertices` is 3 floats per
/// vertex; `indices` is 3 vertex indices per triangle. [`slice`] returns
/// `None` when the plate is single-material (no tower).
#[derive(Debug, Clone)]
pub struct TowerMesh {
    pub vertices: Vec<f32>,
    pub indices: Vec<u32>,
}

/// Severity of an advisory slice [diagnostic](SliceOutcome). Fatal errors
/// abort the slice and come back as the `Err` of `SliceOutcome::result`
/// instead of as a diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Warning,
}

/// The outcome of a slice attempt: the advisory `diagnostics` libslic3r
/// reported (zero or more `(severity, message)` pairs — e.g. a mismatched-
/// filament-shrinkage warning), paired with the slice `result` (the tower
/// mesh on success, or the error). Diagnostics are computed before
/// `process()` runs, so they're reported whether the slice then **succeeds
/// or fails** — letting the UI surface a warning even on a failed slice.
#[must_use]
pub struct SliceOutcome {
    pub diagnostics: Vec<(Severity, String)>,
    pub result: Result<Option<TowerMesh>>,
}

/// Slice, returning the advisory diagnostics alongside the result — see
/// [`SliceOutcome`].
pub fn slice_outcome<P, F>(
    model: &Model,
    config: &Config,
    out_gcode_path: P,
    mut progress: F,
) -> SliceOutcome
where
    P: AsRef<Path>,
    F: FnMut(i32, &str),
{
    let _guard = SLICE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let p = match CString::new(out_gcode_path.as_ref().to_string_lossy().as_bytes()) {
        Ok(p) => p,
        Err(_) => {
            return SliceOutcome {
                diagnostics: Vec::new(),
                result: Err(Error {
                    kind: ErrorKind::InvalidArg,
                    message: Some("path has NUL".into()),
                }),
            };
        }
    };
    let mut err: *mut c_char = ptr::null_mut();
    // Pin the closure as a trait object on the stack and hand its
    // address to C as opaque user_data. The double-reference
    // (`&mut &mut dyn FnMut`) keeps the trait-object fat pointer
    // addressable through a thin `*mut c_void`.
    let mut cb_ref: &mut dyn FnMut(i32, &str) = &mut progress;
    let user_data = &mut cb_ref as *mut &mut dyn FnMut(i32, &str) as *mut std::ffi::c_void;
    let mut tower_verts: *mut f32 = ptr::null_mut();
    let mut tower_vcount: usize = 0;
    let mut tower_idx: *mut u32 = ptr::null_mut();
    let mut tower_icount: usize = 0;
    let mut warning: *mut c_char = ptr::null_mut();
    // SAFETY:
    // - handles are valid; p + user_data live through the call.
    // - err is an out-param we own on non-null return.
    // - the trampoline only dereferences user_data during the call;
    //   `cb_ref` outlives it (stack frame lives until slice returns).
    // - the tower out-params are an all-or-nothing group the shim fills
    //   on success; `take_tower_mesh` copies them out and frees them with
    //   the matching deallocator.
    let status = unsafe {
        sys::slic3r_slice(
            model.raw,
            config.raw,
            p.as_ptr(),
            Some(progress_trampoline),
            user_data,
            &mut tower_verts,
            &mut tower_vcount,
            &mut tower_idx,
            &mut tower_icount,
            &mut err,
            &mut warning,
        )
    };
    let tower = unsafe { take_tower_mesh(tower_verts, tower_vcount, tower_idx, tower_icount) };
    // Always reclaim the warning string (freed here) even on the error path,
    // and keep it whether the slice succeeded or failed.
    let warning = unsafe { take_c_string(warning) };
    let result = unsafe { check_with_err(status, err) }.map(|()| tower);
    let diagnostics = warning
        .into_iter()
        .map(|message| (Severity::Warning, message))
        .collect();
    SliceOutcome {
        diagnostics,
        result,
    }
}

/// Slice and return just the tower mesh (or the error), discarding any
/// advisory diagnostics. Convenience for callers that don't surface them
/// (tests, examples); the orchestrator uses [`slice_outcome`] instead.
pub fn slice<P, F>(
    model: &Model,
    config: &Config,
    out_gcode_path: P,
    progress: F,
) -> Result<Option<TowerMesh>>
where
    P: AsRef<Path>,
    F: FnMut(i32, &str),
{
    slice_outcome(model, config, out_gcode_path, progress).result
}

/// Copy a heap-allocated C string out-param into an owned `String` and free
/// it with `slic3r_string_free`. `None` for a null pointer.
///
/// # Safety
/// `ptr` must be null or a string the shim allocated via `set_err`; it is
/// freed here and must not be used afterward.
unsafe fn take_c_string(ptr: *mut c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    let s = CStr::from_ptr(ptr).to_string_lossy().into_owned();
    sys::slic3r_string_free(ptr);
    Some(s)
}

/// Copy the shim's tower buffers into owned `Vec`s and free the C
/// allocations. `None` when either buffer is null/empty (the
/// single-material no-tower case). Always frees, so it's safe to call on
/// the slice error path too.
///
/// # Safety
/// `verts`/`idx` must be the exact pointers the shim wrote (or null) with
/// `vcount`/`icount` their element counts; they are freed here and must
/// not be used afterward.
unsafe fn take_tower_mesh(
    verts: *mut f32,
    vcount: usize,
    idx: *mut u32,
    icount: usize,
) -> Option<TowerMesh> {
    if verts.is_null() || idx.is_null() || vcount == 0 || icount == 0 {
        // Defensive: free whichever half (if any) is non-null.
        sys::slic3r_tower_mesh_free(verts, idx);
        return None;
    }
    let vertices = std::slice::from_raw_parts(verts, vcount * 3).to_vec();
    let indices = std::slice::from_raw_parts(idx, icount * 3).to_vec();
    sys::slic3r_tower_mesh_free(verts, idx);
    Some(TowerMesh { vertices, indices })
}

// ---- Log sink ----
//
// Same pattern as the progress callback: the C side stores a
// global fn pointer + user_data, the Rust side owns the closure
// behind a `Mutex<Option<Box<dyn FnMut + Send>>>`, an `extern "C"`
// trampoline bridges them.

/// Severity ladder mirroring boost::log::trivial. Matches the
/// integer values libslic3r emits — 0=trace, 5=fatal. Cast directly
/// from the FFI `int`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warning,
    Error,
    Fatal,
}

impl LogLevel {
    fn from_raw(value: i32) -> Self {
        match value {
            0 => Self::Trace,
            1 => Self::Debug,
            2 => Self::Info,
            3 => Self::Warning,
            4 => Self::Error,
            5 => Self::Fatal,
            // Out-of-range values bucket as Warning so the caller
            // still sees them — silently dropping a misclassified
            // record would be worse than over-reporting.
            _ => Self::Warning,
        }
    }
}

type LogClosure = Box<dyn FnMut(LogLevel, &str) + Send>;

static LOG_CALLBACK: Mutex<Option<LogClosure>> = Mutex::new(None);

extern "C" fn log_trampoline(
    severity: i32,
    message: *const c_char,
    _user_data: *mut std::ffi::c_void,
) {
    let msg_str = if message.is_null() {
        ""
    } else {
        // SAFETY: caller (C side) guarantees message is NUL-terminated.
        unsafe { CStr::from_ptr(message) }
            .to_str()
            .unwrap_or_default()
    };
    let level = LogLevel::from_raw(severity);
    if let Ok(mut guard) = LOG_CALLBACK.lock() {
        if let Some(cb) = guard.as_mut() {
            cb(level, msg_str);
        }
    }
}

/// Register a Rust closure as the log sink. Replaces any previously
/// registered callback. Every libslic3r `BOOST_LOG_TRIVIAL(...)`
/// record at or above the current severity filter (set via
/// `init(.., log_level)`) fires the closure.
pub fn set_log_sink<F>(callback: F)
where
    F: FnMut(LogLevel, &str) + Send + 'static,
{
    let mut guard = LOG_CALLBACK.lock().expect("log callback mutex");
    *guard = Some(Box::new(callback));
    drop(guard);
    // SAFETY: trampoline is a static `extern "C"` fn; user_data is
    // null because the closure pointer lives in our Rust static,
    // not in the C side.
    unsafe {
        sys::slic3r_set_log_sink(Some(log_trampoline), ptr::null_mut());
    }
}

/// Clear the log sink. Records still flow through libslic3r's
/// internal sink list (the FFI keeps its sink installed for the
/// process lifetime), but with no callback they no-op.
pub fn clear_log_sink() {
    // SAFETY: passing nullptr to the C API is the documented
    // "unregister" semantics.
    unsafe {
        sys::slic3r_set_log_sink(None, ptr::null_mut());
    }
    let mut guard = LOG_CALLBACK.lock().expect("log callback mutex");
    *guard = None;
}
