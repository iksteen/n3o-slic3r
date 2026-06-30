/* slic3r_ffi.h — flat C API over libslic3r for FFI consumers (Rust, etc.).
 *
 * Design: opaque handles, string-keyed config, runtime introspection over the
 * full ConfigOptionDef set. The consumer owns the configuration schema; this
 * surface only relays typed key/value pairs into libslic3r's own deserializer.
 *
 * Threading: each handle (slic3r_config_t*, slic3r_model_t*) is single-threaded.
 * slic3r_init() must be called once before any other function.
 *
 * License: AGPLv3 (matches the rest of OrcaSlicer).
 */
#ifndef SLIC3R_FFI_H
#define SLIC3R_FFI_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef enum {
    SLIC3R_OK              = 0,
    SLIC3R_ERR_INVALID_ARG = 1,
    SLIC3R_ERR_NOT_INIT    = 2,
    SLIC3R_ERR_UNKNOWN_KEY = 3,
    SLIC3R_ERR_PARSE_VALUE = 4,
    SLIC3R_ERR_IO          = 5,
    SLIC3R_ERR_VALIDATE    = 6,
    SLIC3R_ERR_SLICE       = 7,
    SLIC3R_ERR_INTERNAL    = 100
} slic3r_status;

/* Mirrors Slic3r::ConfigOptionType. Vector variants set the SLIC3R_OPT_VECTOR
 * bit so the consumer can branch on scalar vs. list without a giant switch. */
#define SLIC3R_OPT_VECTOR 0x4000
typedef enum {
    SLIC3R_OPT_NONE              = 0,
    SLIC3R_OPT_FLOAT             = 1,
    SLIC3R_OPT_FLOATS            = 1 | SLIC3R_OPT_VECTOR,
    SLIC3R_OPT_INT               = 2,
    SLIC3R_OPT_INTS              = 2 | SLIC3R_OPT_VECTOR,
    SLIC3R_OPT_STRING            = 3,
    SLIC3R_OPT_STRINGS           = 3 | SLIC3R_OPT_VECTOR,
    SLIC3R_OPT_PERCENT           = 4,
    SLIC3R_OPT_PERCENTS          = 4 | SLIC3R_OPT_VECTOR,
    SLIC3R_OPT_FLOAT_OR_PERCENT  = 5,
    SLIC3R_OPT_FLOATS_OR_PERCENTS = 5 | SLIC3R_OPT_VECTOR,
    SLIC3R_OPT_POINT             = 6,
    SLIC3R_OPT_POINTS            = 6 | SLIC3R_OPT_VECTOR,
    SLIC3R_OPT_POINT3            = 7,
    SLIC3R_OPT_BOOL              = 8,
    SLIC3R_OPT_BOOLS             = 8 | SLIC3R_OPT_VECTOR,
    SLIC3R_OPT_ENUM              = 9,
    SLIC3R_OPT_ENUMS             = 9 | SLIC3R_OPT_VECTOR
} slic3r_opt_type;

/* Mirrors Slic3r::ConfigOptionMode (Simple/Advanced/Expert/Develop). */
typedef enum {
    SLIC3R_MODE_SIMPLE   = 0,
    SLIC3R_MODE_ADVANCED = 1,
    SLIC3R_MODE_EXPERT   = 2,
    SLIC3R_MODE_DEVELOP  = 3
} slic3r_opt_mode;

/* Bitmask of the scopes an option can be set at. Scope is encoded
 * structurally in libslic3r by which static config class declares the
 * option (PrintObjectConfig, PrintRegionConfig, PrintConfig, the SLA
 * variants); we surface that as a bitmask so consumers can validate
 * "can this override be applied per-object" / "where does this value
 * go in the DynamicPrintConfig adapter layer" without grepping headers.
 *
 * An option can belong to multiple scopes (most commonly when an FFF
 * and an SLA class both declare the same key).
 *
 *   PRINT      project-level (PrintConfig + its parents
 *              MachineEnvelopeConfig and GCodeConfig).
 *   OBJECT     per-object (PrintObjectConfig); set via
 *              ModelObject::config.
 *   REGION     per-region/volume (PrintRegionConfig); set via
 *              ModelVolume::config or inherited from object scope.
 *   SLA_*      same idea for SLA workflows. */
