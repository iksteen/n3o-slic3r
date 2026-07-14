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
use std::sync::Mutex;

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

/// One-time process init. The shim is idempotent + mutex-guarded, so calling
/// this more than once is safe and — unlike a `Once` — a failed init (e.g. a
/// bad `TMPDIR`) stays retriable instead of caching the first outcome forever.
/// `resources_dir` is optional and only required for STEP import and font
/// embossing. `log_level` follows boost::log severity: 0=trace, 1=debug,
/// 2=info, 3=warning, 4=error, 5=fatal.
pub fn init(resources_dir: Option<&Path>, log_level: u32) -> Result<()> {
    let cstr = match resources_dir {
        Some(p) => Some(CString::new(p.to_string_lossy().as_bytes()).map_err(|_| Error {
            kind: ErrorKind::InvalidArg,
            message: Some("resources_dir has NUL".into()),
        })?),
        None => None,
    };
    let raw = cstr.as_ref().map_or(ptr::null(), |c| c.as_ptr());
    // SAFETY: pointer either null or valid for the duration of this call.
    let status = unsafe { sys::slic3r_init(raw, log_level) };
    check(status)
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

/// One side of a [`cut_mesh_deferred`] result: an indexed mesh (flattened xyz
/// vertices + triangle index triples). Empty (both vecs cleared) when the input
/// lay entirely on the other side of the plane.
#[derive(Debug, Clone, Default)]
pub struct CutHalf {
    pub vertices: Vec<f32>,
    pub indices: Vec<u32>,
    /// One MMU paint string per triangle (libslic3r `FacetsAnnotation`
    /// encoding), carried from the source mesh by [`cut_mesh_deferred`].
    /// `None` when the source carried no paint (or on connector/dowel halves).
    pub paint: Option<Vec<String>>,
}

impl CutHalf {
    /// True when this side carried no geometry (the mesh was wholly on the
    /// other side of the plane).
    pub fn is_empty(&self) -> bool {
        self.vertices.is_empty() || self.indices.is_empty()
    }
}

/// Copy one cut half out of shim-owned heap arrays into a [`CutHalf`], then free
/// the shim buffers. A null/zero half yields an empty `CutHalf`.
unsafe fn take_cut_half(verts: *mut f32, vcount: usize, idx: *mut u32, icount: usize) -> CutHalf {
    if verts.is_null() || idx.is_null() || vcount == 0 || icount == 0 {
        sys::slic3r_cut_mesh_free(verts, idx); // free whichever (if any) is non-null
        return CutHalf::default();
    }
    let vertices = std::slice::from_raw_parts(verts, vcount * 3).to_vec();
    let indices = std::slice::from_raw_parts(idx, icount * 3).to_vec();
    sys::slic3r_cut_mesh_free(verts, idx);
    CutHalf { vertices, indices, paint: None }
}

/// Copy a per-triangle paint string array (`n` = the half's triangle count) out
/// of the shim, then free it. `None` when the shim returned no paint.
unsafe fn take_paint(arr: *mut *mut c_char, n: usize) -> Option<Vec<String>> {
    if arr.is_null() {
        return None;
    }
    let out = (0..n)
        .map(|k| {
            let s = *arr.add(k);
            if s.is_null() {
                String::new()
            } else {
                CStr::from_ptr(s).to_string_lossy().into_owned()
            }
        })
        .collect();
    sys::slic3r_cut_connectors_free_paint(arr, n);
    Some(out)
}

/// Serialize a string vector the way libslic3r's `ConfigOptionStrings` does —
/// `;`-joined with cstyle quoting — so profile composition round-trips
/// `coStrings` vectors byte-identically to the engine (a lone empty element is
/// quoted so it survives the round-trip). Empty input yields `""`.
pub fn escape_strings_cstyle(strs: &[String]) -> Result<String> {
    let cstrings = strs
        .iter()
        .map(|s| cstring(s, "cstyle string"))
        .collect::<Result<Vec<_>>>()?;
    let ptrs: Vec<*const c_char> = cstrings.iter().map(|c| c.as_ptr()).collect();
    let mut out: *mut c_char = ptr::null_mut();
    // SAFETY: ptrs/cstrings live through the call; out is an owned heap string on OK.
    let status = unsafe { sys::slic3r_escape_strings_cstyle(ptrs.as_ptr(), ptrs.len(), &mut out) };
    check(status)?;
    // SAFETY: out is a non-null NUL-terminated heap string on OK; we own + free it.
    let s = unsafe { CStr::from_ptr(out).to_string_lossy().into_owned() };
    unsafe { sys::slic3r_string_free(out) };
    Ok(s)
}

/// Inverse of [`escape_strings_cstyle`]. Errors (`ParseValue`) on malformed
/// input (unterminated quote / trailing backslash) rather than truncating.
pub fn unescape_strings_cstyle(s: &str) -> Result<Vec<String>> {
    let cs = cstring(s, "cstyle input")?;
    let mut arr: *mut *mut c_char = ptr::null_mut();
    let mut count: usize = 0;
    // SAFETY: cs lives through the call; arr/count are out-params owned on OK.
    let status = unsafe { sys::slic3r_unescape_strings_cstyle(cs.as_ptr(), &mut arr, &mut count) };
    check(status)?;
    if arr.is_null() {
        return Ok(Vec::new());
    }
    // SAFETY: arr points to `count` non-null NUL-terminated heap strings on OK.
    let out = (0..count)
        .map(|k| unsafe { CStr::from_ptr(*arr.add(k)).to_string_lossy().into_owned() })
        .collect();
    unsafe { sys::slic3r_free_string_array(arr, count) };
    Ok(out)
}

/// Connector (joint) type — matches OrcaSlicer's `CutConnectorType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectorType {
    /// A solid peg integral to one half + a matching hole in the other.
    Plug,
    /// A free pin printed separately + a hole in each half.
    Dowel,
    /// Like Plug, with a click-fit bulge.
    Snap,
}

/// Connector profile along its axis — `CutConnectorStyle`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectorStyle {
    /// Straight (constant cross-section).
    Prism,
    /// Tapered.
    Frustum,
}

/// Connector cross-section — `CutConnectorShape`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectorShape {
    Triangle,
    Square,
    Hexagon,
    Circle,
}

