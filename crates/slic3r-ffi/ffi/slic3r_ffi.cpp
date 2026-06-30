// slic3r_ffi.cpp — implementation of the flat C API over libslic3r.
// See slic3r_ffi.h for the contract.

#include "slic3r_ffi.h"

#include <libslic3r/Config.hpp>
#include <libslic3r/PrintConfig.hpp>
#include <libslic3r/Preset.hpp>
#include <libslic3r/Model.hpp>
#include <libslic3r/Print.hpp>
#include <libslic3r/PrintBase.hpp>
#include <libslic3r/Exception.hpp>
#include <libslic3r/TriangleMesh.hpp>
#include <libslic3r/TriangleMeshSlicer.hpp>
#include <libslic3r/TriangleSelector.hpp>
#include <libslic3r/MeshBoolean.hpp>
#include <libslic3r/Geometry.hpp>
#include <libslic3r/Orient.hpp>
#include <libslic3r/Arrange.hpp>
#include <libslic3r/Utils.hpp>
#include <libslic3r/GCode/GCodeProcessor.hpp>
#include <libslic3r/Format/bbs_3mf.hpp>

#include <boost/log/core.hpp>
#include <boost/log/trivial.hpp>
#include <boost/log/expressions.hpp>
#include <boost/log/sinks/sync_frontend.hpp>
#include <boost/log/sinks/basic_sink_backend.hpp>
#include <boost/log/attributes/value_extraction.hpp>
#include <boost/make_shared.hpp>
#include <boost/shared_ptr.hpp>

#include <algorithm>
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
        int is_color;
        double min;
        double max;
        unsigned int scope;
        unsigned int bucket;
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

        // Bucket is the preset tab that owns a key, from the same Preset
        // option vectors the (now-removed) Python scraper read:
        //   Process  = Preset::print_options()
        //   Filament = Preset::filament_options()
        //   Printer  = Preset::printer_options() (already unions
        //              s_Preset_machine_limits_options + nozzle/extruder keys)
        // A key in exactly one bucket gets it; a key in zero or more than one
        // (per-preset metadata like compatible_printers/inherits) is NONE.
        const auto bkt_process  = to_set(Preset::print_options());
        const auto bkt_filament = to_set(Preset::filament_options());
        const auto bkt_printer  = to_set(Preset::printer_options());

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
            e->is_color           = (d.gui_type == ConfigOptionDef::GUIType::color) ? 1 : 0;
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

            int bucket_hits = 0;
            e->bucket = SLIC3R_BUCKET_NONE;
            if (bkt_process.count(kv.first))  { bucket_hits++; e->bucket = SLIC3R_BUCKET_PROCESS; }
            if (bkt_filament.count(kv.first)) { bucket_hits++; e->bucket = SLIC3R_BUCKET_FILAMENT; }
            if (bkt_printer.count(kv.first))  { bucket_hits++; e->bucket = SLIC3R_BUCKET_PRINTER; }
            if (bucket_hits != 1) e->bucket = SLIC3R_BUCKET_NONE;

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
        out->is_color           = e.is_color;
        out->enum_values        = e.enum_value_ptrs.empty() ? nullptr : e.enum_value_ptrs.data();
        out->enum_labels        = e.enum_label_ptrs.empty() ? nullptr : e.enum_label_ptrs.data();
        out->enum_value_count   = e.enum_value_ptrs.size();
        out->min                = e.min;
        out->max                = e.max;
        out->scope              = e.scope;
        out->bucket             = e.bucket;
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

