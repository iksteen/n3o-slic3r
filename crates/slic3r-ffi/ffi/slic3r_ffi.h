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

/* ---- Slicing ---- */

/* Slice model with config and write G-code to out_gcode_path.
 *
 * Pipeline:
 *   Print::apply(model, config) → Print::validate() → Print::process()
 *     → Print::export_gcode(out_gcode_path).
 *
 * For single-object STLs this slices at the origin. For 3MFs, the objects'
 * embedded transforms are honored.
 *
 * out_err may be NULL. If non-NULL and the call fails, *out_err receives a
 * heap-allocated message; caller frees with slic3r_string_free.
 *
 * SLIC3R_ERR_VALIDATE: cross-option / object validation failed.
 * SLIC3R_ERR_SLICE:    exception thrown during process() or export_gcode(). */
slic3r_status slic3r_slice(slic3r_model_t* model,
                            slic3r_config_t* config,
                            const char* out_gcode_path,
                            char** out_err);

#ifdef __cplusplus
}
#endif

#endif /* SLIC3R_FFI_H */