// The enum codes match the C `slic3r_connector_*` values (0-based, contiguous).
impl ConnectorType {
    fn code(self) -> i32 {
        match self {
            ConnectorType::Plug => 0,
            ConnectorType::Dowel => 1,
            ConnectorType::Snap => 2,
        }
    }
}
impl ConnectorStyle {
    fn code(self) -> i32 {
        match self {
            ConnectorStyle::Prism => 0,
            ConnectorStyle::Frustum => 1,
        }
    }
}
impl ConnectorShape {
    fn code(self) -> i32 {
        match self {
            ConnectorShape::Triangle => 0,
            ConnectorShape::Square => 1,
            ConnectorShape::Hexagon => 2,
            ConnectorShape::Circle => 3,
        }
    }
}

/// One reassembly connector to bake into a cut. `pos` is a point on the cut
/// plane in the mesh's local frame; `radius`/`height` size it; `r_tol`/`h_tol`
/// widen the *hole* (mm); `z_angle` rotates the cross-section about the plane
/// normal (radians).
#[derive(Debug, Clone, Copy)]
pub struct Connector {
    pub pos: [f32; 3],
    pub radius: f32,
    pub height: f32,
    pub r_tol: f32,
    pub h_tol: f32,
    pub z_angle: f32,
    pub ty: ConnectorType,
    pub style: ConnectorStyle,
    pub shape: ConnectorShape,
}

/// Copy each dowel pin out of the shim-owned array-of-arrays into a `CutHalf`,
/// then free the whole group. Empty/absent → empty Vec.
unsafe fn take_dowels(
    dv: *mut *mut f32,
    dvc: *mut usize,
    di: *mut *mut u32,
    dtc: *mut usize,
    n: usize,
) -> Vec<CutHalf> {
    if dv.is_null() || di.is_null() || dvc.is_null() || dtc.is_null() || n == 0 {
        sys::slic3r_cut_connectors_free_dowels(dv, di, dvc, dtc, n);
        return Vec::new();
    }
    let mut out = Vec::with_capacity(n);
    for k in 0..n {
        let v = *dv.add(k);
        let vc = *dvc.add(k);
        let i = *di.add(k);
        let tc = *dtc.add(k);
        if !v.is_null() && !i.is_null() && vc > 0 && tc > 0 {
            out.push(CutHalf {
                vertices: std::slice::from_raw_parts(v, vc * 3).to_vec(),
                indices: std::slice::from_raw_parts(i, tc * 3).to_vec(),
                paint: None,
            });
        } else {
            out.push(CutHalf::default());
        }
    }
    sys::slic3r_cut_connectors_free_dowels(dv, di, dvc, dtc, n);
    out
}

/// One connector volume from [`cut_mesh_deferred`] — a peg or hole mesh in the
/// input frame, plus which half it attaches to and whether it's subtracted.
#[derive(Debug, Clone)]
pub struct CutModifier {
    pub vertices: Vec<f32>,
    pub indices: Vec<u32>,
    /// 0 = pos (upper) half, 1 = neg (lower) half.
    pub half: u8,
    /// `true` = NEGATIVE_VOLUME (hole), `false` = MODEL_PART (peg).
    pub negative: bool,
}

/// A deferred connector cut: the two plane-cut halves (paint preserved) plus the
/// connector geometry as separate [`CutModifier`] volumes (applied at slice time
/// as negative/positive volumes — no baked booleans) and the free dowel pins.
#[derive(Debug, Clone, Default)]
pub struct CutDeferred {
    pub pos: CutHalf,
    pub neg: CutHalf,
    pub modifiers: Vec<CutModifier>,
    pub dowels: Vec<CutHalf>,
}