namespace {

// Walk one triangle's MMU paint bitstream (mirrors TriangleSelector::serialize):
// copy the split-tree structure verbatim and remap each leaf's filament state
// through `perm` (state s -> perm[s], identity when s is out of range). `c`
// advances past exactly one triangle's bits. Operates on the already-decoded
// in-memory bitstream — no hex packing involved (that's only 3MF I/O).
void remap_paint_walk(const std::vector<bool>& in, size_t& c,
                      std::vector<bool>& out, const std::vector<int>& perm,
                      std::vector<bool>& used_states) {
    bool s0 = in[c], s1 = in[c + 1];
    c += 2;
    out.push_back(s0);
    out.push_back(s1);
    int split_sides = (s0 ? 1 : 0) | (s1 ? 2 : 0);
    if (split_sides != 0) {
        // special_side (2 bits) — structural, copied verbatim.
        out.push_back(in[c]);
        out.push_back(in[c + 1]);
        c += 2;
        for (int i = 0; i <= split_sides; ++i) // split_sides + 1 children
            remap_paint_walk(in, c, out, perm, used_states);
    } else {
        // Leaf: state is 2 bits, or "11" prefix + 4 bits for states >= 3.
        bool p0 = in[c], p1 = in[c + 1];
        c += 2;
        int n;
        if (p0 && p1) {
            n = 0;
            for (int i = 0; i < 4; ++i)
                if (in[c + i]) n |= (1 << i);
            c += 4;
            n += 3;
        } else {
            n = (p0 ? 1 : 0) | (p1 ? 2 : 0);
        }
        int nn = (n >= 0 && n < static_cast<int>(perm.size())) ? perm[n] : n;
        if (nn >= 0 && nn < static_cast<int>(used_states.size()))
            used_states[nn] = true;
        if (nn >= 3) {
            out.push_back(true);
            out.push_back(true);
            int m = nn - 3;
            for (int i = 0; i < 4; ++i)
                out.push_back((m & (1 << i)) != 0);
        } else {
            out.push_back((nn & 1) != 0);
            out.push_back((nn & 2) != 0);
        }
    }
}

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
static void normalize_filament_map(DynamicPrintConfig& cfg) {
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
}

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
static void resolve_region_filaments(Model& model, DynamicPrintConfig& cfg) {
    int common_default = -1;  // -1 = unset, -2 = disagree
    for (auto* obj : model.objects) {
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
                // Deliberate write-through to the caller's model in
                // place — unlike `cfg` (copied at the top of this fn).
                // Copying the whole Model per slice would be wasteful
                // and it's consumed once per slice; see slice_outcome's
                // Rust doc, which documents the mutation.
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

// Pin the Bambu-Lab-specific engine quirks. `is_bbl` is the printer_model
// "Bambu Lab" prefix check, computed once by the caller.
//
// BBL printers are forced onto the Type1 (old, rectangular) wipe
// tower (Print::wipe_tower_type() returns Type1 whenever
// is_BBL_printer()). That tower has no stabilization cone — that's a
// Type2/rib-tower feature — yet Print::first_layer_wipe_tower_corners()
// unconditionally sizes one as `tan(cone_angle/2) *
// m_wipe_tower_data.height`, and only the Type2 path ever assigns
// `height`. So with the engine-default cone_angle=30 and an unset
// (garbage) `height`, the skirt convex hull picks up an infinite
// corner and ClipperLib throws "Coordinate outside allowed range" in
// _make_skirt — heap-dependent, so it strikes intermittently. Pin the
// cone angle to 0 for BBL printers (cone radius collapses to 0
// regardless of the unset height); a cone is meaningless on a Type1
// tower anyway. Non-BBL printers (e.g. the Snapmaker U1) use the Type2
// tower, which sets `height` and wants a real cone, so leave their
// value alone. See docs/libslic3r-workarounds.md §7.
//
// Print::is_BBL_printer() is a manually-set flag (Print.hpp:1143,
// declared without an initializer). The GUI sets it from the active
// preset bundle; the CLI checks the printer_model prefix. Without
// it, validators that know Bambu printers don't follow Marlin's
// relative-E + per-layer-G92 convention take the wrong branch.
static void pin_bbl_quirks(Print& print, DynamicPrintConfig& cfg, bool is_bbl) {
    if (is_bbl)
        cfg.option<ConfigOptionFloat>("wipe_tower_cone_angle", true)->value = 0.;
    print.is_BBL_printer() = is_bbl;
}

// ---- Shared in-memory volume construction (add_object / add_group / add_volume) ----
// Outside extern "C" — these are C++ (one is a template) and only ever called
// from the construction shims below.

// Rebuild an indexed_triangle_set from the flat object-local buffers — the same
// ingest as slic3r_orient_mesh.
static indexed_triangle_set its_from_buffers(const float* verts, size_t vcount,
                                             const uint32_t* indices, size_t tcount) {
    indexed_triangle_set its;
    its.vertices.reserve(vcount);
    for (size_t i = 0; i < vcount; ++i)
        its.vertices.emplace_back(verts[i * 3 + 0], verts[i * 3 + 1],
                                  verts[i * 3 + 2]);
    its.indices.reserve(tcount);
    for (size_t i = 0; i < tcount; ++i)
        its.indices.emplace_back(static_cast<int32_t>(indices[i * 3 + 0]),
                                 static_cast<int32_t>(indices[i * 3 + 1]),
                                 static_cast<int32_t>(indices[i * 3 + 2]));
    return its;
}

// MMU color-painting: the hex strings are already in the BBS format
// FacetsAnnotation::set_triangle_from_string expects — pass through.
static void apply_paint(ModelVolume* vol, const char* const* paint_hex, size_t paint_count) {
    if (paint_count == 0) return;
    vol->mmu_segmentation_facets.reserve(paint_count);
    for (size_t i = 0; i < paint_count; ++i)
        if (paint_hex[i] && paint_hex[i][0] != '\0')
            vol->mmu_segmentation_facets.set_triangle_from_string(
                static_cast<int>(i), paint_hex[i]);
    vol->mmu_segmentation_facets.shrink_to_fit();
}

// Per-object/-volume config overrides. Mirrors the 3MF loader's metadata path
// (set_deserialize routes the string through the schema deserializer so it
// parses to the right option type). Unknown keys are skipped so one bad key
// doesn't fail the whole object; silent forward-compat substitution matches
// the loader.
static void apply_overrides(ModelConfigObject& config, const char* const* keys,
                            const char* const* vals, size_t count) {
    if (count == 0) return;
    ConfigSubstitutionContext ctx(ForwardCompatibilitySubstitutionRule::EnableSilent);
    for (size_t i = 0; i < count; ++i) {
        const char* key = keys[i];
        const char* val = vals[i];
        if (!key || !val || !print_config_def.has(key))
            continue;
        try {
            config.set_deserialize(key, val, ctx);
        } catch (const std::exception&) {
            // Skip a value the deserializer rejects rather than aborting.
        }
    }
}

// The Print currently running in slic3r_slice (serialized by SLICE_LOCK, so at
// most one). `slic3r_cancel` flips its cancel flag from another thread; process()
// then aborts at its next throw_if_canceled() checkpoint. The mutex makes the
// cancel and the slice's set/clear mutually exclusive, so cancel never touches a
// Print that's being torn down.
static std::mutex g_active_print_mtx;
static Print* g_active_print = nullptr;

// Registers `print` as the active slice for its scope; clears it on exit (incl.
// stack unwind), so a cancel after the slice ends is a no-op rather than a UAF.
struct ActivePrintGuard {
    explicit ActivePrintGuard(Print& p) {
        std::lock_guard<std::mutex> lk(g_active_print_mtx);
        g_active_print = &p;
    }
    ~ActivePrintGuard() {
        std::lock_guard<std::mutex> lk(g_active_print_mtx);
        g_active_print = nullptr;
    }
};

} // namespace

extern "C" {

const char* slic3r_version(void) {
    // N3O_ORCA_SHA is the pinned OrcaSlicer submodule short SHA, injected by
    // CMake (from build.rs) when building inside a git checkout. Stringify the
    // bare token into the version literal; absent it, report just the base.
#ifdef N3O_ORCA_SHA
#define N3O_STRINGIFY_(x) #x
#define N3O_STRINGIFY(x) N3O_STRINGIFY_(x)
    return "OrcaSlicer libslic3r_ffi v0 (" N3O_STRINGIFY(N3O_ORCA_SHA) ")";
#undef N3O_STRINGIFY
#undef N3O_STRINGIFY_
#else
    return "OrcaSlicer libslic3r_ffi v0";
#endif
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

        // Disable libslic3r's BBS auto-backup scheduler. It's a
        // headless-irrelevant feature (GUI Plater opts into it via
        // Model::set_need_backup; we never do). The singleton's
        // background thread is harmless when idle, but this is a
        // PR-7c-2 probe: if abort rate stays the same with interval=0
        // the backup manager isn't the heap-corruption source we're
        // hunting. Keep this call regardless of the probe outcome —
        // there's no reason for a headless slicer to run backup ticks.
        Slic3r::set_backup_interval(0);

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

// ---- MMU paint remap ----

slic3r_status slic3r_model_remap_paint_filaments(slic3r_model_t* m,
                                                 const int32_t* perm,
                                                 size_t perm_len) {
    if (m == nullptr || (perm == nullptr && perm_len != 0))
        return SLIC3R_ERR_INVALID_ARG;
    try {
        std::vector<int> p(perm, perm + perm_len);
        for (ModelObject* obj : m->model.objects) {
            for (ModelVolume* vol : obj->volumes) {
                if (vol->mmu_segmentation_facets.empty())
                    continue;
                const TriangleSelector::TriangleSplittingData& in =
                    vol->mmu_segmentation_facets.get_data();
                TriangleSelector::TriangleSplittingData out;
                std::fill(out.used_states.begin(), out.used_states.end(), false);
                out.bitstream.reserve(in.bitstream.size());
                out.triangles_to_split.reserve(in.triangles_to_split.size());
                for (const auto& mapping : in.triangles_to_split) {
                    out.triangles_to_split.emplace_back(
                        mapping.triangle_idx, static_cast<int>(out.bitstream.size()));
                    size_t c = static_cast<size_t>(mapping.bitstream_start_idx);
                    remap_paint_walk(in.bitstream, c, out.bitstream, p, out.used_states);
                }
                vol->mmu_segmentation_facets.set_data(std::move(out));
            }
        }
        return SLIC3R_OK;
    } catch (const std::exception&) {
        return SLIC3R_ERR_INTERNAL;
    }
}

// ---- In-memory model construction ----

slic3r_status slic3r_model_add_object(
    slic3r_model_t* m, const char* name,
    const float* verts, size_t vcount,
    const uint32_t* indices, size_t tcount,
    const double transform[16], int extruder,
    const char* const* paint_hex, size_t paint_count,
    const char* const* ovr_keys, const char* const* ovr_vals, size_t ovr_count,
    char** out_err) {
    if (out_err) *out_err = nullptr;
    if (!m || !verts || !indices || !transform || vcount == 0 || tcount == 0)
        return SLIC3R_ERR_INVALID_ARG;
    if (paint_count != 0 && !paint_hex)
        return SLIC3R_ERR_INVALID_ARG;
    if (ovr_count != 0 && (!ovr_keys || !ovr_vals))
        return SLIC3R_ERR_INVALID_ARG;
    try {
        TriangleMesh mesh(its_from_buffers(verts, vcount, indices, tcount));
        // Match the .3mf loader (bbs_3mf.cpp): a negative signed volume means
        // inverted winding — flip so the in-memory build slices identically.
        if (mesh.volume() < 0.0)
            mesh.flip_triangles();

        ModelObject* obj = m->model.add_object();
        obj->name = name ? name : "";
        ModelVolume* vol = obj->add_volume(std::move(mesh));

        // Per-object base extruder (1-based filament index).
        obj->config.set_key_value("extruder", new ConfigOptionInt(extruder));

        // Object->world transform: 16 column-major doubles map straight onto
        // Eigen's (column-major) Matrix4d, which is what Transform3d wraps. For
        // a solo object the world placement rides the instance (a single volume
        // stays centered) — the .3mf loader's non-component path does the same.
        Eigen::Map<const Eigen::Matrix4d> mat(transform);
        Transform3d t(mat);
        // ModelInstance only exposes the Geometry::Transformation overload
        // (unlike ModelVolume, which also takes a raw Transform3d), so wrap.
        ModelInstance* inst = obj->add_instance();
        inst->set_transformation(Geometry::Transformation(t));

        apply_paint(vol, paint_hex, paint_count);
        apply_overrides(obj->config, ovr_keys, ovr_vals, ovr_count);

        return SLIC3R_OK;
    } catch (const std::exception& e) {
        set_err(out_err, e.what());
        return SLIC3R_ERR_INTERNAL;
    } catch (...) {
        set_err(out_err, "unknown error in slic3r_model_add_object");
        return SLIC3R_ERR_INTERNAL;
    }
}

slic3r_status slic3r_model_add_group(
    slic3r_model_t* m, const char* name, size_t* out_index, char** out_err) {
    if (out_err) *out_err = nullptr;
    if (!m) return SLIC3R_ERR_INVALID_ARG;
    try {
        ModelObject* obj = m->model.add_object();
        obj->name = name ? name : "";
        // A multi-volume group is one ModelObject with an identity instance;
        // each volume carries its own world placement (added via
        // slic3r_model_add_volume). This mirrors the .3mf writer's
        // components-with-identity-build-item shape and the loader's
        // component path (bbs_3mf.cpp), so buffer-load and temp-.3mf agree.
        ModelInstance* inst = obj->add_instance();
        inst->set_transformation(Geometry::Transformation(Transform3d::Identity()));
        if (out_index) *out_index = m->model.objects.size() - 1;
        return SLIC3R_OK;
    } catch (const std::exception& e) {
        set_err(out_err, e.what());
        return SLIC3R_ERR_INTERNAL;
    } catch (...) {
        set_err(out_err, "unknown error in slic3r_model_add_group");
        return SLIC3R_ERR_INTERNAL;
    }
}

slic3r_status slic3r_model_add_volume(
    slic3r_model_t* m, size_t object_index, const char* name,
    const float* verts, size_t vcount,
    const uint32_t* indices, size_t tcount,
    const double transform[16], int extruder,
    const char* const* paint_hex, size_t paint_count,
    const char* const* ovr_keys, const char* const* ovr_vals, size_t ovr_count,
    char** out_err) {
    if (out_err) *out_err = nullptr;
    if (!m || !verts || !indices || !transform || vcount == 0 || tcount == 0)
        return SLIC3R_ERR_INVALID_ARG;
    if (paint_count != 0 && !paint_hex)
        return SLIC3R_ERR_INVALID_ARG;
    if (ovr_count != 0 && (!ovr_keys || !ovr_vals))
        return SLIC3R_ERR_INVALID_ARG;
    if (object_index >= m->model.objects.size())
        return SLIC3R_ERR_INVALID_ARG;
    try {
        TriangleMesh mesh(its_from_buffers(verts, vcount, indices, tcount));
        // Match the .3mf loader (bbs_3mf.cpp): a negative signed volume means
        // inverted winding — flip so the in-memory build slices identically.
        if (mesh.volume() < 0.0)
            mesh.flip_triangles();

        ModelObject* obj = m->model.objects[object_index];
        ModelVolume* vol = obj->add_volume(std::move(mesh));
        vol->name = name ? name : "";

        // Per-volume extruder (1-based filament index) — group members each
        // print with their own filament, so the hint lives on the volume.
        vol->config.set_key_value("extruder", new ConfigOptionInt(extruder));

        // Volume->world transform. `add_volume` (modify_to_center_geometry
        // defaults true) centers the mesh and bakes a compensating translation
        // into the volume transform; compose the world placement onto that,
        // exactly as the loader's component path does
        // (bbs_3mf.cpp: set_transformation(comp * volume->get_transformation())).
        Eigen::Map<const Eigen::Matrix4d> mat(transform);
        Transform3d world_mat(mat);
        Geometry::Transformation world(world_mat);
        vol->set_transformation(world * vol->get_transformation());

        apply_paint(vol, paint_hex, paint_count);
        apply_overrides(vol->config, ovr_keys, ovr_vals, ovr_count);

        return SLIC3R_OK;
    } catch (const std::exception& e) {
        set_err(out_err, e.what());
        return SLIC3R_ERR_INTERNAL;
    } catch (...) {
        set_err(out_err, "unknown error in slic3r_model_add_volume");
        return SLIC3R_ERR_INTERNAL;
    }
}

// ---- Slicing ----

slic3r_status slic3r_slice(slic3r_model_t* model,
                            slic3r_config_t* config,
                            const char* out_path,
                            slic3r_progress_fn_t progress_cb,
                            void* progress_user_data,
                            float** out_tower_vertices,
                            size_t* out_tower_vertex_count,
                            uint32_t** out_tower_indices,
                            size_t* out_tower_index_count,
                            char** out_err,
                            char** out_warning) {
    if (!model || !config || !out_path) return SLIC3R_ERR_INVALID_ARG;
    if (out_err) *out_err = nullptr;
    if (out_warning) *out_warning = nullptr;
    // Tower-mesh out-params are an all-or-nothing group; clear them up front
    // so an early return (or a single-material plate) leaves no tower.
    const bool want_tower = out_tower_vertices && out_tower_vertex_count &&
                            out_tower_indices && out_tower_index_count;
    if (want_tower) {
        *out_tower_vertices = nullptr;
        *out_tower_vertex_count = 0;
        *out_tower_indices = nullptr;
        *out_tower_index_count = 0;
    }
    try {
        // Normalize filament_map / nozzle_volume_type to the printer's
        // geometry on a temporary copy so we don't mutate the caller's config.
        DynamicPrintConfig cfg = config->cfg;
        normalize_filament_map(cfg);

        // printer_model "Bambu Lab" prefix — the single source of truth for
        // the BBL-specific engine quirks (wipe-tower cone + is_BBL flag).
        const bool is_bbl =
            cfg.opt_string("printer_model").compare(0, 9, "Bambu Lab") == 0;

        resolve_region_filaments(model->model, cfg);

        Print print;
        pin_bbl_quirks(print, cfg, is_bbl);
        print.apply(model->model, cfg);

        // Print::m_origin (the plate origin) is declared without an
        // initializer (Print.hpp) and is normally set by the GUI's
        // PartPlate; a headless slice never touches it, so it holds
        // uninitialized garbage. Print::validate()'s clearance check
        // translates the bed-exclusion polygon by scale_(plate_origin) —
        // when the garbage is large the polygon overflows ClipperLib's
        // coordinate range and validate() throws "Coordinate outside
        // allowed range". The failure is heap/binary-dependent (any
        // unrelated code change can flip it), so pin the origin to zero.
        print.set_plate_origin(Vec3d(0.0, 0.0, 0.0));

        // Install the caller-supplied progress callback on this
        // Print instance. The lambda captures progress_cb +
        // progress_user_data by value so the callback travels with
        // the slice; NULL cb → silent slice (suppresses libslic3r's
        // stderr default).
        //
        // Serialize the call into Rust with a per-slice mutex.
        // libslic3r's `Print::process` fans work out across many TBB
        // worker threads (parallel_for over PrintObjects → infill,
        // generate_support_material, etc.) and each one invokes
        // `PrintBase::set_status` independently. The Rust callback is
        // an `FnMut` whose `call_mut` requires exclusive access;
        // concurrent invocation is UB and was caught in PR-7c-2 ASan
        // as a heap-use-after-free in `ProgressThrottle::should_emit`
        // (two workers racing on `last_stage: String` — one assigned
        // a fresh `to_owned()` value, dropping the prior String's
        // buffer while the other was still reading it inside memcmp).
        //
        // The mutex lives on slic3r_slice's stack; the lambda's
        // lifetime is bounded by Print's destruction at end of scope
        // (Print stores the std::function), so by-ref capture is
        // safe.
        std::mutex progress_mtx;
        if (progress_cb) {
            print.set_status_callback(
                [progress_cb, progress_user_data, &progress_mtx](const PrintBase::SlicingStatus& s) {
                    std::lock_guard<std::mutex> lk(progress_mtx);
                    progress_cb(s.percent, s.text.c_str(), progress_user_data);
                });
        } else {
            print.set_status_silent();
        }

        // Print::validate() reports non-fatal validation *warnings* through
        // its `warning` out-param (a StringObjectException*), and ~20 of those
        // sites deref it unconditionally — e.g. Print.cpp:1890, which fires
        // when the filaments' shrinkage compensations don't all match (common
        // on a multi-material BBL plate). The param defaults to nullptr, so the
        // headless entry — unlike the GUI, which always passes a real pointer
        // to surface warnings to the user — null-derefs and hard-crashes the
        // process before process() even runs. Pass a sink to catch (and, since
        // we have no UI for them, discard) the warnings, exactly as the GUI
        // does. The *returned* StringObjectException is the fatal error; the
        // sink is the advisory warning. See docs/libslic3r-workarounds.md.
        StringObjectException validation_warning;
        StringObjectException err = print.validate(&validation_warning);
        if (!err.string.empty()) {
            set_err(out_err, err.string);
            return SLIC3R_ERR_VALIDATE;
        }
        // Surface the non-fatal validation warning (if any) to the caller so
        // it can show it in the UI — the same advisory the GUI displays.
        if (out_warning && !validation_warning.string.empty())
            set_err(out_warning, validation_warning.string);

        // Publish the Print for the duration of process() so slic3r_cancel can
        // abort it (process() throws CanceledException at its next checkpoint;
        // that's a std::exception, caught below and surfaced as SLIC3R_ERR_SLICE
        // — the caller distinguishes a user cancel by its own flag).
        {
            ActivePrintGuard active(print);
            print.process();
        }

        // Capture the prime/wipe tower's exact mesh, built by process() via
        // WipeTowerData::construct_mesh: a box for AMS purge towers, the
        // rib/cone solid for toolchangers. In tower-local millimetres; the
        // caller places it at wipe_tower_x/y. Absent (optional unset) on a
        // single-material plate, which has no tower.
        if (want_tower) {
            const auto& wtd = print.wipe_tower_data();
            if (wtd.wipe_tower_mesh_data) {
                // The printed tower is the body + its first-layer brim, both
                // in the same tower-local frame. Concatenate them into one
                // mesh (brim vertices appended after the body's, brim indices
                // shifted past them) so the overlay shows the full footprint
                // the user sees on the plate. Either may be empty.
                const indexed_triangle_set& body =
                    wtd.wipe_tower_mesh_data->real_wipe_tower_mesh.its;
                const indexed_triangle_set& brim =
                    wtd.wipe_tower_mesh_data->real_brim_mesh.its;
                const size_t body_v = body.vertices.size();
                const size_t vcount = body_v + brim.vertices.size();
                const size_t icount = body.indices.size() + brim.indices.size();
                if (vcount > 0 && icount > 0) {
                    float* verts = static_cast<float*>(
                        std::malloc(vcount * 3 * sizeof(float)));
                    uint32_t* idx = static_cast<uint32_t*>(
                        std::malloc(icount * 3 * sizeof(uint32_t)));
                    if (verts && idx) {
                        size_t v = 0;
                        for (size_t i = 0; i < body_v; ++i, ++v) {
                            verts[v * 3 + 0] = body.vertices[i].x();
                            verts[v * 3 + 1] = body.vertices[i].y();
                            verts[v * 3 + 2] = body.vertices[i].z();
                        }
                        for (size_t i = 0; i < brim.vertices.size(); ++i, ++v) {
                            verts[v * 3 + 0] = brim.vertices[i].x();
                            verts[v * 3 + 1] = brim.vertices[i].y();
                            verts[v * 3 + 2] = brim.vertices[i].z();
                        }
                        size_t t = 0;
                        for (size_t i = 0; i < body.indices.size(); ++i, ++t) {
                            idx[t * 3 + 0] = static_cast<uint32_t>(body.indices[i][0]);
                            idx[t * 3 + 1] = static_cast<uint32_t>(body.indices[i][1]);
                            idx[t * 3 + 2] = static_cast<uint32_t>(body.indices[i][2]);
                        }
                        for (size_t i = 0; i < brim.indices.size(); ++i, ++t) {
                            idx[t * 3 + 0] = static_cast<uint32_t>(brim.indices[i][0] + body_v);
                            idx[t * 3 + 1] = static_cast<uint32_t>(brim.indices[i][1] + body_v);
                            idx[t * 3 + 2] = static_cast<uint32_t>(brim.indices[i][2] + body_v);
                        }
                        *out_tower_vertices = verts;
                        *out_tower_vertex_count = vcount;
                        *out_tower_indices = idx;
                        *out_tower_index_count = icount;
                    } else {
                        std::free(verts);
                        std::free(idx);
                    }
                }
            }
        }

        // GCodeProcessorResult holds time/filament analysis. We must pass a
        // valid pointer (export_gcode populates it); we just drop it on return
        // since v0 doesn't surface preview data yet.
        GCodeProcessorResult gcode_result;
        print.export_gcode(out_path, &gcode_result, nullptr);
        return SLIC3R_OK;
    } catch (const SlicingErrors& e) {
        // libslic3r aggregates per-object slicing failures into SlicingErrors,
        // whose what() is the bare "Errors" — the real, actionable diagnoses
        // live in errors_ (each a SlicingError with the true message). Join
        // them so the caller sees the actual reason(s) instead of "Errors".
        // (Singular SlicingError carries its message in what(), so the generic
        // handler below already surfaces those.)
        std::string joined;
        for (const SlicingError& se : e.errors_) {
            if (!joined.empty()) joined += "\n";
            joined += se.what();
        }
        set_err(out_err, joined.empty() ? e.what() : joined);
        return SLIC3R_ERR_SLICE;
    } catch (const std::exception& e) {
        set_err(out_err, e.what());
        return SLIC3R_ERR_SLICE;
    }
}

slic3r_status slic3r_cancel(void) {
    std::lock_guard<std::mutex> lk(g_active_print_mtx);
    if (g_active_print) g_active_print->cancel();
    return SLIC3R_OK;
}

void slic3r_set_log_sink(slic3r_log_fn_t cb, void* user_data) {
    std::lock_guard<std::mutex> lk(g_log_mutex);
    g_log_cb = cb;
    g_log_user_data = user_data;
}

void slic3r_tower_mesh_free(float* vertices, uint32_t* indices) {
    std::free(vertices);
    std::free(indices);
}

slic3r_status slic3r_orient_mesh(const float* vertices, size_t vertex_count,
                                 const uint32_t* indices, size_t triangle_count,
                                 float overhang_angle, float out_quat_xyzw[4],
                                 char** out_err) {
    if (out_err) *out_err = nullptr;
    if (!vertices || !indices || !out_quat_xyzw || vertex_count == 0 || triangle_count == 0)
        return SLIC3R_ERR_INVALID_ARG;
    try {
        // Rebuild an indexed_triangle_set from the raw arrays (object-local coords).
        indexed_triangle_set its;
        its.vertices.reserve(vertex_count);
        for (size_t i = 0; i < vertex_count; ++i)
            its.vertices.emplace_back(vertices[i * 3 + 0], vertices[i * 3 + 1],
                                      vertices[i * 3 + 2]);
        its.indices.reserve(triangle_count);
        for (size_t i = 0; i < triangle_count; ++i)
            its.indices.emplace_back(static_cast<int32_t>(indices[i * 3 + 0]),
                                     static_cast<int32_t>(indices[i * 3 + 1]),
                                     static_cast<int32_t>(indices[i * 3 + 2]));

        Slic3r::orientation::OrientMesh om;
        om.mesh = TriangleMesh(its);
        if (overhang_angle > 0.f)
            om.overhang_angle = overhang_angle;

        Slic3r::orientation::OrientMeshs items;
        items.push_back(std::move(om));
        Slic3r::orientation::OrientMeshs excludes;
        Slic3r::orientation::OrientParams params;
        if (overhang_angle > 0.f)
            params.overhang_angle = overhang_angle;
        // _orient() invokes these unconditionally; the defaults are empty
        // std::functions, so leaving them throws std::bad_function_call. Supply
        // no-ops (we have no progress UI and never abort).
        params.progressind = [](unsigned, std::string) {};
        params.stopcondition = []() { return false; };
        Slic3r::orientation::orient(items, excludes, params);

        // orient() fills rotation_matrix (the rotation to apply); convert to a
        // unit quaternion for the Rust/glam side.
        const Eigen::Matrix3d& R = items.front().rotation_matrix;
        Eigen::Quaterniond q(R);
        q.normalize();
        out_quat_xyzw[0] = static_cast<float>(q.x());
        out_quat_xyzw[1] = static_cast<float>(q.y());
        out_quat_xyzw[2] = static_cast<float>(q.z());
        out_quat_xyzw[3] = static_cast<float>(q.w());
        return SLIC3R_OK;
    } catch (const std::exception& e) {
        set_err(out_err, e.what());
        return SLIC3R_ERR_INTERNAL;
    } catch (...) {
        set_err(out_err, "unknown error in slic3r_orient_mesh");
        return SLIC3R_ERR_INTERNAL;
    }
}

slic3r_status slic3r_cut_mesh(const float* vertices, size_t vertex_count,
                              const uint32_t* indices, size_t triangle_count,
                              const float plane_origin[3], const float plane_normal[3],
                              float** out_pos_vertices, size_t* out_pos_vertex_count,
                              uint32_t** out_pos_indices, size_t* out_pos_triangle_count,
                              float** out_neg_vertices, size_t* out_neg_vertex_count,
                              uint32_t** out_neg_indices, size_t* out_neg_triangle_count,
                              char** out_err) {
    if (out_err) *out_err = nullptr;
    // Default every output to "empty half" so a missing side is unambiguous.
    if (out_pos_vertices) *out_pos_vertices = nullptr;
    if (out_pos_vertex_count) *out_pos_vertex_count = 0;
    if (out_pos_indices) *out_pos_indices = nullptr;
    if (out_pos_triangle_count) *out_pos_triangle_count = 0;
    if (out_neg_vertices) *out_neg_vertices = nullptr;
    if (out_neg_vertex_count) *out_neg_vertex_count = 0;
    if (out_neg_indices) *out_neg_indices = nullptr;
    if (out_neg_triangle_count) *out_neg_triangle_count = 0;
    if (!vertices || !indices || !plane_origin || !plane_normal ||
        !out_pos_vertices || !out_pos_vertex_count || !out_pos_indices ||
        !out_pos_triangle_count || !out_neg_vertices || !out_neg_vertex_count ||
        !out_neg_indices || !out_neg_triangle_count ||
        vertex_count == 0 || triangle_count == 0)
        return SLIC3R_ERR_INVALID_ARG;
    try {
        Eigen::Vector3d n(plane_normal[0], plane_normal[1], plane_normal[2]);
        if (n.norm() < 1e-9)
            return SLIC3R_ERR_INVALID_ARG;
        n.normalize();
        Eigen::Vector3d o(plane_origin[0], plane_origin[1], plane_origin[2]);

        // cut_mesh only cuts the horizontal z=0 plane, so rotate the mesh so the
        // plane normal lands on +Z (then a point on the plane has z=0). q maps
        // normal→+Z; the inverse maps the cut halves back to the input frame.
        Eigen::Quaterniond q = Eigen::Quaterniond::FromTwoVectors(n, Eigen::Vector3d::UnitZ());
        Eigen::Quaterniond qi = q.conjugate();

        indexed_triangle_set its = its_from_buffers(vertices, vertex_count, indices, triangle_count);
        for (auto& v : its.vertices) {
            Eigen::Vector3d t = q * (Eigen::Vector3d(v.x(), v.y(), v.z()) - o);
            v = Vec3f(static_cast<float>(t.x()), static_cast<float>(t.y()),
                      static_cast<float>(t.z()));
        }

        // upper = z>0 = the side the normal points toward (positive); lower = neg.
        indexed_triangle_set upper, lower;
        cut_mesh(its, 0.0f, &upper, &lower, /*triangulate_caps=*/true);

        // Marshal a half to heap arrays, transforming each vertex back to the
        // input frame. Empty half → leave the (already-nulled) outputs.
        auto marshal = [&](const indexed_triangle_set& half, float** ov, size_t* ovc,
                           uint32_t** oi, size_t* oic) {
            if (half.vertices.empty() || half.indices.empty())
                return;
            float* verts = static_cast<float*>(std::malloc(half.vertices.size() * 3 * sizeof(float)));
            uint32_t* idx = static_cast<uint32_t*>(std::malloc(half.indices.size() * 3 * sizeof(uint32_t)));
            if (!verts || !idx) {
                std::free(verts);
                std::free(idx);
                return;
            }
            for (size_t i = 0; i < half.vertices.size(); ++i) {
                Eigen::Vector3d p =
                    qi * Eigen::Vector3d(half.vertices[i].x(), half.vertices[i].y(),
                                         half.vertices[i].z()) + o;
                verts[i * 3 + 0] = static_cast<float>(p.x());
                verts[i * 3 + 1] = static_cast<float>(p.y());
                verts[i * 3 + 2] = static_cast<float>(p.z());
            }
            for (size_t i = 0; i < half.indices.size(); ++i) {
                idx[i * 3 + 0] = static_cast<uint32_t>(half.indices[i][0]);
                idx[i * 3 + 1] = static_cast<uint32_t>(half.indices[i][1]);
                idx[i * 3 + 2] = static_cast<uint32_t>(half.indices[i][2]);
            }
            *ov = verts;
            *ovc = half.vertices.size();
            *oi = idx;
            *oic = half.indices.size();
        };
        marshal(upper, out_pos_vertices, out_pos_vertex_count, out_pos_indices, out_pos_triangle_count);
        marshal(lower, out_neg_vertices, out_neg_vertex_count, out_neg_indices, out_neg_triangle_count);
        return SLIC3R_OK;
    } catch (const std::exception& e) {
        set_err(out_err, e.what());
        return SLIC3R_ERR_INTERNAL;
    } catch (...) {
        set_err(out_err, "unknown error in slic3r_cut_mesh");
        return SLIC3R_ERR_INTERNAL;
    }
}

void slic3r_cut_mesh_free(float* vertices, uint32_t* indices) {
    std::free(vertices);
    std::free(indices);
}

// ---- Cut connectors (joints) -------------------------------------------

// Cross-section segment count per shape (Orca's get_connector_mesh).
static int conn_shape_sectors(int shape) {
    switch (shape) {
        case SLIC3R_CONN_TRIANGLE: return 3;
        case SLIC3R_CONN_SQUARE:   return 4;
        case SLIC3R_CONN_HEXAGON:  return 6;
        default:                   return 60; // Circle
    }
}

// One unit connector mesh (r=1, h=1), recentered so it straddles z=0 — so after
// scaling by height the peg sits half into each side of the cut plane. Mirrors
// Orca's get_connector_mesh decision tree; `force_cylinder` forces a plain
// cylinder (the hole shape for a Snap connector).
static indexed_triangle_set conn_unit_mesh(int type, int style, int sectors, bool force_cylinder) {
    indexed_triangle_set its;
    const double fa = 2.0 * PI / sectors;
    if (force_cylinder)                      its = its_make_cylinder(1.0, 1.0, fa);
    else if (type == SLIC3R_CONN_SNAP)       its = its_make_snap(1.0, 1.0);
    else if (style == SLIC3R_CONN_PRISM)     its = its_make_cylinder(1.0, 1.0, fa);
    else if (type == SLIC3R_CONN_PLUG)       its = its_make_frustum(1.0, 1.0, fa);
    else /* Dowel + Frustum */               its = its_make_frustum_dowel(1.0, 1.0, sectors);
    if (its.vertices.empty())
        return its;
    Vec3f lo = its.vertices[0], hi = its.vertices[0];
    for (const auto& v : its.vertices) { lo = lo.cwiseMin(v); hi = hi.cwiseMax(v); }
    const Vec3f c = 0.5f * (lo + hi);
    for (auto& v : its.vertices) v -= c;
    return its;
}

// Capture MMU color paint off the (un-rotated, local-frame) input mesh as a
// libslic3r SavedPainting, so it can be re-projected onto the cut halves.
// `nullopt` when no paint was supplied or nothing is painted (the common case,
// so we never build a Model for an unpainted cut). `modify_to_center_geometry =
// false` keeps the volume mesh in the input frame (init_shift 0) — both this and
// the restore target share that frame, so the remap is a pure spatial overlap.
static std::optional<TriangleSelector::SavedPainting>
conn_save_paint(const indexed_triangle_set& its, const char* const* in_paint, size_t tri_count) {
    if (!in_paint)
        return std::nullopt;
    bool any = false;
    for (size_t i = 0; i < tri_count; ++i)
        if (in_paint[i] && in_paint[i][0]) { any = true; break; }
    if (!any)
        return std::nullopt;
    Model model;
    ModelObject* obj = model.add_object();
    ModelVolume* vol = obj->add_volume(TriangleMesh(its), false);
    for (size_t i = 0; i < tri_count; ++i)
        if (in_paint[i] && in_paint[i][0])
            vol->mmu_segmentation_facets.set_triangle_from_string(static_cast<int>(i), in_paint[i]);
    return vol->save_painting();
}

// Re-project the saved paint onto a cut half (same local frame), reading back
// one FacetsAnnotation string per triangle. Cut faces + connector walls are new
// interior geometry with no matching source surface, so they remap to "".
static void conn_restore_paint(const indexed_triangle_set& half,
                               const std::optional<TriangleSelector::SavedPainting>& saved,
                               std::vector<std::string>& out) {
    out.assign(half.indices.size(), std::string());
    if (!saved)
        return;
    Model model;
    ModelObject* obj = model.add_object();
    ModelVolume* vol = obj->add_volume(TriangleMesh(half), false);
    vol->restore_painting(saved);
    for (size_t i = 0; i < half.indices.size(); ++i)
        out[i] = vol->mmu_segmentation_facets.get_triangle_as_string(static_cast<int>(i));
}

// Inverse-rotate `its` from the cut-aligned frame back to the input frame.
static void conn_unalign(indexed_triangle_set& its, const Eigen::Quaterniond& qi,
                         const Eigen::Vector3d& o) {
    for (auto& v : its.vertices) {
        const Eigen::Vector3d p = qi * Eigen::Vector3d(v.x(), v.y(), v.z()) + o;
        v = Vec3f(static_cast<float>(p.x()), static_cast<float>(p.y()), static_cast<float>(p.z()));
    }
}

// Heap-copy per-triangle paint strings into a char* array (one malloc'd string
// each). Empty input → null. Caller frees with slic3r_cut_connectors_free_paint.
static char** conn_marshal_paint(const std::vector<std::string>& strs) {
    if (strs.empty())
        return nullptr;
    char** arr = static_cast<char**>(std::malloc(strs.size() * sizeof(char*)));
    if (!arr)
        return nullptr;
    for (size_t i = 0; i < strs.size(); ++i)
        arr[i] = dup_c(strs[i]);
    return arr;
}

// Copy `its` and apply the affine `T` to every vertex.
static indexed_triangle_set conn_place(indexed_triangle_set its, const Transform3d& T) {
    for (auto& v : its.vertices) {
        const Eigen::Vector3d p = T * Eigen::Vector3d(v.x(), v.y(), v.z());
        v = Vec3f(static_cast<float>(p.x()), static_cast<float>(p.y()), static_cast<float>(p.z()));
    }
    return its;
}

// Marshal one half to heap arrays, inverse-rotating each vertex back to the
// input frame (qi, +o). Empty half → null/0. Returns false only on malloc fail.
static bool conn_marshal(const indexed_triangle_set& half, const Eigen::Quaterniond& qi,
                         const Eigen::Vector3d& o, float** ov, size_t* ovc, uint32_t** oi,
                         size_t* oic) {
    *ov = nullptr; *ovc = 0; *oi = nullptr; *oic = 0;
    if (half.vertices.empty() || half.indices.empty())
        return true;
    float* verts = static_cast<float*>(std::malloc(half.vertices.size() * 3 * sizeof(float)));
    uint32_t* idx = static_cast<uint32_t*>(std::malloc(half.indices.size() * 3 * sizeof(uint32_t)));
    if (!verts || !idx) { std::free(verts); std::free(idx); return false; }
    for (size_t i = 0; i < half.vertices.size(); ++i) {
        const Eigen::Vector3d p =
            qi * Eigen::Vector3d(half.vertices[i].x(), half.vertices[i].y(), half.vertices[i].z()) + o;
        verts[i * 3 + 0] = static_cast<float>(p.x());
        verts[i * 3 + 1] = static_cast<float>(p.y());
        verts[i * 3 + 2] = static_cast<float>(p.z());
    }
    for (size_t i = 0; i < half.indices.size(); ++i) {
        idx[i * 3 + 0] = static_cast<uint32_t>(half.indices[i][0]);
        idx[i * 3 + 1] = static_cast<uint32_t>(half.indices[i][1]);
        idx[i * 3 + 2] = static_cast<uint32_t>(half.indices[i][2]);
    }
    *ov = verts; *ovc = half.vertices.size(); *oi = idx; *oic = half.indices.size();
    return true;
}

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
    char** out_err) {
    if (out_err) *out_err = nullptr;
    if (out_pos_vertices) *out_pos_vertices = nullptr;
    if (out_pos_vertex_count) *out_pos_vertex_count = 0;
    if (out_pos_indices) *out_pos_indices = nullptr;
    if (out_pos_triangle_count) *out_pos_triangle_count = 0;
    if (out_pos_paint) *out_pos_paint = nullptr;
    if (out_neg_vertices) *out_neg_vertices = nullptr;
    if (out_neg_vertex_count) *out_neg_vertex_count = 0;
    if (out_neg_indices) *out_neg_indices = nullptr;
    if (out_neg_triangle_count) *out_neg_triangle_count = 0;
    if (out_neg_paint) *out_neg_paint = nullptr;
    if (out_dowel_vertices) *out_dowel_vertices = nullptr;
    if (out_dowel_vertex_counts) *out_dowel_vertex_counts = nullptr;
    if (out_dowel_indices) *out_dowel_indices = nullptr;
    if (out_dowel_triangle_counts) *out_dowel_triangle_counts = nullptr;
    if (out_dowel_count) *out_dowel_count = 0;
    if (!vertices || !indices || !plane_origin || !plane_normal || !out_pos_vertices ||
        !out_neg_vertices || !out_dowel_count || vertex_count == 0 || triangle_count == 0)
        return SLIC3R_ERR_INVALID_ARG;
    if (connector_count > 0 && (!connector_floats || !connector_ints))
        return SLIC3R_ERR_INVALID_ARG;
    try {
        Eigen::Vector3d n(plane_normal[0], plane_normal[1], plane_normal[2]);
        if (n.norm() < 1e-9)
            return SLIC3R_ERR_INVALID_ARG;
        n.normalize();
        const Eigen::Vector3d o(plane_origin[0], plane_origin[1], plane_origin[2]);
        const Eigen::Quaterniond q = Eigen::Quaterniond::FromTwoVectors(n, Eigen::Vector3d::UnitZ());
        const Eigen::Quaterniond qi = q.conjugate();

        indexed_triangle_set its = its_from_buffers(vertices, vertex_count, indices, triangle_count);
        // Capture paint off the original (un-rotated, local-frame) mesh first.
        const std::optional<TriangleSelector::SavedPainting> saved =
            conn_save_paint(its, in_paint, triangle_count);
        for (auto& v : its.vertices) {
            const Eigen::Vector3d t = q * (Eigen::Vector3d(v.x(), v.y(), v.z()) - o);
            v = Vec3f(static_cast<float>(t.x()), static_cast<float>(t.y()), static_cast<float>(t.z()));
        }

        // upper = z>0 = positive side; lower = negative side. (aligned frame)
        indexed_triangle_set upper, lower;
        cut_mesh(its, 0.0f, &upper, &lower, /*triangulate_caps=*/true);

        std::vector<indexed_triangle_set> dowels;
        for (size_t ci = 0; ci < connector_count; ++ci) {
            const float* cf = connector_floats + ci * 8;
            const int32_t* cn = connector_ints + ci * 3;
            const int type = cn[0], style = cn[1], shape = cn[2];
            const double radius = cf[3], height = cf[4];
            const double r_tol = cf[5], h_tol = cf[6], z_angle = cf[7];
            if (radius <= 1e-6 || height <= 1e-6)
                continue; // degenerate → skip
            const int sectors = conn_shape_sectors(shape);
            const Eigen::Vector3d pa = q * (Eigen::Vector3d(cf[0], cf[1], cf[2]) - o);

            const Transform3d peg_T =
                Geometry::translation_transform(pa) *
                Geometry::rotation_transform(Eigen::Vector3d(0, 0, -z_angle)) *
                Geometry::scale_transform(Eigen::Vector3d(radius, radius, height));
            // The hole is the peg widened by tolerance, shifted along the axis by
            // half the depth tolerance (so the extra depth is on the open side).
            const Transform3d hole_T =
                Geometry::translation_transform(pa + Eigen::Vector3d(0, 0, 0.5 * h_tol)) *
                Geometry::rotation_transform(Eigen::Vector3d(0, 0, -z_angle)) *
                Geometry::scale_transform(
                    Eigen::Vector3d(radius + r_tol, radius + r_tol, height + h_tol));

            const indexed_triangle_set peg =
                conn_place(conn_unit_mesh(type, style, sectors, false), peg_T);
            const indexed_triangle_set hole = conn_place(
                conn_unit_mesh(type, style, type == SLIC3R_CONN_SNAP ? 60 : sectors,
                               type == SLIC3R_CONN_SNAP),
                hole_T);

            try {
                if (type == SLIC3R_CONN_DOWEL) {
                    // Hole in BOTH halves; the pin is printed separately.
                    indexed_triangle_set u = upper, l = lower;
                    MeshBoolean::cgal::minus(u, hole);
                    MeshBoolean::cgal::minus(l, hole);
                    upper = std::move(u);
                    lower = std::move(l);
                    dowels.push_back(peg);
                } else {
                    // Plug / Snap: solid peg in the neg half, matching hole in pos.
                    indexed_triangle_set ph = lower, hh = upper;
                    MeshBoolean::cgal::plus(ph, peg);
                    MeshBoolean::cgal::minus(hh, hole);
                    lower = std::move(ph);
                    upper = std::move(hh);
                }
            } catch (const std::exception& e) {
                BOOST_LOG_TRIVIAL(warning)
                    << "slic3r_cut_mesh_connectors: connector " << ci << " skipped: " << e.what();
            } catch (...) {
                BOOST_LOG_TRIVIAL(warning)
                    << "slic3r_cut_mesh_connectors: connector " << ci << " skipped (unknown)";
            }
        }

        if (!conn_marshal(upper, qi, o, out_pos_vertices, out_pos_vertex_count, out_pos_indices,
                          out_pos_triangle_count) ||
            !conn_marshal(lower, qi, o, out_neg_vertices, out_neg_vertex_count, out_neg_indices,
                          out_neg_triangle_count)) {
            set_err(out_err, "out of memory marshalling cut halves");
            return SLIC3R_ERR_INTERNAL;
        }

        // Re-project paint onto each half. Restore needs the halves in the input
        // frame (where `saved` lives), so un-rotate copies — the marshaled
        // geometry above is unchanged.
        if (saved) {
            indexed_triangle_set up_o = upper, lo_o = lower;
            conn_unalign(up_o, qi, o);
            conn_unalign(lo_o, qi, o);
            std::vector<std::string> pos_paint, neg_paint;
            conn_restore_paint(up_o, saved, pos_paint);
            conn_restore_paint(lo_o, saved, neg_paint);
            if (out_pos_paint) *out_pos_paint = conn_marshal_paint(pos_paint);
            if (out_neg_paint) *out_neg_paint = conn_marshal_paint(neg_paint);
        }

        if (!dowels.empty()) {
            const size_t nd = dowels.size();
            float** dv = static_cast<float**>(std::malloc(nd * sizeof(float*)));
            uint32_t** di = static_cast<uint32_t**>(std::malloc(nd * sizeof(uint32_t*)));
            size_t* dvc = static_cast<size_t*>(std::malloc(nd * sizeof(size_t)));
            size_t* dtc = static_cast<size_t*>(std::malloc(nd * sizeof(size_t)));
            if (dv && di && dvc && dtc) {
                for (size_t k = 0; k < nd; ++k)
                    conn_marshal(dowels[k], qi, o, &dv[k], &dvc[k], &di[k], &dtc[k]);
                if (out_dowel_vertices) *out_dowel_vertices = dv;
                if (out_dowel_indices) *out_dowel_indices = di;
                if (out_dowel_vertex_counts) *out_dowel_vertex_counts = dvc;
                if (out_dowel_triangle_counts) *out_dowel_triangle_counts = dtc;
                if (out_dowel_count) *out_dowel_count = nd;
            } else {
                std::free(dv); std::free(di); std::free(dvc); std::free(dtc);
            }
        }
        return SLIC3R_OK;
    } catch (const std::exception& e) {
        set_err(out_err, e.what());
        return SLIC3R_ERR_INTERNAL;
    } catch (...) {
        set_err(out_err, "unknown error in slic3r_cut_mesh_connectors");
        return SLIC3R_ERR_INTERNAL;
    }
}

void slic3r_cut_connectors_free_dowels(
    float** dowel_vertices, uint32_t** dowel_indices,
    size_t* dowel_vertex_counts, size_t* dowel_triangle_counts, size_t dowel_count) {
    if (dowel_vertices) {
        for (size_t k = 0; k < dowel_count; ++k)
            std::free(dowel_vertices[k]);
        std::free(dowel_vertices);
    }
    if (dowel_indices) {
        for (size_t k = 0; k < dowel_count; ++k)
            std::free(dowel_indices[k]);
        std::free(dowel_indices);
    }
    std::free(dowel_vertex_counts);
    std::free(dowel_triangle_counts);
}

void slic3r_cut_connectors_free_paint(char** paint, size_t count) {
    if (!paint)
        return;
    for (size_t i = 0; i < count; ++i)
        std::free(paint[i]);
    std::free(paint);
}

slic3r_status slic3r_arrange(const double* contours, const size_t* contour_lengths,
                             size_t item_count, const double* exclude_rects,
                             size_t exclude_count, size_t bed_count, double bed_w,
                             double bed_h, double min_dist, int allow_rotations,
                             double* out_dx_dy, double* out_rotation,
                             int* out_bed_idx, char** out_err) {
    if (out_err) *out_err = nullptr;
    if (!contours || !contour_lengths || !out_dx_dy || !out_rotation || !out_bed_idx
        || item_count == 0 || bed_w <= 0.0 || bed_h <= 0.0
        || (exclude_count > 0 && !exclude_rects))
        return SLIC3R_ERR_INVALID_ARG;
    try {
        // Build the arrange items from the flattened mm contours (libnest2d
        // wants convex polygons in scaled integer coords).
        arrangement::ArrangePolygons items;
        items.reserve(item_count);
        size_t pair_off = 0; // running offset into `contours`, counted in xy pairs
        for (size_t i = 0; i < item_count; ++i) {
            size_t n = contour_lengths[i];
            if (n < 3) {
                set_err(out_err, "each arrange item needs at least 3 contour points");
                return SLIC3R_ERR_INVALID_ARG;
            }
            Polygon contour;
            contour.points.reserve(n);
            for (size_t k = 0; k < n; ++k) {
                double x = contours[(pair_off + k) * 2 + 0];
                double y = contours[(pair_off + k) * 2 + 1];
                contour.points.emplace_back(scaled<coord_t>(x), scaled<coord_t>(y));
            }
            pair_off += n;
            arrangement::ArrangePolygon ap;
            ap.poly = ExPolygon(contour);
            // _arrange() zeroes min_obj_distance and expects items to carry the
            // spacing as inflation (mirrors update_selected_items_inflation).
            ap.inflation = scaled<coord_t>(min_dist / 2.0);
            // The default bed_idx is UNARRANGED (-1) — but the nester's
            // BIN_ID_UNFIT is also -1, so it would skip every item as
            // "already unfit". Start them at bed 0 (a real bed) instead.
            ap.bed_idx = 0;
            items.push_back(std::move(ap));
        }

        arrangement::ArrangeParams params;
        params.min_obj_distance = scaled<coord_t>(min_dist);
        params.allow_rotations = allow_rotations != 0;
        // Silence the chatty default progress printer; supply the stop predicate
        // (its default is an empty std::function, which would throw if invoked).
        params.progressind = [](unsigned, std::string) {};
        params.stopcondition = []() { return false; };

        // No-go regions (AMS feed zones, the wipe/prime tower): axis-aligned
        // rects (minx, miny, maxx, maxy). These go in as **fixed items**, not
        // ArrangeParams::excluded_regions — the latter is only a soft scoring
        // penalty the nester overrides on a crowded bed, whereas a fixed item is
        // preloaded and hard-avoided via the no-fit polygon (matching how
        // OrcaSlicer pins the wipe tower). `is_virt_object` keeps it immovable
        // and exempt from the oversize-item cull. The regions are per-plate
        // hardware/geometry present on every bed, so each is reserved on every
        // bed the packer might open (0 .. bed_count) — exactly how OrcaSlicer's
        // prepare_wipe_tower replicates the tower across MAX_NUM_PLATES. Without
        // this, items spilled onto an extra bed would sit on that bed's tower.
        size_t beds = bed_count > 0 ? bed_count : 1;
        arrangement::ArrangePolygons fixed;
        for (size_t e = 0; e < exclude_count; ++e) {
            // Normalize so the rect is well-formed regardless of corner order;
            // an inverted (minx > maxx) rect would build a self-intersecting
            // polygon and confuse libnest2d's collision tests.
            double x0 = std::min(exclude_rects[e * 4 + 0], exclude_rects[e * 4 + 2]);
            double y0 = std::min(exclude_rects[e * 4 + 1], exclude_rects[e * 4 + 3]);
            double x1 = std::max(exclude_rects[e * 4 + 0], exclude_rects[e * 4 + 2]);
            double y1 = std::max(exclude_rects[e * 4 + 1], exclude_rects[e * 4 + 3]);
            if (x1 - x0 <= 0.0 || y1 - y0 <= 0.0) {
                continue; // skip a zero-area exclusion rect
            }
            Polygon r;
            r.points = {
                Point(scaled<coord_t>(x0), scaled<coord_t>(y0)),
                Point(scaled<coord_t>(x1), scaled<coord_t>(y0)),
                Point(scaled<coord_t>(x1), scaled<coord_t>(y1)),
                Point(scaled<coord_t>(x0), scaled<coord_t>(y1)),
            };
            for (size_t b = 0; b < beds; ++b) {
                arrangement::ArrangePolygon ex;
                ex.poly = ExPolygon(r);
                ex.is_virt_object = true;
                ex.bed_idx = static_cast<int>(b);
                fixed.push_back(ex);
            }
        }

        BoundingBox bed(Point(0, 0),
                        Point(scaled<coord_t>(bed_w), scaled<coord_t>(bed_h)));
        arrangement::arrange(items, fixed, bed, params);

        // Results are written back in place, item order preserved.
        for (size_t i = 0; i < item_count; ++i) {
            out_dx_dy[i * 2 + 0] = unscaled<double>(items[i].translation.x());
            out_dx_dy[i * 2 + 1] = unscaled<double>(items[i].translation.y());
            out_rotation[i] = items[i].rotation;
            out_bed_idx[i] = items[i].bed_idx;
        }
        return SLIC3R_OK;
    } catch (const std::exception& e) {
        set_err(out_err, e.what());
        return SLIC3R_ERR_INTERNAL;
    } catch (...) {
        set_err(out_err, "unknown error in slic3r_arrange");
        return SLIC3R_ERR_INTERNAL;
    }
}

} // extern "C"