typedef enum {
    SLIC3R_SCOPE_PRINT        = 1u << 0,
    SLIC3R_SCOPE_OBJECT       = 1u << 1,
    SLIC3R_SCOPE_REGION       = 1u << 2,
    SLIC3R_SCOPE_SLA_PRINT    = 1u << 3,
    SLIC3R_SCOPE_SLA_OBJECT   = 1u << 4,
    SLIC3R_SCOPE_SLA_MATERIAL = 1u << 5,
    SLIC3R_SCOPE_SLA_PRINTER  = 1u << 6
} slic3r_opt_scope;

/* Preset bucket — which OrcaSlicer preset tab owns this option, from
 * Preset::print_options() / filament_options() / printer_options() (the last
 * unions machine-limits + per-extruder/nozzle keys). Single-valued, unlike
 * scope: NONE for keys in zero or more-than-one bucket (per-preset metadata
 * like compatible_printers / inherits, or non-FFF / SLA-only keys). */
typedef enum {
    SLIC3R_BUCKET_NONE     = 0,
    SLIC3R_BUCKET_PRINTER  = 1,
    SLIC3R_BUCKET_FILAMENT = 2,
    SLIC3R_BUCKET_PROCESS  = 3
} slic3r_opt_bucket;

/* Borrowed view over a single ConfigOptionDef. All pointers point into
 * process-lifetime storage owned by libslic3r (the global print_config_def).
 * Strings are NUL-terminated UTF-8; nullable fields are NULL when absent. */
typedef struct {
    const char*         key;
    slic3r_opt_type     type;
    const char*         label;
    const char*         full_label;
    const char*         tooltip;
    const char*         category;
    const char*         sidetext;
    const char*         default_serialized;
    slic3r_opt_mode     mode;
    int                 readonly;     /* 0/1 */
    int                 multiline;    /* 0/1 */
    /* 1 when the option's gui_type is a color picker (e.g. filament_colour,
     * extruder_colour) — libslic3r's authoritative color classification. */
    int                 is_color;     /* 0/1 */
    /* For ENUM/ENUMS: parallel arrays of internal keys and display labels.
     * Both NULL/0 for non-enum types. */
    const char* const*  enum_values;
    const char* const*  enum_labels;
    size_t              enum_value_count;
    /* min/max as set in PrintConfig.cpp. libslic3r stores these as float;
     * we widen to double. No "has_min" flag — defaults are 0.0, so 0.0 is
     * ambiguous. Consumers wanting strict bounds checking should treat both
     * == 0.0 as "no range declared". */
    double              min;
    double              max;
    /* Bitmask of slic3r_opt_scope values. Zero means the option isn't
     * declared by any known static config class — currently unreachable
     * for keys in print_config_def but defensible as a default. */
    unsigned int        scope;
    /* Preset bucket (slic3r_opt_bucket value): which preset tab owns this
     * option. SLIC3R_BUCKET_NONE for metadata / non-FFF keys. */
    unsigned int        bucket;
} slic3r_option_def_t;

/* ---- Library lifecycle ---- */

/* One-time process init. Safe to call multiple times (subsequent calls no-op).
 * resources_dir: path to OrcaSlicer's resources/ directory. May be NULL if the
 *   caller never loads STEP files or uses font embossing (those features look
 *   up files under resources/). Pass it if you have it.
 * log_level: 0=trace, 1=debug, 2=info, 3=warning, 4=error, 5=fatal.
 *   Recommended: 3 (warning). 2 prints chatty progress to stderr. */
slic3r_status slic3r_init(const char* resources_dir, unsigned int log_level);

/* Static version banner. Pointer valid for process lifetime. */
const char* slic3r_version(void);

/* Free a heap string returned by this library (slic3r_config_get, error outs).
 * Safe to pass NULL. */
void slic3r_string_free(char* s);

/* ---- Option introspection ---- */

/* Number of options in the global print_config_def. */
size_t slic3r_option_def_count(void);

/* Fill *out for the option at index i (0..count-1). Returns
 * SLIC3R_ERR_INVALID_ARG on out-of-range. Option order is libslic3r's
 * internal map order; do not assume stability across upstream versions. */
slic3r_status slic3r_option_def_at(size_t i, slic3r_option_def_t* out);

/* Look up by key. SLIC3R_ERR_UNKNOWN_KEY if not present. */
slic3r_status slic3r_option_def_lookup(const char* key, slic3r_option_def_t* out);