/// Cut a mesh by a plane, returning reassembly connectors as separate volume
/// meshes ([`CutModifier`]) instead of baking them — the Orca-parity path: the
/// slice layer subtracts hole volumes per-layer in 2D, so the cut does no 3D
/// boolean. With no connectors this is a plain plane cut (plus paint carry) —
/// caps triangulated so both halves come back watertight.
/// `plane_origin`/`plane_normal` and each `Connector.pos` are in the mesh's
/// local frame.
pub fn cut_mesh_deferred(
    vertices: &[f32],
    indices: &[u32],
    plane_origin: [f32; 3],
    plane_normal: [f32; 3],
    connectors: &[Connector],
    paint: Option<&[String]>,
) -> Result<CutDeferred> {
    validate_indices(vertices, indices)?;
    let mut cf: Vec<f32> = Vec::with_capacity(connectors.len() * 8);
    let mut cn: Vec<i32> = Vec::with_capacity(connectors.len() * 3);
    for c in connectors {
        cf.extend_from_slice(&[
            c.pos[0], c.pos[1], c.pos[2], c.radius, c.height, c.r_tol, c.h_tol, c.z_angle,
        ]);
        cn.extend_from_slice(&[c.ty.code(), c.style.code(), c.shape.code()]);
    }
    let (cf_ptr, cn_ptr) = if connectors.is_empty() {
        (ptr::null(), ptr::null())
    } else {
        (cf.as_ptr(), cn.as_ptr())
    };
    let triangle_count = indices.len() / 3;
    // Paint must be one string per triangle, NUL-free — reject a mismatch
    // instead of silently dropping it (this param exists to carry stale paint
    // across an edit, so a silent drop is exactly the bug it should prevent).
    let paint_cstrings: Option<Vec<CString>> = match paint {
        Some(p) => {
            if p.len() != triangle_count {
                return Err(Error {
                    kind: ErrorKind::InvalidArg,
                    message: Some(format!(
                        "paint length {} != triangle count {triangle_count}",
                        p.len()
                    )),
                });
            }
            Some(
                p.iter()
                    .map(|s| {
                        CString::new(s.as_str()).map_err(|_| Error {
                            kind: ErrorKind::InvalidArg,
                            message: Some("paint string has interior NUL".into()),
                        })
                    })
                    .collect::<Result<Vec<_>>>()?,
            )
        }
        None => None,
    };
    let paint_ptrs: Option<Vec<*const c_char>> =
        paint_cstrings.as_ref().map(|v| v.iter().map(|c| c.as_ptr()).collect());
    let paint_ptr = paint_ptrs.as_ref().map_or(ptr::null(), |v| v.as_ptr());

    let mut pos_v = ptr::null_mut();
    let mut pos_vc = 0;
    let mut pos_i = ptr::null_mut();
    let mut pos_tc = 0;
    let mut pos_p: *mut *mut c_char = ptr::null_mut();
    let mut neg_v = ptr::null_mut();
    let mut neg_vc = 0;
    let mut neg_i = ptr::null_mut();
    let mut neg_tc = 0;
    let mut neg_p: *mut *mut c_char = ptr::null_mut();
    let mut mv: *mut *mut f32 = ptr::null_mut();
    let mut mvc: *mut usize = ptr::null_mut();
    let mut mi: *mut *mut u32 = ptr::null_mut();
    let mut mtc: *mut usize = ptr::null_mut();
    let mut mh: *mut i32 = ptr::null_mut();
    let mut mt: *mut i32 = ptr::null_mut();
    let mut mn: usize = 0;
    let mut dv: *mut *mut f32 = ptr::null_mut();
    let mut dvc: *mut usize = ptr::null_mut();
    let mut di: *mut *mut u32 = ptr::null_mut();
    let mut dtc: *mut usize = ptr::null_mut();
    let mut dn: usize = 0;
    let mut err: *mut c_char = ptr::null_mut();

    // SAFETY: inputs validated; connector streams are connector_count*{8,3} (or
    // null); every out pointer is a valid local we own on return.
    let status = unsafe {
        sys::slic3r_cut_mesh_deferred(
            vertices.as_ptr(),
            vertices.len() / 3,
            indices.as_ptr(),
            triangle_count,
            paint_ptr,
            plane_origin.as_ptr(),
            plane_normal.as_ptr(),
            cf_ptr,
            cn_ptr,
            connectors.len(),
            &mut pos_v,
            &mut pos_vc,
            &mut pos_i,
            &mut pos_tc,
            &mut pos_p,
            &mut neg_v,
            &mut neg_vc,
            &mut neg_i,
            &mut neg_tc,
            &mut neg_p,
            &mut mv,
            &mut mvc,
            &mut mi,
            &mut mtc,
            &mut mh,
            &mut mt,
            &mut mn,
            &mut dv,
            &mut dvc,
            &mut di,
            &mut dtc,
            &mut dn,
            &mut err,
        )
    };
    unsafe { check_with_err(status, err) }?;

    let mut pos = unsafe { take_cut_half(pos_v, pos_vc, pos_i, pos_tc) };
    let mut neg = unsafe { take_cut_half(neg_v, neg_vc, neg_i, neg_tc) };
    pos.paint = unsafe { take_paint(pos_p, pos_tc) };
    neg.paint = unsafe { take_paint(neg_p, neg_tc) };
    let modifiers = unsafe { take_mods(mv, mvc, mi, mtc, mh, mt, mn) };
    let dowels = unsafe { take_dowels(dv, dvc, di, dtc, dn) };
    Ok(CutDeferred { pos, neg, modifiers, dowels })
}

