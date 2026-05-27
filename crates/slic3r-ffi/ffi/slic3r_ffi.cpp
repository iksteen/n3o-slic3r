// slic3r_ffi.cpp — implementation of the flat C API over libslic3r.
// See slic3r_ffi.h for the contract.

#include "slic3r_ffi.h"

#include <libslic3r/Config.hpp>
#include <libslic3r/PrintConfig.hpp>
#include <libslic3r/Model.hpp>
#include <libslic3r/Print.hpp>
#include <libslic3r/PrintBase.hpp>
#include <libslic3r/Utils.hpp>
#include <libslic3r/GCode/GCodeProcessor.hpp>

#include <boost/log/core.hpp>
#include <boost/log/trivial.hpp>
#include <boost/log/expressions.hpp>
#include <boost/log/sinks/sync_frontend.hpp>
#include <boost/log/sinks/basic_sink_backend.hpp>
#include <boost/log/attributes/value_extraction.hpp>
#include <boost/make_shared.hpp>
#include <boost/shared_ptr.hpp>

#include <cstdlib>
#include <cstring>
#include <exception>
#include <filesystem>
#include <map>
#include <memory>
#include <mutex>
#include <string>
#include <unordered_map>
#include <unordered_set>
#include <vector>

using namespace Slic3r;