/* ---- Config ---- */

typedef struct slic3r_config_t slic3r_config_t;

/* Allocate a DynamicPrintConfig seeded with FullPrintConfig defaults. */
slic3r_config_t* slic3r_config_new(void);
void             slic3r_config_free(slic3r_config_t* cfg);

/* Set an option. value is libslic3r's serialized form (same as JSON profile
 *   values): "0.2", "3", "0.4,0.4,0.4", "true", "concentric", "100%", etc.
 * Returns SLIC3R_ERR_UNKNOWN_KEY if key not in print_config_def,
 *   SLIC3R_ERR_PARSE_VALUE if libslic3r's deserializer rejects value. */
slic3r_status slic3r_config_set(slic3r_config_t* cfg,
                                 const char* key,
                                 const char* value);

/* Get an option's current serialized value. *out_value is heap-allocated;
 * caller must slic3r_string_free() it. */
slic3r_status slic3r_config_get(slic3r_config_t* cfg,
                                 const char* key,
                                 char** out_value);

/* Run libslic3r's cross-option validation (PrintConfig.cpp's validate()).
 * Returns SLIC3R_OK and *out_err = NULL on success.
 * Returns SLIC3R_ERR_VALIDATE and a heap-allocated, NUL-terminated message
 *   (joined first error) on failure; caller frees with slic3r_string_free.
 * out_err may be NULL to discard the message. */
slic3r_status slic3r_config_validate(slic3r_config_t* cfg, char** out_err);

/* ---- Model ---- */

typedef struct slic3r_model_t slic3r_model_t;

slic3r_model_t* slic3r_model_new(void);
void            slic3r_model_free(slic3r_model_t* model);

/* Load a model file. Format auto-detected from extension (3MF / STL / OBJ /
 * STEP / AMF). Replaces any previous contents of *model.
 * Adds a default instance for each ModelObject (LoadStrategy::AddDefaultInstances).
 * out_err may be NULL. If non-NULL and the call fails, *out_err receives a
 * heap-allocated message; caller frees with slic3r_string_free. */
slic3r_status slic3r_model_load(slic3r_model_t* model,
                                 const char* path,
                                 char** out_err);

/* Like slic3r_model_load, but also folds any printer/print/filament settings
 * embedded in the file into `config`. Currently meaningful for .3mf (and
 * .amf): the project's `Metadata/project_settings.config` is parsed and
 * applied on top of `config`'s existing values. Settings the file doesn't
 * mention are left untouched, so seeding `config` with FullPrintConfig
 * defaults (via slic3r_config_new) and then calling this merges 3MF
 * overrides on top.
 *
 * STL/OBJ/STEP files have no embedded config; for them this is identical
 * to slic3r_model_load with respect to `config` (left unchanged).
 *
 * Forward-compatibility substitution is enabled silently — older 3MFs with
 * renamed/removed option keys are accepted, with substitutions applied
 * without throwing. */
slic3r_status slic3r_model_load_with_config(slic3r_model_t* model,
                                             slic3r_config_t* config,
                                             const char* path,
                                             char** out_err);

/* Remap MMU color-painting (paint_color) filament states in place.
 *
 * For every painted ModelVolume, each per-face filament state `s` is
 * replaced with `perm[s]` (states >= perm_len, and any unpainted face, are
 * left unchanged). Used to reconcile painted filament indices with n3o's
 * per-object extruder remap on toolchanger printers, where the object's base
 * extruder is rewritten to a flat-slot index and the paint must follow.
 * State 0 (the object's own extruder) should map to itself. No-op on a model
 * with no painting. */
slic3r_status slic3r_model_remap_paint_filaments(slic3r_model_t* model,
                                                 const int32_t* perm,
                                                 size_t perm_len);

/* Build one ModelObject in-memory from raw buffers and append it to *model.
 * Mirrors what loading a .3mf object does, without the file round-trip.
 *   verts: flat object-local XYZ (vcount = number of vertices)
 *   indices: flat triangle triples (tcount = number of triangles)
 *   transform: 4x4 object->world, COLUMN-MAJOR (glam/Eigen native order)
 *   extruder: 1-based; sets ModelObject config["extruder"]
 *   paint_hex / paint_count: per-triangle BBS paint hex strings (paint_count
 *     == tcount when present, else 0); entries may be "" for unpainted faces.
 *   ovr_keys / ovr_vals / ovr_count: per-object config overrides as
 *     key/value strings, applied to the ModelObject config via the schema
 *     (set_deserialize) so they parse to the right option type.
 * out_err may be NULL. */