/// Copy the connector-volume array-of-arrays (+ half/type tags) out, then free.
unsafe fn take_mods(
    mv: *mut *mut f32,
    mvc: *mut usize,
    mi: *mut *mut u32,
    mtc: *mut usize,
    mh: *mut i32,
    mt: *mut i32,
    n: usize,
) -> Vec<CutModifier> {
    if mv.is_null() || mi.is_null() || mvc.is_null() || mtc.is_null() || mh.is_null()
        || mt.is_null()
        || n == 0
    {
        sys::slic3r_cut_connectors_free_mods(mv, mi, mvc, mtc, mh, mt, n);
        return Vec::new();
    }
    let mut out = Vec::with_capacity(n);
    for k in 0..n {
        let v = *mv.add(k);
        let vc = *mvc.add(k);
        let i = *mi.add(k);
        let tc = *mtc.add(k);
        if !v.is_null() && !i.is_null() && vc > 0 && tc > 0 {
            out.push(CutModifier {
                vertices: std::slice::from_raw_parts(v, vc * 3).to_vec(),
                indices: std::slice::from_raw_parts(i, tc * 3).to_vec(),
                half: *mh.add(k) as u8,
                negative: *mt.add(k) == 1,
            });
        }
    }
    sys::slic3r_cut_connectors_free_mods(mv, mi, mvc, mtc, mh, mt, n);
    out
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
/// `contours` is one **convex** footprint per item (>= 3 points, mm); `excludes`
/// are axis-aligned no-go regions `[minx, miny, maxx, maxy]` (mm) the nester
/// keeps clear (e.g. AMS feed zones, the wipe tower); `bed` is `[width, height]`
/// (mm, origin at 0,0); `min_dist` is the minimum gap between items (mm);
/// `allow_rotations` lets the nester try discrete rotations;
/// `align_to_y_axis` biases placement so items' long sides align with Y
/// (OrcaSlicer enables it for i3/bed-slinger structures). The excludes are
/// per-plate obstacles present on every bed, so each is reserved on every bed
/// the pack might spill onto — pass `bed_count` as the worst-case bed bound
/// (e.g. the item count; clamped to >= 1). Returns a placement per item in the
/// same order. Pure computation — no init or model handle required; may run
/// multithreaded (TBB).
pub fn arrange(
    contours: &[Vec<[f64; 2]>],
    excludes: &[[f64; 4]],
    bed_count: usize,
    bed: [f64; 2],
    min_dist: f64,
    allow_rotations: bool,
    align_to_y_axis: bool,
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
    let excl_flat: Vec<f64> = excludes.iter().flat_map(|r| r.iter().copied()).collect();
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
            if excludes.is_empty() {
                ptr::null()
            } else {
                excl_flat.as_ptr()
            },
            excludes.len(),
            bed_count,
            bed[0],
            bed[1],
            min_dist,
            allow_rotations as i32,
            align_to_y_axis as i32,
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
    fn contains(self, other: Self) -> bool {
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
    pub fn is_sla_material(self) -> bool {
        self.contains(Self::SLA_MATERIAL)
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

/// Preset-bucket classification — which OrcaSlicer preset tab owns an option.
///
/// Every FFF config-key belongs to exactly one bucket — Printer, Filament, or
/// Process. The partitioning comes from libslic3r's `Preset::print_options()` /
/// `filament_options()` / `printer_options()` (the last unions the
/// machine-limits + per-extruder/nozzle keys); the FFI computes it C++-side in
/// `DefCache::build`, exactly like `scope`. See
/// `docs/dev/orcaslicer-settings-classification.md` for the upstream rationale.
///
/// Some metadata keys (`compatible_printers`, `inherits`, …) appear in more
/// than one preset vector; they resolve to `None` (`bucket_of` returns `None`)
/// — correct UX since they're not user-editable settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptBucket {
    Printer,
    Filament,
    Process,
}

impl OptBucket {
    /// Decode the C `slic3r_opt_bucket` value; `SLIC3R_BUCKET_NONE` and any
    /// unknown value map to `None`.
    fn from_raw(v: u32) -> Option<OptBucket> {
        // `as u32`: the SLIC3R_BUCKET_* constants are u32 under GCC but i32
        // under MSVC; the values are small positive, so the cast is lossless.
        match v {
            x if x == sys::SLIC3R_BUCKET_PRINTER as u32 => Some(OptBucket::Printer),
            x if x == sys::SLIC3R_BUCKET_FILAMENT as u32 => Some(OptBucket::Filament),
            x if x == sys::SLIC3R_BUCKET_PROCESS as u32 => Some(OptBucket::Process),
            _ => None,
        }
    }
}

/// Bucket for an option key, or `None` for keys not owned by a single preset
/// tab (metadata, SLA-only, internal). Backed by the FFI-computed buckets on
/// [`option_defs`], cached on first call. Call after [`init`].
pub fn bucket_of(key: &str) -> Option<OptBucket> {
    static BUCKET_BY_KEY: std::sync::OnceLock<std::collections::HashMap<String, OptBucket>> =
        std::sync::OnceLock::new();
    if let Some(map) = BUCKET_BY_KEY.get() {
        return map.get(key).copied();
    }
    let defs = option_defs_cached();
    if defs.is_empty() {
        // Pre-init: the option table isn't populated. Return None transiently
        // rather than caching an empty map, which would classify every key as
        // bucketless for the process lifetime.
        return None;
    }
    let map: std::collections::HashMap<String, OptBucket> = defs
        .iter()
        .filter_map(|d| d.bucket.map(|b| (d.key.clone(), b)))
        .collect();
    let _ = BUCKET_BY_KEY.set(map);
    BUCKET_BY_KEY.get().and_then(|m| m.get(key).copied())
}

mod option_display_order;
pub use option_display_order::display_order_of;

mod option_printer_pages;
pub use option_printer_pages::{
    filament_line_of, filament_page_of, filament_subgroup_of, printer_line_of,
    printer_page_of, printer_subgroup_of,
};

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
    /// True when libslic3r's `gui_type` marks this option a color picker
    /// (`filament_colour`, `extruder_colour`, …) — the authoritative
    /// color classification, replacing any hand-curated key list.
    pub is_color: bool,
    pub enum_values: Vec<String>,
    pub enum_labels: Vec<String>,
    pub min: f64,
    pub max: f64,
    pub scope: OptScope,
    /// Preset bucket (Printer / Filament / Process). `None` for metadata
    /// keys that span all buckets (`compatible_printers`, `inherits`, …)
    /// or keys outside the FFF preset universe (SLA-only, internal scratch).
    pub bucket: Option<OptBucket>,
    /// True when the key is in libslic3r's `m_extruder_option_keys` — the
    /// authoritative per-extruder set (options sized to the extruder count,
    /// one editor rendered per toolhead). Straight from `print_config_def`,
    /// not a scraped list.
    pub per_extruder: bool,
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
                is_color: raw.is_color != 0,
                enum_values,
                enum_labels,
                min: raw.min,
                max: raw.max,
                scope: OptScope(raw.scope),
                bucket: OptBucket::from_raw(raw.bucket),
                per_extruder: raw.per_extruder != 0,
            }
        }
    }
}

/// All registered options. Call after [`init`].
pub fn option_defs() -> Vec<OptionDef> {
    // SAFETY: shim guarantees thread-safe read after init.
    let count = unsafe { sys::slic3r_option_def_count() };
    let mut out = Vec::with_capacity(count);
    let mut raw: sys::slic3r_option_def_t = Default::default();
    for i in 0..count {
        let status = unsafe { sys::slic3r_option_def_at(i, &mut raw) };
        if status == sys::SLIC3R_OK {
            out.push(OptionDef::from_raw(&raw));
        }
    }
    out
}

/// The full option table, marshalled from the shim once and cached for
/// the process lifetime — the table is immutable after [`init`], and
/// panel surfaces read it repeatedly, so this avoids re-marshalling the
/// ~600 entries per call. Only a non-empty table is cached, so a
/// pre-`init` call returns a transient empty slice without poisoning it.
pub fn option_defs_cached() -> &'static [OptionDef] {
    static CACHE: std::sync::OnceLock<Vec<OptionDef>> = std::sync::OnceLock::new();
    if let Some(defs) = CACHE.get() {
        return defs;
    }
    let defs = option_defs();
    if defs.is_empty() {
        return &[];
    }
    // A concurrent caller may win the race; either way we return the
    // stored table.
    let _ = CACHE.set(defs);
    CACHE.get().map(Vec::as_slice).unwrap_or(&[])
}

