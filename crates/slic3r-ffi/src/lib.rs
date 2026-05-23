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
use std::sync::Once;

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
            sys::SLIC3R_ERR_NOT_INIT    => Some(Self::NotInitialized),
            sys::SLIC3R_ERR_UNKNOWN_KEY => Some(Self::UnknownKey),
            sys::SLIC3R_ERR_PARSE_VALUE => Some(Self::ParseValue),
            sys::SLIC3R_ERR_IO          => Some(Self::Io),
            sys::SLIC3R_ERR_VALIDATE    => Some(Self::Validate),
            sys::SLIC3R_ERR_SLICE       => Some(Self::Slice),
            _                           => Some(Self::Internal),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.message {
            Some(msg) => write!(f, "{:?}: {msg}", self.kind),
            None      => write!(f, "{:?}", self.kind),
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
            sys::SLIC3R_OPT_NONE               => Self::None,
            sys::SLIC3R_OPT_FLOAT              => Self::Float,
            sys::SLIC3R_OPT_FLOATS             => Self::Floats,
            sys::SLIC3R_OPT_INT                => Self::Int,
            sys::SLIC3R_OPT_INTS               => Self::Ints,
            sys::SLIC3R_OPT_STRING             => Self::String,
            sys::SLIC3R_OPT_STRINGS            => Self::Strings,
            sys::SLIC3R_OPT_PERCENT            => Self::Percent,
            sys::SLIC3R_OPT_PERCENTS           => Self::Percents,
            sys::SLIC3R_OPT_FLOAT_OR_PERCENT   => Self::FloatOrPercent,
            sys::SLIC3R_OPT_FLOATS_OR_PERCENTS => Self::FloatsOrPercents,
            sys::SLIC3R_OPT_POINT              => Self::Point,
            sys::SLIC3R_OPT_POINTS             => Self::Points,
            sys::SLIC3R_OPT_POINT3             => Self::Point3,
            sys::SLIC3R_OPT_BOOL               => Self::Bool,
            sys::SLIC3R_OPT_BOOLS              => Self::Bools,
            sys::SLIC3R_OPT_ENUM               => Self::Enum,
            sys::SLIC3R_OPT_ENUMS              => Self::Enums,
            other                              => Self::Unknown(other),
        }
    }

    pub fn is_vector(&self) -> bool {
        matches!(self,
            Self::Floats | Self::Ints | Self::Strings | Self::Percents |
            Self::FloatsOrPercents | Self::Points | Self::Bools | Self::Enums)
    }
}

/// Mirrors `slic3r_opt_mode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptMode { Simple, Advanced, Expert, Develop }