slic3r_status slic3r_model_add_object(
    slic3r_model_t* m, const char* name,
    const float* verts, size_t vcount,
    const uint32_t* indices, size_t tcount,
    const double transform[16], int extruder,
    const char* const* paint_hex, size_t paint_count,
    const char* const* ovr_keys, const char* const* ovr_vals, size_t ovr_count,
    char** out_err);

/* Create an empty multi-volume group object (one ModelObject + identity
 * instance) and append it to *model; *out_index (nullable) receives its index
 * for the slic3r_model_add_volume calls that follow. Use this + add_volume to
 * build a grouped object in-memory instead of round-tripping a .3mf — each
 * volume carries its own world transform (the group instance stays identity),
 * matching the .3mf writer's components shape. out_err may be NULL. */
slic3r_status slic3r_model_add_group(
    slic3r_model_t* m, const char* name, size_t* out_index, char** out_err);

/* Append one ModelVolume (from raw buffers) to model->objects[object_index]
 * (created by slic3r_model_add_group). Buffers/paint/overrides as in
 * slic3r_model_add_object, except:
 *   transform: 4x4 volume->world, COLUMN-MAJOR — composed onto add_volume's
 *     centering compensation exactly as the .3mf loader's component path, so
 *     the world placement matches a round-tripped .3mf.
 *   extruder + overrides: set on the *volume* config (each group member prints
 *     with its own filament), not the object config.
 *   volume_type: ModelVolumeType — 0 = MODEL_PART, 1 = NEGATIVE_VOLUME (subtracted
 *     per-layer in 2D at slice time, e.g. a deferred cut-connector hole). A peg
 *     is a positive MODEL_PART volume of the same object.
 * SLIC3R_ERR_INVALID_ARG if object_index is out of range. out_err may be NULL. */
slic3r_status slic3r_model_add_volume(
    slic3r_model_t* m, size_t object_index, const char* name,
    const float* verts, size_t vcount,
    const uint32_t* indices, size_t tcount,
    const double transform[16], int extruder, int volume_type,
    const char* const* paint_hex, size_t paint_count,
    const char* const* ovr_keys, const char* const* ovr_vals, size_t ovr_count,
    char** out_err);

/* ---- Slicing ---- */

/* Slice progress callback.
 *
 * Signature: invoked from inside slic3r_slice on libslic3r's slicing
 * thread (which is the caller's thread today — slice() is synchronous).
 * `percent` ranges 0..100; `stage` is libslic3r's human-readable
 * status text ("Generating perimeters", "Generating support material",
 * etc.) and lives only for the duration of the call (do NOT retain
 * the pointer beyond the callback's return). `user_data` is whatever
 * was passed alongside the callback in slic3r_slice; the FFI doesn't
 * interpret it.
 *
 * The callback is bound per slic3r_slice call (passed in as
 * progress_cb + progress_user_data parameters), so concurrent slice
 * runs each carry their own callback with no shared state. Pass
 * cb=NULL for a silent slice. */
typedef void (*slic3r_progress_fn_t)(int percent, const char* stage, void* user_data);

/* Slice model with config and write G-code to out_gcode_path.
 *
 * Pipeline:
 *   Print::apply(model, config) → Print::validate() → Print::process()
 *     → Print::export_gcode(out_gcode_path).
 *
 * For single-object STLs this slices at the origin. For 3MFs, the objects'
 * embedded transforms are honored.
 *
 * progress_cb (may be NULL) fires synchronously on every libslic3r
 * status tick. progress_user_data is opaque and passed through to the
 * callback verbatim. The callback is captured per-call — no global
 * registration state — so concurrent slice runs on distinct
 * (model, config) inputs each carry their own callback. (Whether
 * libslic3r itself is safe to run concurrently is a separate
 * question; the callback path is.)
 *
 * out_err may be NULL. If non-NULL and the call fails, *out_err receives a
 * heap-allocated message; caller frees with slic3r_string_free.
 *
 * out_warning may be NULL. On a *successful* slice that produced a non-fatal
 * validation warning (the advisory libslic3r's validate() reports through its
 * warning out-param — e.g. mismatched filament shrinkage), *out_warning
 * receives a heap-allocated message; caller frees with slic3r_string_free.
 * Set to NULL when there was no warning or the caller passes NULL.
 *
 * out_tower_* (all four nullable; all-or-nothing): on a successful slice
 * that generates a prime/wipe tower, receive the tower's exact mesh — the
 * rib/cone solid for toolchangers, a box for AMS purge towers — in
 * tower-local millimetres. *out_tower_vertices holds 3 floats per vertex,
 * *out_tower_indices holds 3 vertex indices per triangle; both are
 * heap-allocated and freed together with slic3r_tower_mesh_free. All are
 * set to NULL / 0 when the plate is single-material (no tower) or when the
 * caller passes NULL for the group.
 *
 * SLIC3R_ERR_VALIDATE: cross-option / object validation failed.
 * SLIC3R_ERR_SLICE:    exception thrown during process() or export_gcode(). */