/// Look up a single option by key.
pub fn option_def(key: &str) -> Result<OptionDef> {
    let ckey = CString::new(key).map_err(|_| Error {
        kind: ErrorKind::InvalidArg,
        message: Some("key contains NUL".into()),
    })?;
    let mut raw: sys::slic3r_option_def_t = Default::default();
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

    /// Build one `ModelObject` in memory from raw buffers and append it to the
    /// model — the file-less equivalent of loading a single object from a
    /// `.3mf`. Lets the slice path hand geometry straight to libslic3r without
    /// writing/parsing a temp file.
    ///
    /// - `verts`: flat object-local XYZ (3 floats per vertex).
    /// - `indices`: flat triangle triples (3 indices per triangle).
    /// - `transform`: 4x4 object->world matrix, column-major (glam/Eigen order).
    /// - `extruder`: 1-based base filament index, set on the object config.
    /// - `paint_hex`: per-triangle BBS MMU color paint hex strings. Pass an
    ///   empty slice for none; otherwise one entry per triangle (`""` = none).
    /// - `support_hex`: per-triangle BBS support enforcer/blocker paint hex
    ///   strings, same shape as `paint_hex`. Empty slice for none.
    /// - `overrides`: per-object config overrides as `(key, value)` pairs,
    ///   applied through libslic3r's schema deserializer (unknown keys skipped).
    #[allow(clippy::too_many_arguments)]
    pub fn add_object(
        &mut self,
        name: &str,
        verts: &[f32],
        indices: &[u32],
        transform: &[f64; 16],
        extruder: i32,
        paint_hex: &[String],
        support_hex: &[String],
        overrides: &[(String, String)],
    ) -> Result<()> {
        let cname = cstring(name, "name")?;
        validate_indices(verts, indices)?;
        validate_paint(indices, paint_hex)?;
        validate_paint(indices, support_hex)?;

        // Own the CStrings for the lifetime of the call; the C side reads the
        // derived pointer arrays but does not retain them. An empty Vec's
        // `as_ptr()` is valid and the C side never derefs it when the count is 0.
        let strs = ObjectStrings::marshal(paint_hex, support_hex, overrides)?;
        let paint_ptrs = strs.paint_ptrs();
        let support_ptrs = strs.support_ptrs();
        let key_ptrs = strs.key_ptrs();
        let val_ptrs = strs.val_ptrs();

        let mut err: *mut c_char = ptr::null_mut();
        // SAFETY: self.raw is a live model handle; verts/indices point to
        // vcount*3 / tcount*3 valid elements; transform points to 16 doubles;
        // the CStrings and their pointer vecs outlive the call (the C side does
        // not retain them); err is an out-param we own on non-null return.
        let status = unsafe {
            sys::slic3r_model_add_object(
                self.raw,
                cname.as_ptr(),
                verts.as_ptr(),
                verts.len() / 3,
                indices.as_ptr(),
                indices.len() / 3,
                transform.as_ptr(),
                extruder,
                paint_ptrs.as_ptr(),
                paint_ptrs.len(),
                support_ptrs.as_ptr(),
                support_ptrs.len(),
                key_ptrs.as_ptr(),
                val_ptrs.as_ptr(),
                key_ptrs.len(),
                &mut err,
            )
        };
        let result = unsafe { check_with_err(status, err) };
        // Keep the owning CStrings alive until after the FFI call returns.
        drop(strs);
        result
    }

    /// Create an empty multi-volume group object (one `ModelObject` + identity
    /// instance) and return its index for the [`Model::add_volume`] calls that
    /// follow. Build a grouped object in-memory with `add_group` + one
    /// `add_volume` per member, instead of round-tripping a `.3mf`.
    pub fn add_group(&mut self, name: &str) -> Result<usize> {
        let cname = cstring(name, "name")?;
        let mut index: usize = 0;
        let mut err: *mut c_char = ptr::null_mut();
        // SAFETY: self.raw is a live model handle; cname lives through the call;
        // index + err are out-params we own on return.
        let status =
            unsafe { sys::slic3r_model_add_group(self.raw, cname.as_ptr(), &mut index, &mut err) };
        unsafe { check_with_err(status, err) }?;
        Ok(index)
    }

    /// Append one `ModelVolume` (from raw buffers) to the group object at
    /// `object_index` (from [`Model::add_group`]). Buffers / paint / overrides
    /// match [`Model::add_object`], except `transform` is the volume->world
    /// placement (composed onto the volume's centering compensation, matching a
    /// `.3mf` round-trip) and `extruder` + `overrides` are set on the *volume*
    /// config — each group member prints with its own filament.
    #[allow(clippy::too_many_arguments)]
    pub fn add_volume(
        &mut self,
        object_index: usize,
        name: &str,
        verts: &[f32],
        indices: &[u32],
        transform: &[f64; 16],
        extruder: i32,
        volume_type: VolumeType,
        paint_hex: &[String],
        support_hex: &[String],
        overrides: &[(String, String)],
    ) -> Result<()> {
        let cname = cstring(name, "name")?;
        validate_indices(verts, indices)?;
        validate_paint(indices, paint_hex)?;
        validate_paint(indices, support_hex)?;
        let strs = ObjectStrings::marshal(paint_hex, support_hex, overrides)?;
        let paint_ptrs = strs.paint_ptrs();
        let support_ptrs = strs.support_ptrs();
        let key_ptrs = strs.key_ptrs();
        let val_ptrs = strs.val_ptrs();

        let mut err: *mut c_char = ptr::null_mut();
        // SAFETY: self.raw is a live model handle; verts/indices point to
        // vcount*3 / tcount*3 valid elements; transform points to 16 doubles;
        // the CStrings + their pointer vecs outlive the call (the C side does
        // not retain them, and an empty Vec's `as_ptr()` is fine — count is 0);
        // err is an out-param we own on non-null return.
        let status = unsafe {
            sys::slic3r_model_add_volume(
                self.raw,
                object_index,
                cname.as_ptr(),
                verts.as_ptr(),
                verts.len() / 3,
                indices.as_ptr(),
                indices.len() / 3,
                transform.as_ptr(),
                extruder,
                volume_type as i32,
                paint_ptrs.as_ptr(),
                paint_ptrs.len(),
                support_ptrs.as_ptr(),
                support_ptrs.len(),
                key_ptrs.as_ptr(),
                val_ptrs.as_ptr(),
                key_ptrs.len(),
                &mut err,
            )
        };
        let result = unsafe { check_with_err(status, err) };
        drop(strs);
        result
    }
}

/// A volume's role in its object — matches libslic3r `ModelVolumeType`. A
/// `Negative` volume is subtracted per-layer in 2D at slice time (a deferred
/// cut-connector hole); a peg is a `Part` volume of the same object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VolumeType {
    Part = 0,
    Negative = 1,
}

/// Owned `CString`s for one `add_object` / `add_volume` call. The C side reads
/// the derived `*const c_char` arrays but does not retain them, so the owners
/// must outlive the FFI call — keep this on the stack across it.
struct ObjectStrings {
    paint: Vec<CString>,
    support: Vec<CString>,
    keys: Vec<CString>,
    vals: Vec<CString>,
}