impl OptMode {
    fn from_raw(v: sys::slic3r_opt_mode) -> Self {
        match v {
            sys::SLIC3R_MODE_SIMPLE   => Self::Simple,
            sys::SLIC3R_MODE_ADVANCED => Self::Advanced,
            sys::SLIC3R_MODE_DEVELOP  => Self::Develop,
            _                         => Self::Expert,
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
    pub const PRINT: Self        = Self(sys::SLIC3R_SCOPE_PRINT as u32);
    pub const OBJECT: Self       = Self(sys::SLIC3R_SCOPE_OBJECT as u32);
    pub const REGION: Self       = Self(sys::SLIC3R_SCOPE_REGION as u32);
    pub const SLA_PRINT: Self    = Self(sys::SLIC3R_SCOPE_SLA_PRINT as u32);
    pub const SLA_OBJECT: Self   = Self(sys::SLIC3R_SCOPE_SLA_OBJECT as u32);
    pub const SLA_MATERIAL: Self = Self(sys::SLIC3R_SCOPE_SLA_MATERIAL as u32);
    pub const SLA_PRINTER: Self  = Self(sys::SLIC3R_SCOPE_SLA_PRINTER as u32);

    /// True if `other`'s bits are all set on `self`.
    pub fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0 && other.0 != 0
    }

    pub fn is_print(self)        -> bool { self.contains(Self::PRINT) }
    pub fn is_object(self)       -> bool { self.contains(Self::OBJECT) }
    pub fn is_region(self)       -> bool { self.contains(Self::REGION) }
    pub fn is_sla_print(self)    -> bool { self.contains(Self::SLA_PRINT) }
    pub fn is_sla_object(self)   -> bool { self.contains(Self::SLA_OBJECT) }
    pub fn is_sla_material(self) -> bool { self.contains(Self::SLA_MATERIAL) }
    pub fn is_sla_printer(self)  -> bool { self.contains(Self::SLA_PRINTER) }

    /// True for any FFF scope (Print / Object / Region).
    pub fn is_fff(self) -> bool {
        self.0 & (Self::PRINT.0 | Self::OBJECT.0 | Self::REGION.0) != 0
    }

    /// True for any SLA scope.
    pub fn is_sla(self) -> bool {
        self.0 & (Self::SLA_PRINT.0 | Self::SLA_OBJECT.0
                | Self::SLA_MATERIAL.0 | Self::SLA_PRINTER.0) != 0
    }
}

/// An owned, allocated copy of a `slic3r_option_def_t` view, decoded into Rust types.
/// The original C struct's strings are process-lifetime so we _could_ borrow them,
/// but copying keeps the consumer ergonomics simple.
#[derive(Debug, Clone)]
pub struct OptionDef {
    pub key:        String,
    pub ty:         OptType,
    pub label:      Option<String>,
    pub full_label: Option<String>,
    pub tooltip:    Option<String>,
    pub category:   Option<String>,
    pub sidetext:   Option<String>,
    pub default_serialized: Option<String>,
    pub mode:       OptMode,
    pub readonly:   bool,
    pub multiline:  bool,
    pub enum_values: Vec<String>,
    pub enum_labels: Vec<String>,
    pub min: f64,
    pub max: f64,
    pub scope: OptScope,
}

unsafe fn maybe_cstr(p: *const c_char) -> Option<String> {
    if p.is_null() { None } else { Some(CStr::from_ptr(p).to_string_lossy().into_owned()) }
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
                label:      maybe_cstr(raw.label),
                full_label: maybe_cstr(raw.full_label),
                tooltip:    maybe_cstr(raw.tooltip),
                category:   maybe_cstr(raw.category),
                sidetext:   maybe_cstr(raw.sidetext),
                default_serialized: maybe_cstr(raw.default_serialized),
                mode: OptMode::from_raw(raw.mode),
                readonly:  raw.readonly  != 0,
                multiline: raw.multiline != 0,
                enum_values,
                enum_labels,
                min: raw.min,
                max: raw.max,
                scope: OptScope(raw.scope),
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
            out.push(OptionDef::from_raw(&raw));
        }
    }
    out
}

/// Look up a single option by key.
pub fn option_def(key: &str) -> Result<OptionDef> {
    let ckey = CString::new(key).map_err(|_| Error { kind: ErrorKind::InvalidArg, message: Some("key contains NUL".into()) })?;
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
            return Err(Error { kind: ErrorKind::NotInitialized, message: Some("did you call init()?".into()) });
        }
        Ok(Self { raw })
    }

    /// Set an option by key, using libslic3r's serialized value form.
    pub fn set(&mut self, key: &str, value: &str) -> Result<()> {
        let k = CString::new(key).map_err(|_| Error { kind: ErrorKind::InvalidArg, message: Some("key has NUL".into()) })?;
        let v = CString::new(value).map_err(|_| Error { kind: ErrorKind::InvalidArg, message: Some("value has NUL".into()) })?;
        // SAFETY: self.raw is a valid handle; k and v live through the call.
        let status = unsafe { sys::slic3r_config_set(self.raw, k.as_ptr(), v.as_ptr()) };
        check(status)
    }

    /// Read the current serialized value of an option.
    pub fn get(&self, key: &str) -> Result<String> {
        let k = CString::new(key).map_err(|_| Error { kind: ErrorKind::InvalidArg, message: Some("key has NUL".into()) })?;
        let mut out: *mut c_char = ptr::null_mut();
        // SAFETY: out is an out-param the shim writes; we free it via slic3r_string_free.
        let status = unsafe { sys::slic3r_config_get(self.raw, k.as_ptr(), &mut out) };
        check(status)?;
        if out.is_null() {
            return Ok(String::new());
        }
        let s = unsafe { CStr::from_ptr(out).to_string_lossy().into_owned() };
        unsafe { sys::slic3r_string_free(out); }
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
            return Err(Error { kind: ErrorKind::Internal, message: Some("slic3r_model_new returned null".into()) });
        }
        Ok(Self { raw })
    }

    /// Load a model file. Format detected from extension.
    pub fn load<P: AsRef<Path>>(&mut self, path: P) -> Result<()> {
        let p = CString::new(path.as_ref().to_string_lossy().as_bytes())
            .map_err(|_| Error { kind: ErrorKind::InvalidArg, message: Some("path has NUL".into()) })?;
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
        let p = CString::new(path.as_ref().to_string_lossy().as_bytes())
            .map_err(|_| Error { kind: ErrorKind::InvalidArg, message: Some("path has NUL".into()) })?;
        let mut err: *mut c_char = ptr::null_mut();
        // SAFETY: handles are valid; p lives through the call; err is an out-param we own on non-null return.
        let status = unsafe {
            sys::slic3r_model_load_with_config(self.raw, config.raw, p.as_ptr(), &mut err)
        };
        unsafe { check_with_err(status, err) }
    }
}