slic3r_status slic3r_slice(slic3r_model_t* model,
                            slic3r_config_t* config,
                            const char* out_gcode_path,
                            slic3r_progress_fn_t progress_cb,
                            void* progress_user_data,
                            float** out_tower_vertices,
                            size_t* out_tower_vertex_count,
                            uint32_t** out_tower_indices,
                            size_t* out_tower_index_count,
                            char** out_err,
                            char** out_warning);

/* Request cancellation of the in-flight slic3r_slice (if any) from another
 * thread: flips the running Print's cancel flag, so process() aborts at its next
 * throw_if_canceled() checkpoint and slic3r_slice returns SLIC3R_ERR_SLICE.
 * No-op when no slice is running. Always SLIC3R_OK. */
slic3r_status slic3r_cancel(void);

/* Free the buffers returned in slic3r_slice's out_tower_* params. Safe to
 * call with NULL pointers (no-op). */
void slic3r_tower_mesh_free(float* vertices, uint32_t* indices);

/* Cut an indexed mesh by an arbitrary plane (wraps libslic3r::cut_mesh — the
 * engine behind OrcaSlicer's "Cut" tool — with caps triangulated, so both
 * halves come back watertight/printable).
 *
 * `vertices` is `vertex_count` xyz triples (3 floats each), `indices` is
 * `triangle_count` vertex-index triples (3 uint32 each). `plane_origin` is a
 * point on the plane and `plane_normal` its normal (xyz; need not be unit) —
 * BOTH in the same coordinate frame as `vertices` (the caller pre-transforms a
 * world plane into the mesh's local frame).
 *
 * Writes two halves: "pos" is the side the normal points toward, "neg" the
 * other. Each is heap-allocated as xyz-triple verts + uint32-triple indices via
 * the four out-pointers; counts are vertex/triangle counts (not float/uint
 * counts). EITHER half may come back empty (all four out-* set to NULL/0) when
 * the mesh lies entirely on the other side of the plane. Free every non-NULL
 * pair with slic3r_cut_mesh_free.
 *
 * On error writes *out_err (caller frees with slic3r_string_free) and returns
 * non-OK. Pure computation: touches no slic3r_model_t/config, no slic3r_init(). */
slic3r_status slic3r_cut_mesh(const float* vertices, size_t vertex_count,
                              const uint32_t* indices, size_t triangle_count,
                              const float plane_origin[3], const float plane_normal[3],
                              float** out_pos_vertices, size_t* out_pos_vertex_count,
                              uint32_t** out_pos_indices, size_t* out_pos_triangle_count,
                              float** out_neg_vertices, size_t* out_neg_vertex_count,
                              uint32_t** out_neg_indices, size_t* out_neg_triangle_count,
                              char** out_err);

/* Free one half's buffers returned by slic3r_cut_mesh. Safe with NULL (no-op). */
void slic3r_cut_mesh_free(float* vertices, uint32_t* indices);

/* Connector (joint) enums for slic3r_cut_mesh_connectors — match OrcaSlicer's
 * CutConnectorType / Style / Shape. */
typedef enum { SLIC3R_CONN_PLUG = 0, SLIC3R_CONN_DOWEL = 1, SLIC3R_CONN_SNAP = 2 } slic3r_connector_type;
typedef enum { SLIC3R_CONN_PRISM = 0, SLIC3R_CONN_FRUSTUM = 1 } slic3r_connector_style;
typedef enum { SLIC3R_CONN_TRIANGLE = 0, SLIC3R_CONN_SQUARE = 1,
               SLIC3R_CONN_HEXAGON = 2, SLIC3R_CONN_CIRCLE = 3 } slic3r_connector_shape;