impl ObjectStrings {
    fn marshal(
        paint_hex: &[String],
        support_hex: &[String],
        overrides: &[(String, String)],
    ) -> Result<Self> {
        let paint = paint_hex
            .iter()
            .map(|s| cstring(s, "paint hex"))
            .collect::<Result<_>>()?;
        let support = support_hex
            .iter()
            .map(|s| cstring(s, "support hex"))
            .collect::<Result<_>>()?;
        let mut keys = Vec::with_capacity(overrides.len());
        let mut vals = Vec::with_capacity(overrides.len());
        for (k, v) in overrides {
            keys.push(cstring(k, "override key")?);
            vals.push(cstring(v, "override value")?);
        }
        Ok(Self {
            paint,
            support,
            keys,
            vals,
        })
    }

    fn paint_ptrs(&self) -> Vec<*const c_char> {
        self.paint.iter().map(|c| c.as_ptr()).collect()
    }
    fn support_ptrs(&self) -> Vec<*const c_char> {
        self.support.iter().map(|c| c.as_ptr()).collect()
    }
    fn key_ptrs(&self) -> Vec<*const c_char> {
        self.keys.iter().map(|c| c.as_ptr()).collect()
    }
    fn val_ptrs(&self) -> Vec<*const c_char> {
        self.vals.iter().map(|c| c.as_ptr()).collect()
    }
}

/// `CString::new` with an `InvalidArg` error naming the offending field.
fn cstring(s: &str, what: &str) -> Result<CString> {
    CString::new(s).map_err(|_| Error {
        kind: ErrorKind::InvalidArg,
        message: Some(format!("{what} has NUL")),
    })
}

/// Bounds-check triangle indices before they reach libslic3r — it indexes the
/// vertex array unchecked (convex-hull / face-normal passes), so an out-of-range
/// index is an out-of-bounds read inside the engine. Same guard as `orient_mesh`.
fn validate_indices(verts: &[f32], indices: &[u32]) -> Result<()> {
    let vertex_count = verts.len() / 3;
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
    Ok(())
}

/// Reject malformed MMU paint at the boundary — the same "libslic3r reads it
/// unchecked" rationale as `validate_indices`. libslic3r's paint deserializer
/// (`TriangleSelector::set_triangle_from_string`) only `debug`-asserts on a
/// truncated bitstream, so a crafted string reads past its end in release. A
/// well-formed paint array is either empty (no paint) or has exactly one hex
/// string per triangle. The charset is UPPERCASE hex only: libslic3r decodes
/// `0-9`/`A-F` and silently zeroes anything else in release (`Model.cpp`
/// `set_triangle_from_string`), so accepting `a-f` here would validate a
/// different bitstream than the one libslic3r builds — the exact OOB this
/// guards. The shim's structural walk guards the split-tree; this catches the
/// cheap cases up front.
fn validate_paint(indices: &[u32], paint_hex: &[String]) -> Result<()> {
    if paint_hex.is_empty() {
        return Ok(());
    }
    let triangle_count = indices.len() / 3;
    if paint_hex.len() != triangle_count {
        return Err(Error {
            kind: ErrorKind::InvalidArg,
            message: Some(format!(
                "paint length {} != triangle count {triangle_count}",
                paint_hex.len()
            )),
        });
    }
    for (i, s) in paint_hex.iter().enumerate() {
        if let Some(c) = s.chars().find(|c| !matches!(c, '0'..='9' | 'A'..='F')) {
            return Err(Error {
                kind: ErrorKind::InvalidArg,
                message: Some(format!("paint[{i}] has non-uppercase-hex char {c:?}")),
            });
        }
    }
    Ok(())
}

impl Drop for Model {
    fn drop(&mut self) {
        // SAFETY: raw was returned by slic3r_model_new and we have unique ownership.
        unsafe { sys::slic3r_model_free(self.raw) };
    }
}

// ---- Support paint session ----

/// Brush shape for [`PaintSession`] strokes — matches libslic3r's `CursorType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrushKind {
    /// Screen-projected circle: paints front-facing triangles under the cursor.
    Circle = 0,
    /// 3D sphere: paints every triangle within the radius, front and back.
    Sphere = 1,
}

/// Support-paint state per triangle — matches libslic3r `EnforcerBlockerType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaintState {
    /// Erase back to unpainted.
    None = 0,
    /// Force support here.
    Enforcer = 1,
    /// Block support here.
    Blocker = 2,
}

/// Stateful enforcer/blocker brush over one mesh, wrapping libslic3r's
/// `TriangleSelector` (sub-triangle splitting, exact Orca semantics). Build with
/// [`PaintSession::new`], mutate with [`stroke`](Self::stroke) /
/// [`fill`](Self::fill) / [`undo`](Self::undo), then read back the per-triangle
/// paint strings with [`serialize`](Self::serialize) or the tessellated overlay
/// mesh with [`facets`](Self::facets).
pub struct PaintSession {
    raw: *mut sys::slic3r_paint_session_t,
}

// SAFETY: the handle owns no thread-affine state; the shim is single-threaded
// per handle. Same contract as `Model` — callers don't share a `&mut` across
// threads without external synchronization.
unsafe impl Send for PaintSession {}

/// Take ownership of a shim error message pointer (or null), freeing it.
unsafe fn take_err(err: *mut c_char) -> Option<String> {
    if err.is_null() {
        return None;
    }
    let s = CStr::from_ptr(err).to_string_lossy().into_owned();
    sys::slic3r_string_free(err);
    Some(s)
}

impl PaintSession {
    /// Open a session over one mesh (`vertices`: 3 floats/vertex, `indices`: 3
    /// u32/triangle). `paint` seeds the existing support paint — pass an empty
    /// slice for none, otherwise one hex string per triangle (`""` = unpainted).
    pub fn new(vertices: &[f32], indices: &[u32], paint: &[String]) -> Result<Self> {
        validate_indices(vertices, indices)?;
        validate_paint(indices, paint)?;
        let strs: Vec<CString> = paint
            .iter()
            .map(|s| cstring(s, "paint hex"))
            .collect::<Result<_>>()?;
        let ptrs: Vec<*const c_char> = strs.iter().map(|c| c.as_ptr()).collect();
        let mut err: *mut c_char = ptr::null_mut();
        // SAFETY: buffers are length-validated; the CStrings + ptr vec outlive
        // the call; err is an out-param we own on a null return.
        let raw = unsafe {
            sys::slic3r_paint_session_new(
                vertices.as_ptr(),
                vertices.len() / 3,
                indices.as_ptr(),
                indices.len() / 3,
                ptrs.as_ptr(),
                ptrs.len(),
                &mut err,
            )
        };
        drop(strs);
        if raw.is_null() {
            return Err(Error {
                kind: ErrorKind::InvalidArg,
                message: unsafe { take_err(err) },
            });
        }
        Ok(Self { raw })
    }