namespace {

char* dup_c(const std::string& s) {
    char* p = static_cast<char*>(std::malloc(s.size() + 1));
    if (!p) return nullptr;
    std::memcpy(p, s.data(), s.size());
    p[s.size()] = '\0';
    return p;
}

void set_err(char** out_err, const std::string& msg) {
    if (out_err) *out_err = dup_c(msg);
}

// ---- Slice progress callback ----------------------------------------
//
// The progress callback is passed *into* `slic3r_slice` as a
// (progress_cb, progress_user_data) pair. The slice captures them
// directly into the libslic3r `set_status_callback` lambda for that
// Print instance — no global state, no cross-slice contamination.
// Earlier shape stored a process-global callback registered out-of-
// band; that race-routed Print A's progress events to whoever last
// registered, which made concurrent test runs flaky and ruled out
// any future concurrent slicing.

// ---- Log sink ------------------------------------------------------
//
// `slic3r_set_log_sink` registers a C function pointer that the
// boost::log sink trampolines into. The sink is installed once
// during slic3r_init and remains for the process lifetime; when no
// callback is registered the sink no-ops, so we don't churn the
// boost::log core's sink list on every register/clear.
//
// Severity mapping mirrors libslic3r's `set_logging_level`:
//   trace=0, debug=1, info=2, warning=3, error=4, fatal=5.
// Boost's `severity_level` enum uses the same values so we cast
// directly.
//
// Library hardening: the callback may throw — boost::log won't
// — so the consume() impl swallows std::exception to keep
// libslic3r's logging path crash-safe. A throwing callback is a
// caller bug we don't propagate into the slicer.

std::mutex g_log_mutex;
slic3r_log_fn_t g_log_cb = nullptr;
void* g_log_user_data = nullptr;

class CallbackLogBackend
    : public boost::log::sinks::basic_sink_backend<
          boost::log::sinks::combine_requirements<
              boost::log::sinks::synchronized_feeding>::type> {
public:
    void consume(boost::log::record_view const& rec) {
        slic3r_log_fn_t cb;
        void* user;
        {
            std::lock_guard<std::mutex> lk(g_log_mutex);
            cb = g_log_cb;
            user = g_log_user_data;
        }
        if (!cb) return;
        const auto* sev = boost::log::extract<boost::log::trivial::severity_level>(
                              "Severity", rec).get_ptr();
        const auto* msg = boost::log::extract<std::string>(
                              "Message", rec).get_ptr();
        int severity = sev ? static_cast<int>(*sev) : 2;
        const std::string& text = msg ? *msg : std::string();
        try {
            cb(severity, text.c_str(), user);
        } catch (const std::exception&) {
            // Swallow — see class doc above.
        } catch (...) {
            // Same.
        }
    }
};

using CallbackLogSink = boost::log::sinks::synchronous_sink<CallbackLogBackend>;
boost::shared_ptr<CallbackLogSink> g_log_sink_ptr;

slic3r_opt_type map_type(ConfigOptionType t) {
    // ConfigOptionType is a bit-packed enum where coVectorType = 0x4000.
    // Our public enum mirrors that layout exactly, so a cast is safe for the
    // values we expose. Unknown values fall back to NONE.
    switch (t) {
        case coNone:                return SLIC3R_OPT_NONE;
        case coFloat:               return SLIC3R_OPT_FLOAT;
        case coFloats:              return SLIC3R_OPT_FLOATS;
        case coInt:                 return SLIC3R_OPT_INT;
        case coInts:                return SLIC3R_OPT_INTS;
        case coString:              return SLIC3R_OPT_STRING;
        case coStrings:             return SLIC3R_OPT_STRINGS;
        case coPercent:             return SLIC3R_OPT_PERCENT;
        case coPercents:            return SLIC3R_OPT_PERCENTS;
        case coFloatOrPercent:      return SLIC3R_OPT_FLOAT_OR_PERCENT;
        case coFloatsOrPercents:    return SLIC3R_OPT_FLOATS_OR_PERCENTS;
        case coPoint:               return SLIC3R_OPT_POINT;
        case coPoints:              return SLIC3R_OPT_POINTS;
        case coPoint3:              return SLIC3R_OPT_POINT3;
        case coBool:                return SLIC3R_OPT_BOOL;
        case coBools:               return SLIC3R_OPT_BOOLS;
        case coEnum:                return SLIC3R_OPT_ENUM;
        case coEnums:               return SLIC3R_OPT_ENUMS;
        default:                    return SLIC3R_OPT_NONE;
    }
}

slic3r_opt_mode map_mode(ConfigOptionMode m) {
    switch (m) {
        case comSimple:   return SLIC3R_MODE_SIMPLE;
        case comAdvanced: return SLIC3R_MODE_ADVANCED;
        case comDevelop:  return SLIC3R_MODE_DEVELOP;
        default:          return SLIC3R_MODE_EXPERT;
    }
}

// Serialize the default value of a coEnums (vector-of-enums) option.
//
// libslic3r's ConfigOptionEnumsGenericTempl::serialize() segfaults on a
// null keys_map. The standard set_default_value() path clones the default
// option without propagating the def's enum_keys_map pointer, so every
// coEnums default lands with a null map and can't be serialized through
// the normal route. Mirror what the option's serializer would do, but
// pull the reverse-lookup map from the def instead of the option itself.
std::string serialize_coenums_default(const ConfigOptionDef& d) {
    if (!d.default_value || !d.enum_keys_map) return {};
    const auto* opt = dynamic_cast<const ConfigOptionVector<int>*>(d.default_value.get());
    if (!opt) return {};

    std::string out;
    bool first = true;
    for (int v : opt->values) {
        if (!first) out += ',';
        first = false;
        bool found = false;
        for (const auto& kvp : *d.enum_keys_map) {
            if (kvp.second == v) {
                out += kvp.first;
                found = true;
                break;
            }
        }
        if (!found) out += '?';  // unknown index — make it visible
    }
    return out;
}

// Process-lifetime snapshot of print_config_def. Holds owned strings so the
// pointers we hand out remain valid even if libslic3r's storage moves.
struct DefCache {
    struct Entry {
        std::string key;
        std::string label;
        std::string full_label;
        std::string tooltip;
        std::string category;
        std::string sidetext;
        std::string default_serialized;
        std::vector<std::string> enum_values;
        std::vector<std::string> enum_labels;
        std::vector<const char*> enum_value_ptrs;
        std::vector<const char*> enum_label_ptrs;
        slic3r_opt_type type;
        slic3r_opt_mode mode;
        int readonly;
        int multiline;
        double min;
        double max;
        unsigned int scope;
    };

    std::vector<std::unique_ptr<Entry>> entries; // unique_ptr so c_str() pointers don't move on grow
    std::unordered_map<std::string, size_t> by_key;