/* Cut a mesh by a plane AND bake reassembly connectors (joints) into the
 * halves — the engine behind OrcaSlicer's "Cut" connectors, applied here via
 * mesh booleans so the result is plain printable meshes.
 *
 * Cut args are identical to slic3r_cut_mesh. Connectors are passed as flat
 * parallel arrays (the codebase's convention), `connector_count` entries:
 *   connector_floats: 8 per connector — pos[0],pos[1],pos[2] (a point on the
 *     LOCAL cut plane, same frame as plane_origin), radius, height,
 *     r_tolerance, h_tolerance (mm, widen the HOLE), z_angle (radians, the
 *     cross-section's rotation about the plane normal).
 *   connector_ints: 3 per connector — type, style, shape (enums above).
 * Pass NULL/0 for no connectors (then this == slic3r_cut_mesh).
 *
 * Per connector: a Plug/Snap adds a solid peg to the `neg` half and a matching
 * hole to the `pos` half; a Dowel cuts a hole in BOTH halves and emits a free
 * pin mesh. A connector whose boolean fails is skipped (logged) — the plain cut
 * still succeeds.
 *
 * MMU color paint: pass `in_paint` as `triangle_count` C strings (libslic3r
 * FacetsAnnotation per-triangle encoding; "" = unpainted), or NULL for an
 * unpainted mesh. When supplied, the paint is re-projected onto each kept half
 * (libslic3r's save/restore_painting spatial remap, which tolerates the cut +
 * connector booleans) and returned as `*out_pos_triangle_count` /
 * `*out_neg_triangle_count` strings via *out_pos_paint / *out_neg_paint — free
 * each with slic3r_cut_connectors_free_paint. NULL out when in_paint was NULL.
 * Dowel pins are fresh geometry and carry no paint.
 *
 * Outputs: pos/neg halves exactly as slic3r_cut_mesh (free with
 * slic3r_cut_mesh_free). Dowel pins come back as an array of `*out_dowel_count`
 * meshes (parallel arrays of vertex/index buffers + their counts); free the
 * whole group with slic3r_cut_connectors_free_dowels. On error writes *out_err
 * and returns non-OK. Pure computation; no slic3r_init(). */
slic3r_status slic3r_cut_mesh_connectors(
    const float* vertices, size_t vertex_count,
    const uint32_t* indices, size_t triangle_count,
    const char* const* in_paint,
    const float plane_origin[3], const float plane_normal[3],
    const float* connector_floats, const int32_t* connector_ints, size_t connector_count,
    float** out_pos_vertices, size_t* out_pos_vertex_count,
    uint32_t** out_pos_indices, size_t* out_pos_triangle_count,
    char*** out_pos_paint,
    float** out_neg_vertices, size_t* out_neg_vertex_count,
    uint32_t** out_neg_indices, size_t* out_neg_triangle_count,
    char*** out_neg_paint,
    float*** out_dowel_vertices, size_t** out_dowel_vertex_counts,
    uint32_t*** out_dowel_indices, size_t** out_dowel_triangle_counts,
    size_t* out_dowel_count,
    char** out_err);

/* Free a per-triangle paint string array from slic3r_cut_mesh_connectors (every
 * string + the outer array). Safe with NULL/0 (no-op). */
void slic3r_cut_connectors_free_paint(char** paint, size_t count);

/* Free the dowel array-of-arrays from slic3r_cut_mesh_connectors (every inner
 * buffer + the four outer arrays). Safe with NULL/0 (no-op). */
void slic3r_cut_connectors_free_dowels(
    float** dowel_vertices, uint32_t** dowel_indices,
    size_t* dowel_vertex_counts, size_t* dowel_triangle_counts, size_t dowel_count);