    /// Apply one brush stroke centered at mesh-local `hit` (the ray/mesh
    /// intersection), camera at mesh-local `camera`, `trafo` the 4x4 column-major
    /// mesh->world matrix; `radius` is world mm. `facet` is the unsplit triangle
    /// the hit lies on. Set `push_undo` on the first sample of a drag so the
    /// whole drag collapses to one undo step.
    #[allow(clippy::too_many_arguments)]
    pub fn stroke(
        &mut self,
        facet: i32,
        hit: [f32; 3],
        camera: [f32; 3],
        trafo: &[f64; 16],
        radius: f32,
        brush: BrushKind,
        state: PaintState,
        push_undo: bool,
    ) -> Result<()> {
        let mut err: *mut c_char = ptr::null_mut();
        // SAFETY: self.raw is live; hit/camera are 3 floats, trafo 16 doubles;
        // err is an out-param we own on non-null return.
        let status = unsafe {
            sys::slic3r_paint_session_stroke(
                self.raw,
                facet,
                hit.as_ptr(),
                camera.as_ptr(),
                trafo.as_ptr(),
                radius,
                brush as u32,
                state as u32,
                push_undo as i32,
                &mut err,
            )
        };
        unsafe { check_with_err(status, err) }
    }

    /// Smart fill from mesh-local `hit` on `facet`: flood the connected region
    /// whose adjacent-facet angles stay within `angle_deg`, then paint it `state`.
    pub fn fill(
        &mut self,
        facet: i32,
        hit: [f32; 3],
        trafo: &[f64; 16],
        angle_deg: f32,
        state: PaintState,
        push_undo: bool,
    ) -> Result<()> {
        let mut err: *mut c_char = ptr::null_mut();
        // SAFETY: as `stroke`.
        let status = unsafe {
            sys::slic3r_paint_session_fill(
                self.raw,
                facet,
                hit.as_ptr(),
                trafo.as_ptr(),
                angle_deg,
                state as u32,
                push_undo as i32,
                &mut err,
            )
        };
        unsafe { check_with_err(status, err) }
    }

    /// Undo the last stroke/fill. Returns `true` if a snapshot was restored,
    /// `false` when the in-session undo stack was empty.
    pub fn undo(&mut self) -> bool {
        // SAFETY: self.raw is a live handle.
        unsafe { sys::slic3r_paint_session_undo(self.raw) != 0 }
    }

    /// Read the per-triangle support-paint hex strings (`""` = unpainted) — the
    /// form persisted in the project and fed into slicing.
    pub fn serialize(&self) -> Result<Vec<String>> {
        let mut arr: *mut *mut c_char = ptr::null_mut();
        let mut count: usize = 0;
        let mut err: *mut c_char = ptr::null_mut();
        // SAFETY: out-params we own on OK.
        let status =
            unsafe { sys::slic3r_paint_session_serialize(self.raw, &mut arr, &mut count, &mut err) };
        unsafe { check_with_err(status, err) }?;
        // SAFETY: on OK, arr is a shim-owned array of `count` C strings; own+free.
        let out = (0..count)
            .map(|k| unsafe {
                let p = *arr.add(k);
                if p.is_null() {
                    String::new()
                } else {
                    CStr::from_ptr(p).to_string_lossy().into_owned()
                }
            })
            .collect();
        unsafe { sys::slic3r_free_string_array(arr, count) };
        Ok(out)
    }

    /// Read the tessellated (split-triangle) facets currently in `state` as a
    /// mesh-local indexed mesh `(vertices, indices)` for a viewport overlay.
    /// Empty (both vecs cleared) when nothing is painted `state`.
    pub fn facets(&self, state: PaintState) -> Result<(Vec<f32>, Vec<u32>)> {
        let mut verts: *mut f32 = ptr::null_mut();
        let mut vcount: usize = 0;
        let mut idx: *mut u32 = ptr::null_mut();
        let mut tcount: usize = 0;
        let mut err: *mut c_char = ptr::null_mut();
        // SAFETY: out-params we own on OK.
        let status = unsafe {
            sys::slic3r_paint_session_facets(
                self.raw,
                state as u32,
                &mut verts,
                &mut vcount,
                &mut idx,
                &mut tcount,
                &mut err,
            )
        };
        unsafe { check_with_err(status, err) }?;
        if verts.is_null() || idx.is_null() || vcount == 0 || tcount == 0 {
            // SAFETY: frees whichever (if any) is non-null.
            unsafe { sys::slic3r_cut_mesh_free(verts, idx) };
            return Ok((Vec::new(), Vec::new()));
        }
        // SAFETY: on OK with non-null, the buffers hold vcount*3 / tcount*3
        // elements we own; copy then free.
        let vertices = unsafe { std::slice::from_raw_parts(verts, vcount * 3).to_vec() };
        let indices = unsafe { std::slice::from_raw_parts(idx, tcount * 3).to_vec() };
        unsafe { sys::slic3r_cut_mesh_free(verts, idx) };
        Ok((vertices, indices))
    }
}

impl Drop for PaintSession {
    fn drop(&mut self) {
        // SAFETY: raw was returned by slic3r_paint_session_new; unique ownership.
        unsafe { sys::slic3r_paint_session_free(self.raw) };
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
    // A panic must not unwind across the C ABI (UB). Swallow it — a buggy
    // progress callback degrades to a missed tick, not a crashed slice.
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| cb(percent, stage_str)));
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