    void build() {
        // Scope is encoded structurally in libslic3r — by which static
        // config class declares each option. Pre-collect each class's key
        // set (via the cache populated by print_config_static_initializer)
        // so we can mask every option's scope in one pass.
        auto to_set = [](const t_config_option_keys& v) {
            return std::unordered_set<std::string>(v.begin(), v.end());
        };
        const auto keys_object       = to_set(PrintObjectConfig().keys());
        const auto keys_region       = to_set(PrintRegionConfig().keys());
        const auto keys_print        = to_set(PrintConfig().keys());
        const auto keys_sla_object   = to_set(SLAPrintObjectConfig().keys());
        const auto keys_sla_print    = to_set(SLAPrintConfig().keys());
        const auto keys_sla_material = to_set(SLAMaterialConfig().keys());
        const auto keys_sla_printer  = to_set(SLAPrinterConfig().keys());

        entries.reserve(print_config_def.options.size());
        for (const auto& kv : print_config_def.options) {
            auto e = std::make_unique<Entry>();
            const ConfigOptionDef& d = kv.second;
            e->key                = kv.first;
            e->label              = d.label;
            e->full_label         = d.full_label;
            e->tooltip            = d.tooltip;
            e->category           = d.category;
            e->sidetext           = d.sidetext;
            // coEnums defaults can't go through the option's own
            // serialize() — its keys_map member is null on the cloned
            // default. serialize_coenums_default() does the reverse-lookup
            // using the def's enum_keys_map instead.
            if (d.default_value) {
                try {
                    e->default_serialized = (d.type == coEnums)
                        ? serialize_coenums_default(d)
                        : d.default_value->serialize();
                } catch (...) {
                    e->default_serialized.clear();
                }
            }
            e->enum_values        = d.enum_values;
            e->enum_labels        = d.enum_labels;
            e->type               = map_type(d.type);
            e->mode               = map_mode(d.mode);
            e->readonly           = d.readonly  ? 1 : 0;
            e->multiline          = d.multiline ? 1 : 0;
            e->min                = d.min;
            e->max                = d.max;
            for (const auto& s : e->enum_values) e->enum_value_ptrs.push_back(s.c_str());
            for (const auto& s : e->enum_labels) e->enum_label_ptrs.push_back(s.c_str());

            e->scope = 0;
            if (keys_print.count(kv.first))        e->scope |= SLIC3R_SCOPE_PRINT;
            if (keys_object.count(kv.first))       e->scope |= SLIC3R_SCOPE_OBJECT;
            if (keys_region.count(kv.first))       e->scope |= SLIC3R_SCOPE_REGION;
            if (keys_sla_print.count(kv.first))    e->scope |= SLIC3R_SCOPE_SLA_PRINT;
            if (keys_sla_object.count(kv.first))   e->scope |= SLIC3R_SCOPE_SLA_OBJECT;
            if (keys_sla_material.count(kv.first)) e->scope |= SLIC3R_SCOPE_SLA_MATERIAL;
            if (keys_sla_printer.count(kv.first))  e->scope |= SLIC3R_SCOPE_SLA_PRINTER;

            by_key.emplace(e->key, entries.size());
            entries.push_back(std::move(e));
        }
    }

    void fill(size_t i, slic3r_option_def_t* out) const {
        const Entry& e = *entries[i];
        out->key                = e.key.c_str();
        out->type               = e.type;
        out->label              = e.label.empty()              ? nullptr : e.label.c_str();
        out->full_label         = e.full_label.empty()         ? nullptr : e.full_label.c_str();
        out->tooltip            = e.tooltip.empty()            ? nullptr : e.tooltip.c_str();
        out->category           = e.category.empty()           ? nullptr : e.category.c_str();
        out->sidetext           = e.sidetext.empty()           ? nullptr : e.sidetext.c_str();
        out->default_serialized = e.default_serialized.empty() ? nullptr : e.default_serialized.c_str();
        out->mode               = e.mode;
        out->readonly           = e.readonly;
        out->multiline          = e.multiline;
        out->enum_values        = e.enum_value_ptrs.empty() ? nullptr : e.enum_value_ptrs.data();
        out->enum_labels        = e.enum_label_ptrs.empty() ? nullptr : e.enum_label_ptrs.data();
        out->enum_value_count   = e.enum_value_ptrs.size();
        out->min                = e.min;
        out->max                = e.max;
        out->scope              = e.scope;
    }
};

std::mutex          g_init_mutex;
bool                g_initialized = false;
std::unique_ptr<DefCache> g_def_cache;

} // namespace

// Opaque handle wrappers. Defined in the global namespace so they match the
// forward declarations in slic3r_ffi.h.
struct slic3r_config_t {
    DynamicPrintConfig cfg;
};

struct slic3r_model_t {
    Model model;
};