/* Log sink callback.
 *
 * Replaces libslic3r's stderr-only boost::log default with a
 * caller-supplied function. Fires on every log record emitted by
 * libslic3r through `BOOST_LOG_TRIVIAL(...)` — slice progress text,
 * configuration warnings, internal debug.
 *
 * `severity` mirrors boost::log::trivial's enum:
 *   0=trace, 1=debug, 2=info, 3=warning, 4=error, 5=fatal.
 * Records below the current severity filter (set via slic3r_init's
 * log_level argument) are dropped before they reach the callback —
 * the callback only sees what would have been printed.
 *
 * `message` is a NUL-terminated C string valid for the duration
 * of the call; do NOT retain the pointer. `user_data` is the
 * opaque pointer passed at registration.
 *
 * Threading: libslic3r emits log records from any thread it runs
 * on (today: the caller's thread, since slic3r_slice is synchronous
 * and parsing is single-threaded). The sink is `synchronous_sink`-
 * backed so concurrent emissions serialize internally; the callback
 * itself must be thread-safe.
 *
 * Registration is process-global. Pass cb=NULL to unregister. */
typedef void (*slic3r_log_fn_t)(int severity, const char* message, void* user_data);
void slic3r_set_log_sink(slic3r_log_fn_t cb, void* user_data);

/* Auto-orient a triangle mesh for minimal support material (wraps
 * Slic3r::orientation::orient — the engine behind OrcaSlicer's "Auto orient").
 *
 * Input is a raw indexed mesh in whatever coordinate frame the caller wants the
 * orientation computed in (the returned rotation is in that same frame):
 * `vertices` is `vertex_count` xyz triples (3 floats each), `indices` is
 * `triangle_count` vertex-index triples (3 uint32 each). `overhang_angle` is the
 * support threshold in degrees (e.g. 30..60); pass <= 0 for the engine default.
 *
 * On success writes the computed rotation into `out_quat_xyzw` as a unit
 * quaternion (x, y, z, w) — the rotation that brings the given mesh into the
 * support-minimizing orientation. Applying it (and settling onto the bed) is the
 * caller's job; this function only computes the rotation.
 *
 * Pure computation: touches no slic3r_model_t/config and does not require
 * slic3r_init(). May run multithreaded internally (TBB). */
slic3r_status slic3r_orient_mesh(const float* vertices, size_t vertex_count,
                                 const uint32_t* indices, size_t triangle_count,
                                 float overhang_angle, float out_quat_xyzw[4],
                                 char** out_err);

/* 2D nesting / auto-arrange (wraps Slic3r::arrangement::arrange — the engine
 * behind OrcaSlicer's "Arrange", on libnest2d). Packs a set of CONVEX 2D
 * footprints onto a rectangular bed, computing a translation + rotation for
 * each plus a logical bed index (0 = this bed, >0 = spilled onto an extra bed,
 * -1 = unplaced). All distances are in mm; the bed origin is (0, 0).
 *
 * Items are passed as flattened contours: `contours` holds xy pairs (2 doubles
 * each) for every item concatenated, and `contour_lengths` gives the point
 * count of each item (so item i occupies the next contour_lengths[i] pairs).
 * Each contour must be a convex polygon with >= 3 points (the caller's
 * footprint hull). `exclude_rects` are `exclude_count` axis-aligned no-go
 * regions (4 doubles each: minx, miny, maxx, maxy) the nester keeps items clear
 * of — pass NULL/0 for none. These are physical/per-plate obstacles (AMS feed
 * zones, the wipe tower) present on *every* bed, so they are reserved as fixed
 * obstacles on each of the first `bed_count` beds (pass the item count as the
 * worst-case bed bound; clamped to >= 1). `min_dist` is the minimum gap between
 * items (mm); `allow_rotations` (0/1) lets the nester try discrete rotations.
 *
 * Outputs are caller-allocated, indexed by the same item order:
 *   out_dx_dy:    2 doubles per item — the translation to apply (mm).
 *   out_rotation: 1 double per item — the rotation to apply (radians).
 *   out_bed_idx:  1 int per item — the logical bed it landed on.
 *
 * Pure computation: touches no slic3r_model_t/config and does not require
 * slic3r_init(). May run multithreaded internally (TBB). */
slic3r_status slic3r_arrange(const double* contours, const size_t* contour_lengths,
                             size_t item_count, const double* exclude_rects,
                             size_t exclude_count, size_t bed_count, double bed_w,
                             double bed_h, double min_dist, int allow_rotations,
                             double* out_dx_dy, double* out_rotation,
                             int* out_bed_idx, char** out_err);

#ifdef __cplusplus
}
#endif

#endif /* SLIC3R_FFI_H */