/// The outcome of a slice attempt: the advisory `warnings` libslic3r
/// reported (zero or more messages — e.g. a mismatched-filament-shrinkage
/// warning), paired with the slice `result` (the tower mesh on success, or
/// the error). Fatal errors abort the slice and come back as the `Err` of
/// `result`, not as a warning. Warnings are computed before `process()`
/// runs, so they're reported whether the slice then **succeeds or fails** —
/// letting the UI surface a warning even on a failed slice.
#[must_use]
pub struct SliceOutcome {
    pub warnings: Vec<String>,
    pub result: Result<Option<TowerMesh>>,
}

/// Slice, returning the advisory diagnostics alongside the result — see
/// [`SliceOutcome`].
///
/// Note: although `model` is `&Model`, the slice **mutates the underlying
/// C++ `Slic3r::Model` in place** — it normalizes each object's per-region
/// extruder selectors (`wall_filament` etc.) before building the `Print`
/// (see `slic3r_slice` in the shim). `Model` is a handle to external C++
/// state, so this isn't Rust UB, but re-slicing the same `Model` sees the
/// normalized config from the prior call. Pass a fresh `Model` if that
/// matters.
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
                warnings: Vec::new(),
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
    let warnings = warning.into_iter().collect();
    SliceOutcome { warnings, result }
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

/// Request cancellation of the in-flight slice (if any) from another thread.
/// The running `process()` aborts at its next checkpoint and the in-flight
/// [`slice_outcome`] returns an `Err`; the caller tells a user cancel from a
/// real failure by its own flag. No-op when no slice is running.
pub fn cancel_active_slice() {
    // SAFETY: slic3r_cancel only flips a process-global cancel flag guarded by
    // an internal mutex — safe to call any time, from any thread.
    unsafe { sys::slic3r_cancel() };
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
            // A panic must not unwind across the C ABI (UB). Catching it inside
            // the lock scope also avoids poisoning LOG_CALLBACK.
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| cb(level, msg_str)));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_object_builds_a_single_triangle_in_memory() {
        // No slic3r_init needed: add_object only constructs Model data.
        let mut model = Model::new().expect("model new");
        // Identity object->world transform (column-major; identity is symmetric).
        let identity: [f64; 16] = [
            1.0, 0.0, 0.0, 0.0, //
            0.0, 1.0, 0.0, 0.0, //
            0.0, 0.0, 1.0, 0.0, //
            0.0, 0.0, 0.0, 1.0,
        ];
        let verts: [f32; 9] = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        let indices: [u32; 3] = [0, 1, 2];
        model
            .add_object("tri", &verts, &indices, &identity, 1, &[], &[], &[])
            .expect("add_object should succeed");
    }

    #[test]
    fn validate_paint_length_and_charset() {
        let two_tris: [u32; 6] = [0, 1, 2, 0, 1, 2];
        // Empty = unpainted; one hex string per triangle = fine.
        assert!(validate_paint(&two_tris, &[]).is_ok());
        assert!(validate_paint(&two_tris, &["8".into(), "4".into()]).is_ok());

        let one_tri: [u32; 3] = [0, 1, 2];
        // Length mismatch and non-hex are both rejected at the boundary.
        assert_eq!(
            validate_paint(&one_tri, &["8".into(), "4".into()])
                .unwrap_err()
                .kind,
            ErrorKind::InvalidArg,
        );
        assert_eq!(
            validate_paint(&one_tri, &["8g".into()]).unwrap_err().kind,
            ErrorKind::InvalidArg,
        );
        // Lowercase hex is rejected: libslic3r zeroes it in release, so the
        // validated tree would differ from the one it deserializes.
        assert_eq!(
            validate_paint(&one_tri, &["a".into()]).unwrap_err().kind,
            ErrorKind::InvalidArg,
        );
    }

    #[test]
    fn add_object_paint_boundary_rejects_truncated_split() {
        // No init needed — add_object only constructs Model data.
        let mut model = Model::new().expect("model new");
        let identity: [f64; 16] = [
            1.0, 0.0, 0.0, 0.0, //
            0.0, 1.0, 0.0, 0.0, //
            0.0, 0.0, 1.0, 0.0, //
            0.0, 0.0, 0.0, 1.0,
        ];
        let verts: [f32; 9] = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        let indices: [u32; 3] = [0, 1, 2];

        // "8" = 0b1000: a leaf with state 2. Valid, accepted.
        model
            .add_object("ok", &verts, &indices, &identity, 1, &["8".into()], &[], &[])
            .expect("valid single-leaf paint");

        // Support paint takes the same well-formedness gate: a truncated split
        // in the support slot is an InvalidArg, not a crash.
        let err = model
            .add_object("bad_sup", &verts, &indices, &identity, 1, &[], &["1".into()], &[])
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::InvalidArg);

        // "1" = 0b0001: declares a split (low 2 bits = 1) but supplies no child
        // bits — hex and right length, so it clears the Rust gate; only the
        // shim's bounds-checked walk catches it. Must be a clean InvalidArg,
        // not an OOB read / crash.
        let err = model
            .add_object("bad", &verts, &indices, &identity, 1, &["1".into()], &[], &[])
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::InvalidArg);
    }

    #[test]
    fn add_group_then_volumes_builds_a_multivolume_object() {
        // No slic3r_init needed: these only construct Model data.
        let mut model = Model::new().expect("model new");
        let identity: [f64; 16] = [
            1.0, 0.0, 0.0, 0.0, //
            0.0, 1.0, 0.0, 0.0, //
            0.0, 0.0, 1.0, 0.0, //
            0.0, 0.0, 0.0, 1.0,
        ];
        let verts: [f32; 9] = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        let indices: [u32; 3] = [0, 1, 2];

        let idx = model.add_group("grp").expect("add_group should succeed");
        assert_eq!(idx, 0, "first object created → index 0");
        // Two volumes appended to the same group object, each its own extruder.
        model
            .add_volume(idx, "lower", &verts, &indices, &identity, 1, VolumeType::Part, &[], &[], &[])
            .expect("add_volume 1 should succeed");
        model
            .add_volume(idx, "upper", &verts, &indices, &identity, 2, VolumeType::Part, &[], &[], &[])
            .expect("add_volume 2 should succeed");

        // Out-of-range object index is rejected, not a crash.
        assert!(
            model
                .add_volume(99, "oops", &verts, &indices, &identity, 1, VolumeType::Part, &[], &[], &[])
                .is_err(),
            "add_volume past the last object must error",
        );
    }
}