extern "C" {

const char* slic3r_version(void) {
    return "OrcaSlicer libslic3r_ffi v0";
}

slic3r_status slic3r_init(const char* resources_dir, unsigned int log_level) {
    std::lock_guard<std::mutex> lk(g_init_mutex);
    if (g_initialized) return SLIC3R_OK;
    try {
        set_logging_level(log_level);

        // Install the callback sink once at init. It stays in the
        // boost::log core's sink list for the process lifetime; the
        // sink itself no-ops when no callback is registered, so
        // running without `slic3r_set_log_sink` costs nothing
        // beyond the sink-list traversal per log record.
        if (!g_log_sink_ptr) {
            g_log_sink_ptr = boost::make_shared<CallbackLogSink>();
            boost::log::core::get()->add_sink(g_log_sink_ptr);
        }

        if (resources_dir && *resources_dir) {
            Slic3r::set_resources_dir(resources_dir);
            // OrcaSlicer puts shaders/icons under resources/; var_dir is the
            // images/ subtree by historical convention.
            Slic3r::set_var_dir(std::string(resources_dir) + "/images");
        }

        // libslic3r writes a working-copy backup of every loaded 3MF into
        // temporary_dir(); the unset default resolves to "/orcaslicer_model"
        // (filesystem root), which is not writable for non-root users and
        // causes 3MF loads to silently produce an empty Model. Upstream's
        // CLI sets this via wxFileName::GetTempDir(); we use the C++17
        // equivalent so the shim has no wx dependency.
        Slic3r::set_temporary_dir(std::filesystem::temp_directory_path().string());

        g_def_cache = std::make_unique<DefCache>();
        g_def_cache->build();
        g_initialized = true;
        return SLIC3R_OK;
    } catch (const std::exception&) {
        return SLIC3R_ERR_INTERNAL;
    }
}

void slic3r_string_free(char* s) {
    if (s) std::free(s);
}

// ---- Option introspection ----

size_t slic3r_option_def_count(void) {
    if (!g_def_cache) return 0;
    return g_def_cache->entries.size();
}

slic3r_status slic3r_option_def_at(size_t i, slic3r_option_def_t* out) {
    if (!out) return SLIC3R_ERR_INVALID_ARG;
    if (!g_def_cache) return SLIC3R_ERR_NOT_INIT;
    if (i >= g_def_cache->entries.size()) return SLIC3R_ERR_INVALID_ARG;
    g_def_cache->fill(i, out);
    return SLIC3R_OK;
}

slic3r_status slic3r_option_def_lookup(const char* key, slic3r_option_def_t* out) {
    if (!key || !out) return SLIC3R_ERR_INVALID_ARG;
    if (!g_def_cache) return SLIC3R_ERR_NOT_INIT;
    auto it = g_def_cache->by_key.find(key);
    if (it == g_def_cache->by_key.end()) return SLIC3R_ERR_UNKNOWN_KEY;
    g_def_cache->fill(it->second, out);
    return SLIC3R_OK;
}

// ---- Config ----

slic3r_config_t* slic3r_config_new(void) {
    if (!g_initialized) return nullptr;
    try {
        auto* c = new slic3r_config_t();
        // Seed with FullPrintConfig defaults so every option has a value.
        // FullPrintConfig is macro-generated and default-constructible with
        // each ConfigOptionDef's default applied.
        FullPrintConfig defaults;
        c->cfg.apply(defaults, /*ignore_nonexistent=*/true);
        return c;
    } catch (const std::exception&) {
        return nullptr;
    }
}

void slic3r_config_free(slic3r_config_t* cfg) {
    delete cfg;
}

slic3r_status slic3r_config_set(slic3r_config_t* cfg, const char* key, const char* value) {
    if (!cfg || !key || !value) return SLIC3R_ERR_INVALID_ARG;
    if (!g_def_cache)            return SLIC3R_ERR_NOT_INIT;
    if (!print_config_def.has(key)) return SLIC3R_ERR_UNKNOWN_KEY;
    try {
        ConfigSubstitutionContext ctx(ForwardCompatibilitySubstitutionRule::Disable);
        cfg->cfg.set_deserialize(key, value, ctx);
        return SLIC3R_OK;
    } catch (const std::exception&) {
        return SLIC3R_ERR_PARSE_VALUE;
    }
}

slic3r_status slic3r_config_get(slic3r_config_t* cfg, const char* key, char** out_value) {
    if (!cfg || !key || !out_value) return SLIC3R_ERR_INVALID_ARG;
    *out_value = nullptr;
    if (!cfg->cfg.has(key)) return SLIC3R_ERR_UNKNOWN_KEY;
    try {
        *out_value = dup_c(cfg->cfg.opt_serialize(key));
        return *out_value ? SLIC3R_OK : SLIC3R_ERR_INTERNAL;
    } catch (const std::exception&) {
        return SLIC3R_ERR_INTERNAL;
    }
}

slic3r_status slic3r_config_validate(slic3r_config_t* cfg, char** out_err) {
    if (!cfg) return SLIC3R_ERR_INVALID_ARG;
    if (out_err) *out_err = nullptr;
    try {
        auto errors = cfg->cfg.validate(); // map<key, message>
        if (errors.empty()) return SLIC3R_OK;
        // Join all errors so the caller sees the full picture, not just the first.
        std::string joined;
        for (const auto& kv : errors) {
            if (!joined.empty()) joined += "; ";
            joined += kv.first;
            joined += ": ";
            joined += kv.second;
        }
        set_err(out_err, joined);
        return SLIC3R_ERR_VALIDATE;
    } catch (const std::exception& e) {
        set_err(out_err, e.what());
        return SLIC3R_ERR_VALIDATE;
    }
}

// ---- Model ----

slic3r_model_t* slic3r_model_new(void) {
    try {
        return new slic3r_model_t();
    } catch (const std::exception&) {
        return nullptr;
    }
}

void slic3r_model_free(slic3r_model_t* model) {
    delete model;
}

namespace {

// Shared body of slic3r_model_load and slic3r_model_load_with_config.
// `cfg` may be null when the caller doesn't want the embedded config.
slic3r_status do_load(slic3r_model_t* m,
                      DynamicPrintConfig* cfg,
                      const char* path,
                      char** out_err) {
    if (!m || !path) return SLIC3R_ERR_INVALID_ARG;
    if (out_err) *out_err = nullptr;
    try {
        // LoadModel is required for the BBS 3MF importer to actually attach
        // parsed objects to the model (otherwise it deletes them — see
        // bbs_3mf.cpp:_handle_end_object). LoadConfig pulls plate / object
        // config out of the 3MF's Metadata/ tree when present. STL/OBJ/STEP
        // loaders ignore the flags but accept them harmlessly, so we always
        // pass this set rather than branching on extension.
        const auto opts = LoadStrategy::LoadModel
                        | LoadStrategy::LoadConfig
                        | LoadStrategy::AddDefaultInstances;
        // Silent forward-compat: older 3MFs with renamed keys are accepted.
        // Substitution warnings are dropped on the floor for v0; a future
        // API could surface them.
        ConfigSubstitutionContext ctx(ForwardCompatibilitySubstitutionRule::EnableSilent);
        m->model = Model::read_from_file(path, cfg, &ctx, opts);
        return SLIC3R_OK;
    } catch (const std::exception& e) {
        set_err(out_err, e.what());
        return SLIC3R_ERR_IO;
    }
}

} // namespace

slic3r_status slic3r_model_load(slic3r_model_t* m, const char* path, char** out_err) {
    return do_load(m, nullptr, path, out_err);
}

slic3r_status slic3r_model_load_with_config(slic3r_model_t* m,
                                             slic3r_config_t* c,
                                             const char* path,
                                             char** out_err) {
    if (!c) return SLIC3R_ERR_INVALID_ARG;
    return do_load(m, &c->cfg, path, out_err);
}

// ---- Slicing ----

slic3r_status slic3r_slice(slic3r_model_t* model,
                            slic3r_config_t* config,
                            const char* out_path,
                            slic3r_progress_fn_t progress_cb,
                            void* progress_user_data,
                            char** out_err) {
    if (!model || !config || !out_path) return SLIC3R_ERR_INVALID_ARG;
    if (out_err) *out_err = nullptr;
    try {
        // Several config fields must be normalized to the printer's geometry
        // before slicing — chiefly that filament_map has one entry per
        // filament, and that nozzle_volume_type has one entry per extruder.
        // Upstream's CLI does this between loading and apply()
        // (OrcaSlicer.cpp:5953-5964). Without it, ToolOrdering sees an
        // undersized filament_map, produces degenerate per-layer extruder
        // assignments (sentinel (unsigned)-1 entries), and process() crashes
        // in check_filament_printable_after_group / calc_filament_change_
        // info_by_toolorder when it dereferences those sentinels.
        //
        // Apply the same normalization to a temporary copy so we don't
        // mutate the caller's config.
        DynamicPrintConfig cfg = config->cfg;
        const size_t extruder_count = cfg.has("nozzle_diameter")
            ? cfg.option<ConfigOptionFloats>("nozzle_diameter")->values.size()
            : 1;
        const size_t filament_count = cfg.has("filament_diameter")
            ? cfg.option<ConfigOptionFloats>("filament_diameter")->values.size()
            : 1;

        auto& filament_map = cfg.option<ConfigOptionInts>("filament_map", true)->values;
        if (filament_map.size() < filament_count)
            filament_map.resize(filament_count, 1);
        if (extruder_count == 1) {
            // Force all filaments onto the single extruder. Matches
            // OrcaSlicer.cpp:5957-5960.
            for (size_t i = 0; i < filament_count; ++i)
                filament_map[i] = 1;
        }

        if (!cfg.has("nozzle_volume_type"))
            cfg.option<ConfigOptionEnumsGeneric>("nozzle_volume_type", true)
                ->values.resize(extruder_count, nvtStandard);

        // Per-region / per-object filament selectors carry "0 = use the
        // object/printer default" in 3MF configs. OrcaSlicer's GUI resolves
        // these via PartPlate state pre-apply, substituting the part's
        // default extruder. The headless slicing entry doesn't have
        // PartPlate state, so the 0 sentinels leak straight into
        // Print::process. Once there, `Print::process` invokes ToolOrdering
        // with first_extruder == -1; `handle_dontcare_extruder(-1)` is
        // supposed to promote zeros to a real extruder by scanning the
        // layer-tools list, but if everything is 0 it finds nothing to
        // promote, the sentinel persists into
        // `reorder_extruders_for_minimum_flush_volume` →
        // `check_filament_printable_after_group`, and the unchecked
        // `filament_maps[filament_id]` access in there SIGSEGVs. We can't
        // fix the `-1` at the call site without patching libslic3r
        // (`Print.cpp:2378-2379` hardcodes it), so we resolve the 0s
        // upstream — same role OrcaSlicer's GUI plays.
        //
        // Per-object resolution. `support_filament` and
        // `support_interface_filament` live in PrintObjectConfig. The BBS
        // 3MF importer lifts each object's `<metadata key="extruder">`
        // (object-level default extruder) into
        // `ModelObject::config["extruder"]`, so we re-use that hint per
        // object — supports inherit the part's body extruder by default,
        // matching what OrcaSlicer's GUI emits.
        //
        // Per-region resolution. `wall_filament`, `sparse_infill_filament`,
        // and `solid_infill_filament` live in PrintRegionConfig. Per-volume
        // `extruder` overrides on typed regions (4-color, 5T, etc.) win
        // during Print::apply's region merge — so the print-level cfg
        // value only matters for "untyped" regions with no per-volume
        // override (catch-all like skirt/brim/supports). For these we
        // fall back to filament 1 if every object's default extruder
        // disagrees; if all objects share one default, we use that.
        //
        // PR-3-11 history: commit 1bcf46d removed an earlier coerce-to-1
        // block on the rationale that it was "vestigial after filament_map
        // normalization." That was wrong — the api tests + spike1/spike2
        // don't exercise multi-color ToolOrdering and the removal regressed
        // the fourcolor slice into the SIGSEGV above. `git bisect` pinned
        // the regressor. This per-object resolution is the proper fix
        // (better than the original hardcoded 1, which would have been
        // wrong for any model whose default extruder isn't 1).
        // Per-REGION filament selectors (`wall_filament`,
        // `sparse_infill_filament`, `solid_infill_filament`) must not
        // leak 0 sentinels into ToolOrdering — `handle_dontcare_
        // extruder(-1)` (called inside `Print::process` because
        // `Print.cpp:2378` hardcodes -1) needs at least one non-zero
        // extruder somewhere in the layer-tools list to promote from,
        // and if the print-wide defaults are 0 every region with no
        // per-volume override goes 0 too. Resolve them per-object
        // using each object's `<metadata key="extruder">` hint
        // (lifted into `ModelObject::config["extruder"]` by the BBS
        // 3MF importer), then fall back to filament 1 at the print
        // level if every object's default is identical else
        // disagrees. Per-volume `extruder` overrides on typed regions
        // still dominate this during `Print::apply`'s region merge,
        // so this only affects the catch-all "untyped" region.
        //
        // Per-OBJECT support filament selectors (`support_filament`,
        // `support_interface_filament`) MUST stay at 0 (dontcare).
        // libslic3r's per-layer support-extruder resolution at
        // `GCode.cpp:4794-4820` picks `first_extruder_id =
        // layer_tools.extruders.front()` for the layer in question —
        // exactly what we want for the 4-color stacked case where
        // each layer has only one band active and supports should
        // inherit that band's body extruder. Coercing
        // support_filament to any non-zero value suppresses this
        // routing and pins all supports to one extruder, causing 76
        // mid-print tool changes on fourcolor.3mf vs Orca/BBS's 7.
        // Confirmed empirically: with support_filament left at
        // dontcare, spike3 produces 7 changes / 1h 6m / 14g
        // matching the BBS reference.
        //
        // History: commit 1bcf46d removed an even earlier coerce on
        // all five selectors thinking it was vestigial, regressing
        // the slice into SIGSEGV (bisected back to this commit
        // during PR-3-11). The intermediate "restore as
        // hardcoded 1" fix matched the segfault repair but kept the
        // 76-vs-7 disparity. The split below is the proper shape:
        // resolve the per-region zeros, leave the per-object support
        // zeros alone.
        {
            int common_default = -1;  // -1 = unset, -2 = disagree
            for (auto* obj : model->model.objects) {
                if (!obj) continue;
                int obj_default = 1;
                if (auto* opt = dynamic_cast<const ConfigOptionInt*>(
                        obj->config.option("extruder"))) {
                    if (opt->value > 0) obj_default = opt->value;
                }
                for (const char* key : {"wall_filament",
                                         "sparse_infill_filament",
                                         "solid_infill_filament"}) {
                    const auto* opt = dynamic_cast<const ConfigOptionInt*>(
                        obj->config.option(key));
                    int current = opt ? opt->value : 0;
                    if (current == 0) {
                        obj->config.set_key_value(
                            key, new ConfigOptionInt(obj_default));
                    }
                }
                if (common_default == -1) common_default = obj_default;
                else if (common_default != obj_default) common_default = -2;
            }
            int print_fallback = (common_default > 0) ? common_default : 1;
            for (const char* key : {"wall_filament",
                                     "sparse_infill_filament",
                                     "solid_infill_filament"}) {
                if (auto* opt = cfg.option<ConfigOptionInt>(key); opt && opt->value == 0)
                    opt->value = print_fallback;
            }
        }

        Print print;
        print.apply(model->model, cfg);

        // Install the caller-supplied progress callback on this
        // Print instance. The lambda captures progress_cb +
        // progress_user_data by value so the callback travels with
        // the slice — concurrent slices on distinct Prints each
        // carry their own. NULL cb → silent slice (suppresses
        // libslic3r's stderr default).
        if (progress_cb) {
            print.set_status_callback(
                [progress_cb, progress_user_data](const PrintBase::SlicingStatus& s) {
                    progress_cb(s.percent, s.text.c_str(), progress_user_data);
                });
        } else {
            print.set_status_silent();
        }

        // Print::is_BBL_printer() is a manually-set flag (Print.hpp:1143,
        // declared without an initializer). The GUI sets it from the active
        // preset bundle; the CLI checks the printer_model prefix. Without
        // it, validators that know Bambu printers don't follow Marlin's
        // relative-E + per-layer-G92 convention take the wrong branch.
        const std::string printer_model = cfg.opt_string("printer_model");
        print.is_BBL_printer() = (printer_model.compare(0, 9, "Bambu Lab") == 0);

        StringObjectException err = print.validate();
        if (!err.string.empty()) {
            set_err(out_err, err.string);
            return SLIC3R_ERR_VALIDATE;
        }

        print.process();

        // GCodeProcessorResult holds time/filament analysis. We must pass a
        // valid pointer (export_gcode populates it); we just drop it on return
        // since v0 doesn't surface preview data yet.
        GCodeProcessorResult gcode_result;
        print.export_gcode(out_path, &gcode_result, nullptr);
        return SLIC3R_OK;
    } catch (const std::exception& e) {
        set_err(out_err, e.what());
        return SLIC3R_ERR_SLICE;
    }
}

void slic3r_set_log_sink(slic3r_log_fn_t cb, void* user_data) {
    std::lock_guard<std::mutex> lk(g_log_mutex);
    g_log_cb = cb;
    g_log_user_data = user_data;
}

} // extern "C"