impl Drop for Model {
    fn drop(&mut self) {
        // SAFETY: raw was returned by slic3r_model_new and we have unique ownership.
        unsafe { sys::slic3r_model_free(self.raw) };
    }
}

// ---- Slicing ----

/// Slice `model` with `config`, writing G-code to `out_gcode_path`.
pub fn slice<P: AsRef<Path>>(model: &Model, config: &Config, out_gcode_path: P) -> Result<()> {
    let p = CString::new(out_gcode_path.as_ref().to_string_lossy().as_bytes())
        .map_err(|_| Error { kind: ErrorKind::InvalidArg, message: Some("path has NUL".into()) })?;
    let mut err: *mut c_char = ptr::null_mut();
    // SAFETY: handles are valid; p lives through the call; err is an out-param we own on non-null return.
    let status = unsafe { sys::slic3r_slice(model.raw, config.raw, p.as_ptr(), &mut err) };
    unsafe { check_with_err(status, err) }
}

// ---- Slice progress callback ----
//
// The C side stores one global function pointer + opaque user_data;
// we own the closure on the Rust side and register a trampoline
// that re-enters into it. The closure lives behind a Mutex so it
// can be replaced from any thread; the slice itself is currently
// synchronous so the callback fires on the slice thread.

use std::sync::Mutex;

type ProgressClosure = Box<dyn FnMut(i32, &str) + Send>;

static PROGRESS_CALLBACK: Mutex<Option<ProgressClosure>> = Mutex::new(None);

/// Internal trampoline registered with the C side. Calls the
/// currently-installed Rust closure with the (percent, stage) tuple.
///
/// SAFETY: `stage` must point to a NUL-terminated C string valid
/// for the duration of the call (the C side guarantees this).
/// `_user_data` is ignored — the closure pointer lives in our
/// `PROGRESS_CALLBACK` static, not in user_data.
extern "C" fn progress_trampoline(percent: i32, stage: *const c_char, _user_data: *mut std::ffi::c_void) {
    let stage_str = if stage.is_null() {
        ""
    } else {
        // SAFETY: caller (C side) guarantees stage is NUL-terminated.
        match unsafe { CStr::from_ptr(stage) }.to_str() {
            Ok(s) => s,
            Err(_) => "",
        }
    };
    if let Ok(mut guard) = PROGRESS_CALLBACK.lock() {
        if let Some(cb) = guard.as_mut() {
            cb(percent, stage_str);
        }
    }
}

/// Register a Rust closure as the slice progress callback. The
/// closure is invoked synchronously from `slice` on the calling
/// thread for every progress tick libslic3r emits.
///
/// Replaces any previously registered callback. Pass `None` (via
/// [`clear_slice_progress`]) to unregister, after which libslic3r
/// emits silent slices.
pub fn set_slice_progress<F>(callback: F)
where
    F: FnMut(i32, &str) + Send + 'static,
{
    let mut guard = PROGRESS_CALLBACK.lock().expect("progress callback mutex");
    *guard = Some(Box::new(callback));
    drop(guard);
    // SAFETY: the trampoline + null user_data are static; the
    // C side stores them globally and serializes registration
    // against the slice thread via its own mutex.
    unsafe {
        sys::slic3r_set_slice_progress_cb(Some(progress_trampoline), ptr::null_mut());
    }
}

/// Clear the slice progress callback. Subsequent slices run silent
/// (no callback invocations and no stderr default).
pub fn clear_slice_progress() {
    // SAFETY: passing nullptr through the C API is the documented
    // "unregister" semantics.
    unsafe {
        sys::slic3r_set_slice_progress_cb(None, ptr::null_mut());
    }
    let mut guard = PROGRESS_CALLBACK.lock().expect("progress callback mutex");
    *guard = None;
}
