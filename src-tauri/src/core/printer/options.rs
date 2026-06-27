//! Settings-UI option surfacing.
//!
//! Translates libslic3r's flat option table into the wire shapes the
//! settings panels consume (FR-UI-2/6/7): per-bucket option summaries
//! (Process / Printer / per-extruder / Filament), typed defaults,
//! display ordering, and per-printer capability gating. Depends on the
//! `slic3r_ffi` option metadata plus this module's sibling capability
//! predicates ([`capability_for_key`], [`CapabilityPredicate`]) and the
//! [`PrinterProfile`] they evaluate against — it does not touch the
//! cascade resolver or `core::schema`.

use serde::Serialize;
use slic3r_ffi::{
    display_order_of, option_defs, OptMode as FfiOptMode, OptScope as FfiOptScope, OptType,
};

use super::profile::PrinterProfile;
use super::{capability_for_key, CapabilityPredicate};

/// Wire-format mode for the FR-UI-2 Simple / Advanced / Expert
/// filter. Mirrors `slic3r_ffi::OptMode` but serializes as a stable
/// lowercase string so the TS side gets a tagged-string enum, not
/// a numeric variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum OptMode {
    Simple,
    Advanced,
    Expert,
    Develop,
}

impl From<FfiOptMode> for OptMode {
    fn from(m: FfiOptMode) -> Self {
        match m {
            FfiOptMode::Simple => Self::Simple,
            FfiOptMode::Advanced => Self::Advanced,
            FfiOptMode::Expert => Self::Expert,
            FfiOptMode::Develop => Self::Develop,
        }
    }
}

/// Wire-format scope flags. The FFI's `OptScope` is a u32 bitmask
/// over libslic3r's config classes; the UI only cares about three
/// of them (project / object / region) for FR-3D-3's Object-tab
/// read-only gating. Serializes as a struct of bools so the
/// frontend doesn't need to know the underlying bit positions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct OptScopeFlags {
    pub project: bool,
    pub object: bool,
    pub region: bool,
}

impl From<FfiOptScope> for OptScopeFlags {
    fn from(s: FfiOptScope) -> Self {
        Self {
            project: s.is_print(),
            object: s.is_object(),
            region: s.is_region(),
        }
    }
}

/// Typed default-value wire shape.
///
/// libslic3r serializes every option to a single string, but its
/// vector types (`coStrings`, `coFloats`, `coInts`, …) flatten into
/// forms that look like gibberish if the frontend tries to render
/// them verbatim:
///
///   - `coStrings` uses `escape_strings_cstyle`: entries joined by
///     `;`, with whitespace/quote/newline-containing entries wrapped
///     in `"..."` and c-style-escaped. Showing this to a user as the
///     "default" reads as `0,0;"\n0.2,0.4444";"\n0.4,0.6145";...`
///     instead of one entry per line.
///   - `coFloats` / `coInts` / `coPercents` use comma-joined forms.
///     The frontend wants one entry per slot/extruder; index 0 is
///     not the same logical thing as the whole vector for display.
///
/// Splitting on the Rust side gives each Field component access to
/// the right shape: `Scalar` for the obvious one-value-per-option
/// case, `Vector` carrying pre-split entries for everything else.
/// Both variants carry strings (not typed scalars) — the per-Field
/// parsing (`parseFloat`, `parseBool`, …) already lives on the
/// frontend; we don't move it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DefaultValue {
    Scalar { value: String },
    Vector { values: Vec<String> },
}

impl DefaultValue {
    /// Parse libslic3r's `default_serialized` into the typed wire
    /// shape based on the option's [`OptType`].
    ///
    /// Vector types go through type-specific deserialization:
    ///   - `Strings` → `unescape_strings_cstyle` (`;`-split with
    ///     quote handling), mirroring libslic3r's own deserializer.
    ///   - other vectors → simple comma split.
    pub fn from_serialized(ty: OptType, serialized: &str) -> Self {
        if !ty.is_vector() {
            return Self::Scalar {
                value: serialized.to_owned(),
            };
        }
        let values = if matches!(ty, OptType::Strings) {
            unescape_strings_cstyle(serialized)
        } else if serialized.is_empty() {
            Vec::new()
        } else {
            serialized.split(',').map(str::to_owned).collect()
        };
        Self::Vector { values }
    }
}

/// Mirror of libslic3r's `unescape_strings_cstyle` (Config.cpp:146).
///
/// Splits a serialized `coStrings` value into its entries:
/// `;`-separated, leading whitespace skipped per entry, quoted
/// entries (`"..."`) c-style-unescape `\n`/`\r`/`\\`/`\"`.
fn unescape_strings_cstyle(s: &str) -> Vec<String> {
    let bytes = s.as_bytes();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        // Skip leading whitespace.
        while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let mut buf: Vec<u8> = Vec::new();
        if bytes[i] == b'"' {
            i += 1;
            while i < bytes.len() && bytes[i] != b'"' {
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    i += 1;
                    match bytes[i] {
                        b'n' => buf.push(b'\n'),
                        b'r' => buf.push(b'\r'),
                        c => buf.push(c),
                    }
                    i += 1;
                } else {
                    buf.push(bytes[i]);
                    i += 1;
                }
            }
            if i < bytes.len() {
                i += 1; // closing quote
            }
        } else {
            while i < bytes.len() && bytes[i] != b';' {
                buf.push(bytes[i]);
                i += 1;
            }
        }
        out.push(String::from_utf8_lossy(&buf).into_owned());
        // Skip whitespace before the separator.
        while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
            i += 1;
        }
        if i < bytes.len() && bytes[i] == b';' {
            i += 1;
            if i == bytes.len() {
                // Trailing `;` → one empty entry (matches libslic3r).
                out.push(String::new());
            }
        }
    }
    out
}

#[derive(Serialize)]
pub struct OptionSummary {
    pub key: String,
    pub ty: String,
    pub label: Option<String>,
    pub category: Option<String>,
    /// Optgroup within the category (page) — e.g. "Printable space" under
    /// the "Basic information" page. The printer panel renders these as
    /// sub-headers within a page. `None` for options with no sub-group
    /// (Process-panel options, per-extruder keys, unscraped keys).
    pub group: Option<String>,
    /// Typed default. `None` when libslic3r has no compile-time
    /// default for this option. See [`DefaultValue`] for the wire
    /// shape and why vectors are pre-split server-side.
    pub default_value: Option<DefaultValue>,
    /// True for libslic3r options flagged `multiline = true` —
    /// freeform text areas (start_gcode, end_gcode, the small-area
    /// infill flow compensation model). The frontend renders these
    /// as textareas with `\n`-joined display of the vector default
    /// instead of an index-by-slot picker.
    pub multiline: bool,
    /// Enum value/label pairs in libslic3r declaration order. Empty
    /// for non-enum types. The frontend's DropdownInput consumes
    /// this directly — no per-key lookup needed at render time.
    pub enum_values: Vec<(String, String)>,
    /// libslic3r tooltip text (FR-UI-6, tooltip surface
    /// consumes this).
    pub tooltip: Option<String>,
    /// Unit suffix shown after the input (mm, mm/s, %, °C, …), from
    /// libslic3r's `sidetext`. `None` for unitless options.
    pub sidetext: Option<String>,
    /// Simple / Advanced / Expert / Develop — drives the FR-UI-2
    /// mode filter on the settings panel.
    pub mode: OptMode,
    /// Project / object / region scope bitmask — drives the
    /// Object-tab "project-scope setting" read-only badge.
    pub scope: OptScopeFlags,
    /// Printer-capability predicate that gates this option's
    /// visibility (FR-UI-7). `None` = always show.
    /// [`panel_option_summaries`] returns the predicate verbatim;
    /// `slicer_options_for_printer` returns the same data plus a
    /// pre-evaluated `hidden` flag against the supplied printer.
    pub capability: Option<CapabilityPredicate>,
}

fn summary_from_def(d: slic3r_ffi::OptionDef) -> OptionSummary {
    let default_value = d
        .default_serialized
        .as_deref()
        .map(|s| DefaultValue::from_serialized(d.ty, s));
    // Zip values + labels — libslic3r's enum_values is the canonical
    // wire key, enum_labels is the (possibly-shorter) human label
    // list. When labels run out, fall back to the value as its own
    // label so the dropdown still renders something readable.
    let enum_values = d
        .enum_values
        .iter()
        .cloned()
        .zip(
            d.enum_labels
                .iter()
                .cloned()
                .chain(std::iter::repeat(String::new())),
        )
        .collect();
    OptionSummary {
        capability: capability_for_key(&d.key),
        group: slic3r_ffi::printer_subgroup_of(&d.key).map(str::to_owned),
        key: d.key,
        ty: format!("{:?}", d.ty),
        label: d.label,
        category: d.category,
        default_value,
        multiline: d.multiline,
        enum_values,
        tooltip: d.tooltip,
        sidetext: d.sidetext,
        mode: d.mode.into(),
        scope: d.scope.into(),
    }
}

fn matches_filter(d: &slic3r_ffi::OptionDef, needle: &str) -> bool {
    needle.is_empty()
        || d.key.to_lowercase().contains(needle)
        || d.label
            .as_deref()
            .is_some_and(|s| s.to_lowercase().contains(needle))
}

/// Settings-panel-visible options: Process bucket *and* actually settable
/// in an FFF print config. Printer + filament editing lives on other
/// surfaces; metadata keys (`compatible_printers`, `inherits`, …) and
/// SLA-only keys have no Process bucket and are excluded by that test.
///
/// The `is_fff` guard additionally drops *dangling* options — ones the
/// `ConfigDef` defines (so they carry a label + Process bucket) but that
/// aren't a member of any FFF config class (`PrintConfig` /
/// `PrintObjectConfig` / `PrintRegionConfig`), so their FFI scope bitmask
/// is empty. `ironing_expansion` is the lone current case: nothing reads
/// it, so surfacing it only lets the user author an override that's inert
/// at slice time. Keyed on the FFI scope signal, not a curated denylist.
fn is_panel_visible(d: &slic3r_ffi::OptionDef) -> bool {
    d.bucket == Some(slic3r_ffi::OptBucket::Process) && d.scope.is_fff()
}

/// Settings-panel option summaries for `filter` (matched against the
/// canonical key and display label, case-insensitively), sorted into
/// Orca's UI display order. Capability predicates are returned verbatim,
/// not evaluated against any printer. Shared core of
/// [`slicer_options_for_printer`], which adds the per-printer `hidden`
/// flag.
fn panel_option_summaries(filter: Option<String>) -> Vec<OptionSummary> {
    let needle = filter.unwrap_or_default().to_lowercase();
    let mut out: Vec<OptionSummary> = option_defs()
        .into_iter()
        .filter(is_panel_visible)
        .filter(|d| matches_filter(d, &needle))
        .map(summary_from_def)
        .collect();
    sort_by_display_order(&mut out, |s| &s.key);
    out
}

/// The engine's compiled-in default for `key`, serialized exactly as
/// libslic3r emits it (e.g. `wipe_tower_x` → `"15"`). Exact-key match
/// over the raw FFI option table — unlike [`panel_option_summaries`] it applies
/// no panel-visibility or capability filter, so capability-gated keys
/// (the `wipe_tower_*` family) still return their default. `None` when
/// libslic3r has no compile-time default for the key.
pub fn engine_default_serialized(key: &str) -> Option<String> {
    option_defs()
        .into_iter()
        .find(|d| d.key == key)
        .and_then(|d| d.default_serialized)
}

/// Stable-sort options by their position in Orca's hand-curated
/// Tab.cpp UI layout (scraped at build time via
/// `scripts/scrape_option_display_order.py`). Keys absent from the
/// table (internal/deprecated options) sort to the end while
/// preserving libslic3r's `option_defs()` registration order among
/// themselves.
fn sort_by_display_order<T, F>(items: &mut [T], key: F)
where
    F: Fn(&T) -> &str,
{
    items.sort_by_key(|item| display_order_of(key(item)).unwrap_or(u32::MAX));
}

/// Per-option printer-aware view (/ FR-UI-7). Same shape as
/// [`OptionSummary`] plus a pre-evaluated `hidden` flag derived from
/// the option's [`CapabilityPredicate`] against the supplied
/// [`PrinterProfile`]. Frontend calls this once per printer-switch;
/// per-row capability evaluation in the hot render path is then a
/// single field read instead of a function call.
#[derive(Serialize)]
pub struct PrinterAwareOptionSummary {
    #[serde(flatten)]
    pub summary: OptionSummary,
    /// True when this option's capability predicate is **not**
    /// satisfied by the printer (i.e. the row should be hidden in
    /// `mode='hide'`, or badged "not applicable" in `mode='search'`).
    /// `false` when the predicate is satisfied OR when there's no
    /// predicate at all.
    pub hidden: bool,
}

/// Per-option printer-aware view: [`panel_option_summaries`] plus each
/// option's capability predicate pre-evaluated against the supplied
/// printer. Filter behavior matches the shared core.
#[tauri::command]
#[tracing::instrument(skip(printer))]
pub fn slicer_options_for_printer(
    printer: PrinterProfile,
    filter: Option<String>,
) -> Vec<PrinterAwareOptionSummary> {
    let out: Vec<PrinterAwareOptionSummary> = panel_option_summaries(filter)
        .into_iter()
        .map(|summary| {
            let hidden = summary
                .capability
                .map(|c| !c.satisfied_by(&printer))
                .unwrap_or(false);
            PrinterAwareOptionSummary { summary, hidden }
        })
        .collect();
    let hidden_count = out.iter().filter(|s| s.hidden).count();
    tracing::info!(
        matched = out.len(),
        hidden = hidden_count,
        printer = %printer.model,
        "slicer_options_for_printer",
    );
    out
}

/// Machine/printer-settings-visible options: the **Printer** bucket,
/// minus readonly and SLA-only keys. Unlike [`is_panel_visible`] this is
/// *not* gated on `is_fff()` — that bitmask covers Print/Object/Region
/// only, and Printer-bucket options live in the printer preset with no
/// Process scope, so the guard would drop every one of them.
fn is_machine_visible(d: &slic3r_ffi::OptionDef) -> bool {
    if d.bucket != Some(slic3r_ffi::OptBucket::Printer) || d.readonly || d.scope.is_sla() {
        return false;
    }
    // The machine-limits family (libslic3r category "Machine limits"):
    // always shown, including the per-mode `machine_max_*` vectors (Orca
    // builds these via `append_option_line` + string concatenation, so they
    // carry no scraped editor — the category is their signal). Rendered
    // Normal/Silent.
    if d.category.as_deref() == Some("Machine limits") {
        return true;
    }
    // Everything else: only scalars/multiline that Orca actually lays out an
    // editor for (`printer_page_of` is Some). This drops capability flags
    // (silent_mode, support_*), metadata (printer_model — BBL detection,
    // printer_variant, default_*), and print-host/network keys (the
    // Connection section owns those) that are Printer-bucket but not
    // user-tunable here. Non-machine-limits vectors are per-extruder
    // (extruder tabs) or per-bed-type (no editor yet).
    !d.ty.is_vector() && slic3r_ffi::printer_page_of(&d.key).is_some()
}

fn is_extruder_visible(d: &slic3r_ffi::OptionDef) -> bool {
    // Per-extruder settings: Printer-bucket keys in libslic3r's per-extruder
    // set (`is_per_extruder`) that Orca *also* lays out an editor for
    // (`printer_page_of` is Some). The membership test alone includes keys
    // whose editor is commented out / handled elsewhere (extruder_colour,
    // default_filament_profile, nozzle_flush_dataset, …); like the machine
    // panel we only surface what's actually settable.
    d.bucket == Some(slic3r_ffi::OptBucket::Printer)
        && !d.readonly
        && !d.scope.is_sla()
        && slic3r_ffi::is_per_extruder(&d.key)
        && slic3r_ffi::printer_page_of(&d.key).is_some()
}

/// Shared summary builder for the printer-bucket surfaces. Filters by
/// `visible`, then overrides each option's `category` with its Orca
/// `TabPrinter` grouping (`printer_page_of` — page for machine-wide keys,
/// optgroup for per-extruder keys), since printer options carry no
/// libslic3r `category` of their own.
fn printer_bucket_summaries(
    filter: Option<String>,
    visible: fn(&slic3r_ffi::OptionDef) -> bool,
) -> Vec<OptionSummary> {
    let needle = filter.unwrap_or_default().to_lowercase();
    let mut out: Vec<OptionSummary> = option_defs()
        .into_iter()
        .filter(|d| visible(d))
        .filter(|d| matches_filter(d, &needle))
        .map(summary_from_def)
        .map(|mut s| {
            // Keep the machine-limits family under libslic3r's "Machine
            // limits" so the per-mode set groups together; everything else
            // uses its scraped Orca page/optgroup.
            if s.category.as_deref() != Some("Machine limits") {
                if let Some(cat) = slic3r_ffi::printer_page_of(&s.key) {
                    s.category = Some(cat.to_owned());
                }
            }
            s
        })
        .collect();
    sort_by_display_order(&mut out, |s| &s.key);
    out
}

fn machine_option_summaries(filter: Option<String>) -> Vec<OptionSummary> {
    printer_bucket_summaries(filter, is_machine_visible)
}

/// Printer-bucket ("machine settings") option summaries, printer-aware —
/// the [`slicer_options_for_printer`] analogue for the per-printer
/// settings surface in the printer panel. Capability predicates are
/// pre-evaluated against the supplied printer, same as the Process panel.
#[tauri::command]
#[tracing::instrument(skip(printer))]
pub fn slicer_machine_options_for_printer(
    printer: PrinterProfile,
    filter: Option<String>,
) -> Vec<PrinterAwareOptionSummary> {
    machine_option_summaries(filter)
        .into_iter()
        .map(|summary| {
            let hidden = summary
                .capability
                .map(|c| !c.satisfied_by(&printer))
                .unwrap_or(false);
            PrinterAwareOptionSummary { summary, hidden }
        })
        .collect()
}

/// Per-extruder ("toolhead") Printer-bucket options — the set the printer
/// panel's per-extruder tabs surface. Same shape as
/// [`slicer_machine_options_for_printer`]; the frontend renders one tab
/// per toolhead, showing each option's value at that extruder's vector
/// index.
#[tauri::command]
#[tracing::instrument(skip(printer))]
pub fn slicer_extruder_options_for_printer(
    printer: PrinterProfile,
    filter: Option<String>,
) -> Vec<PrinterAwareOptionSummary> {
    printer_bucket_summaries(filter, is_extruder_visible)
        .into_iter()
        .map(|summary| {
            let hidden = summary
                .capability
                .map(|c| !c.satisfied_by(&printer))
                .unwrap_or(false);
            PrinterAwareOptionSummary { summary, hidden }
        })
        .collect()
}

/// Filament-settings-visible options: the **Filament** bucket, minus
/// readonly + SLA keys, keeping only keys Orca actually lays out an editor
/// for (`filament_page_of` is `Some`). Like the printer surfaces, filament
/// options carry no libslic3r `category` of their own — their grouping is
/// scraped from `TabFilament` — so this both filters (drops metadata like
/// `compatible_printers` / `filament_settings_id` with no editor) and is
/// the source of the page/optgroup grouping applied in
/// [`filament_option_summaries`].
///
/// Filament keys are stored per-filament *vectors* in libslic3r but the
/// editor edits a single filament, so the frontend renders each as its
/// scalar element (`scalarElementKind`). The override we persist is the
/// scalar value the composer zips into the vector at compose time.
fn is_filament_visible(d: &slic3r_ffi::OptionDef) -> bool {
    d.bucket == Some(slic3r_ffi::OptBucket::Filament)
        && !d.readonly
        && !d.scope.is_sla()
        && slic3r_ffi::filament_page_of(&d.key).is_some()
}

fn filament_option_summaries(filter: Option<String>) -> Vec<OptionSummary> {
    let needle = filter.unwrap_or_default().to_lowercase();
    let mut out: Vec<OptionSummary> = option_defs()
        .into_iter()
        .filter(is_filament_visible)
        .filter(|d| matches_filter(d, &needle))
        .map(summary_from_def)
        // Filament options carry no libslic3r category; use the scraped
        // TabFilament page as the nav category and the optgroup as the
        // sub-header (same shape the machine panel uses for printer keys).
        .map(|mut s| {
            s.category = slic3r_ffi::filament_page_of(&s.key).map(str::to_owned);
            s.group = slic3r_ffi::filament_subgroup_of(&s.key).map(str::to_owned);
            // Disambiguate keys whose libslic3r label is generic ("Other
            // layers" / "First layer") by prefixing the multi-option line
            // label Orca lays them out under (the plate type for bed temps,
            // "Nozzle" for print temps) — otherwise the bed-temperature
            // section is a wall of identical labels.
            if let Some(line) = slic3r_ffi::filament_line_of(&s.key) {
                let label = s.label.as_deref().unwrap_or("");
                if label != line {
                    s.label = Some(format!("{line} · {label}"));
                }
            }
            s
        })
        .collect();
    sort_by_display_order(&mut out, |s| &s.key);
    out
}

/// Filament-bucket option summaries for the filament settings editor.
/// Not printer-gated — a user filament isn't bound to a printer — so the
/// `PrinterAwareOptionSummary` shape carries `hidden = false` throughout
/// (keeps the frontend `categorize()` + section components shared with the
/// machine panel without a separate non-printer-aware path).
#[tauri::command]
pub fn slicer_filament_options(filter: Option<String>) -> Vec<PrinterAwareOptionSummary> {
    filament_option_summaries(filter)
        .into_iter()
        .map(|summary| PrinterAwareOptionSummary {
            summary,
            hidden: false,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::printer::profile::{BoundingBox, Toolhead};
    use slic3r_ffi::init as ffi_init;
    use std::sync::Once;

    static FFI: Once = Once::new();
    fn ensure_ffi() {
        FFI.call_once(|| {
            ffi_init(None, 3).expect("libslic3r init");
        });
    }

    /// Every option the settings panel surfaces must be FFF-settable
    /// (print / object / region scope). A Process-bucket option with no
    /// FFF scope is dangling — defined in the `ConfigDef` but in no config
    /// class, so nothing reads it and any override is inert. `is_panel_
    /// visible` filters these out; this guards that it keeps doing so (and
    /// that a real option like `layer_height` still shows).
    #[test]
    fn filament_panel_surfaces_filament_bucket_only() {
        ensure_ffi();
        let keys: Vec<String> = filament_option_summaries(None)
            .into_iter()
            .map(|s| s.key)
            .collect();
        // Real, editable filament settings are present...
        for expect in ["nozzle_temperature", "filament_flow_ratio", "fan_max_speed"] {
            assert!(
                keys.iter().any(|k| k == expect),
                "filament panel missing Filament-bucket `{expect}`",
            );
        }
        // ...Process- and Printer-bucket settings are not...
        for other in ["layer_height", "gcode_flavor", "nozzle_diameter"] {
            assert!(
                !keys.iter().any(|k| k == other),
                "filament panel must not surface non-filament `{other}`",
            );
        }
        // ...and every surfaced key really is Filament-bucket.
        for k in &keys {
            assert_eq!(
                slic3r_ffi::bucket_of(k),
                Some(slic3r_ffi::OptBucket::Filament),
                "`{k}` leaked into the filament panel but isn't Filament-bucket",
            );
        }
    }

    #[test]
    fn panel_surfaces_only_fff_settable_options() {
        ensure_ffi();
        let dangling: Vec<String> = option_defs()
            .into_iter()
            .filter(is_panel_visible)
            .filter(|d| !d.scope.is_fff())
            .map(|d| d.key)
            .collect();
        assert!(
            dangling.is_empty(),
            "panel surfaced non-FFF-settable (dangling) options whose overrides are inert: {dangling:?}",
        );
        let keys: Vec<String> = panel_option_summaries(None).into_iter().map(|s| s.key).collect();
        assert!(
            keys.iter().any(|k| k == "layer_height"),
            "a real FFF option must still be present",
        );
        assert!(
            !keys.iter().any(|k| k == "ironing_expansion"),
            "the dangling `ironing_expansion` must no longer appear",
        );
    }

    #[test]
    fn machine_panel_surfaces_printer_bucket_only() {
        ensure_ffi();
        let keys: Vec<String> = machine_option_summaries(None)
            .into_iter()
            .map(|s| s.key)
            .collect();
        // Scalars + the per-mode machine-limits vectors are present...
        for expect in [
            "gcode_flavor",
            "z_offset",
            "machine_start_gcode",
            "machine_max_acceleration_x",
        ] {
            assert!(
                keys.iter().any(|k| k == expect),
                "machine panel missing Printer-bucket `{expect}`",
            );
        }
        // ...Process-bucket settings are not...
        assert!(
            !keys.iter().any(|k| k == "layer_height"),
            "machine panel must not surface the Process-bucket `layer_height`",
        );
        // ...per-extruder vectors go to the extruder tabs, not here...
        for vector in ["retraction_length", "nozzle_diameter"] {
            assert!(
                !keys.iter().any(|k| k == vector),
                "machine panel must not surface per-extruder vector `{vector}`",
            );
        }
        // ...and capability flags / metadata / print-host keys are not
        // user-tunable settings, so they're hidden (no Orca editor).
        for hidden in [
            "silent_mode",
            "support_parallel_printheads",
            "printer_model",
            "printer_variant",
            "print_host",
            "printhost_apikey",
            "default_print_profile",
        ] {
            assert!(
                !keys.iter().any(|k| k == hidden),
                "machine panel must not surface non-setting `{hidden}`",
            );
        }
    }

    #[test]
    fn extruder_panel_surfaces_rendered_per_extruder_only() {
        ensure_ffi();
        let keys: Vec<String> = printer_bucket_summaries(None, is_extruder_visible)
            .into_iter()
            .map(|s| s.key)
            .collect();
        // Per-extruder keys with a live Orca editor are present...
        for expect in ["retraction_length", "z_hop", "nozzle_diameter"] {
            assert!(
                keys.iter().any(|k| k == expect),
                "extruder panel missing `{expect}`",
            );
        }
        // ...keys in libslic3r's per-extruder set but with no live editor
        // (commented out / metadata / internal) are not surfaced...
        for hidden in ["extruder_colour", "default_filament_profile", "nozzle_flush_dataset"] {
            assert!(
                !keys.iter().any(|k| k == hidden),
                "extruder panel must not surface non-rendered `{hidden}`",
            );
        }
        // ...and machine-wide keys don't leak in.
        assert!(!keys.iter().any(|k| k == "gcode_flavor"));
    }

    fn a1_mini() -> PrinterProfile {
        PrinterProfile {
            model: "Bambu Lab A1 mini".into(),
            ams_max: 1,
            supported_build_plates: vec!["Textured PEI".into()],
            toolheads: vec![Toolhead {
                default_nozzle_diameter: "0.4".to_string(),
                hotend_type: "stainless_steel".into(),
                max_temp: 300.0,
            }],
            build_volume: BoundingBox {
                min: [0.0, 0.0, 0.0],
                max: [180.0, 180.0, 180.0],
            },
            exclusion_zones: vec![],
            ..Default::default()
        }
    }

    fn synthetic_toolchanger() -> PrinterProfile {
        PrinterProfile {
            model: "Synthetic 2-toolhead".into(),
            supported_build_plates: vec!["Textured PEI".into()],
            toolheads: (0..2)
                .map(|_i| Toolhead {
                    default_nozzle_diameter: "0.4".to_string(),
                    hotend_type: "stainless_steel".into(),
                    max_temp: 300.0,
                })
                .collect(),
            build_volume: BoundingBox {
                min: [0.0, 0.0, 0.0],
                max: [200.0, 200.0, 200.0],
            },
            exclusion_zones: vec![],
            ..Default::default()
        }
    }

    #[test]
    fn engine_default_serialized_returns_capability_gated_key_default() {
        ensure_ffi();
        // wipe_tower_x is capability-gated (RequiresPurgeTower) so it can be
        // filtered out of `panel_option_summaries`; the exact-match accessor must
        // still surface its compiled default. The priming-tower overlay
        // leans on this for printers (the U1) that pin no position.
        let d = engine_default_serialized("wipe_tower_x")
            .expect("libslic3r ships a wipe_tower_x default");
        assert!(
            d.trim().parse::<f64>().is_ok(),
            "default parses as a number, got {d:?}"
        );
        assert_eq!(engine_default_serialized("not_a_real_key_xyz"), None);
    }

    #[test]
    fn slicer_options_carries_mode_scope_tooltip_capability() {
        ensure_ffi();
        let opts = panel_option_summaries(Some("layer_height".into()));
        // The schema is huge; layer_height is the canonical
        // representative the rest of the project keys off.
        let lh = opts
            .iter()
            .find(|o| o.key == "layer_height")
            .expect("layer_height present");
        assert_eq!(
            lh.mode,
            OptMode::Simple,
            "layer_height ships in Simple mode"
        );
        // layer_height lives in PrintObjectConfig (per
        // external/OrcaSlicer/src/libslic3r/PrintConfig.hpp:936).
        assert!(lh.scope.object, "layer_height is an object-scope option");
        assert!(
            lh.tooltip.is_some(),
            "libslic3r ships a tooltip for layer_height"
        );
        assert!(
            lh.capability.is_none(),
            "layer_height has no printer-capability gating",
        );

        // outer_wall_filament_id is the canonical region-scope option per the
        // same PrintConfig.hpp (renamed from wall_filament upstream).
        let wf_opts = panel_option_summaries(Some("outer_wall_filament_id".into()));
        let wf = wf_opts
            .iter()
            .find(|o| o.key == "outer_wall_filament_id")
            .expect("outer_wall_filament_id present");
        assert!(wf.scope.region, "outer_wall_filament_id is a region-scope option");
    }

    #[test]
    fn a1_mini_shows_purge_tower_keys_via_printer_aware_view() {
        ensure_ffi();
        // The panel filter narrows to the Process bucket — printer-
        // bucket toolchanger geometry isn't in scope here. The
        // remaining capability-gated process-bucket keys are the
        // purge-tower / prime-tower family, which AMS-style
        // printers DO use.
        let opts = slicer_options_for_printer(a1_mini(), None);
        let purge = opts
            .iter()
            .find(|o| o.summary.key == "enable_prime_tower")
            .expect("enable_prime_tower present");
        assert_eq!(
            purge.summary.capability,
            Some(CapabilityPredicate::RequiresPurgeTower),
        );
        assert!(
            !purge.hidden,
            "purge-tower key should be visible on AMS-style A1 mini",
        );
    }

    #[test]
    fn synthetic_toolchanger_hides_purge_tower_keys() {
        ensure_ffi();
        let opts = slicer_options_for_printer(synthetic_toolchanger(), None);
        let purge = opts
            .iter()
            .find(|o| o.summary.key == "enable_prime_tower")
            .expect("enable_prime_tower present");
        assert!(purge.hidden, "purge-tower key should hide on toolchanger");
    }

    #[test]
    fn default_value_for_scalar_keeps_serialized_string() {
        let dv = DefaultValue::from_serialized(OptType::Float, "0.2");
        assert_eq!(
            dv,
            DefaultValue::Scalar {
                value: "0.2".into()
            }
        );
    }

    #[test]
    fn default_value_for_simple_vector_comma_splits() {
        // coFloats / coInts / coPercents all use comma-joined
        // serialization. The wire form is exactly what libslic3r
        // emits via ConfigOptionFloats::serialize.
        let dv = DefaultValue::from_serialized(OptType::Floats, "220,220,220,220");
        assert_eq!(
            dv,
            DefaultValue::Vector {
                values: vec!["220".into(), "220".into(), "220".into(), "220".into()],
            },
        );
    }

    #[test]
    fn default_value_for_empty_vector_emits_empty_vec() {
        let dv = DefaultValue::from_serialized(OptType::Ints, "");
        assert_eq!(dv, DefaultValue::Vector { values: Vec::new() });
    }

    #[test]
    fn default_value_for_strings_round_trips_through_cstyle_unescape() {
        // small_area_infill_flow_compensation_model's compiled-in default
        // — the canonical regression case. Entries contain embedded
        // newlines + commas; comma-split would corrupt them.
        let serialized = "0,0;\"\\n0.2,0.4444\";\"\\n0.4,0.6145\";\"\\n0.6,0.7059\"";
        let dv = DefaultValue::from_serialized(OptType::Strings, serialized);
        let DefaultValue::Vector { values } = dv else {
            panic!("expected Vector for coStrings");
        };
        assert_eq!(values.len(), 4);
        assert_eq!(values[0], "0,0", "unquoted first entry");
        assert_eq!(
            values[1], "\n0.2,0.4444",
            "quoted entry round-trips with literal newline"
        );
        assert_eq!(values[2], "\n0.4,0.6145");
        assert_eq!(values[3], "\n0.6,0.7059");
    }

    #[test]
    fn default_value_for_strings_handles_unquoted_singleton() {
        // Single-entry default with no special chars — emitted bare by
        // escape_strings_cstyle. Should still split cleanly.
        let dv = DefaultValue::from_serialized(OptType::Strings, "PLA");
        assert_eq!(
            dv,
            DefaultValue::Vector {
                values: vec!["PLA".into()]
            },
        );
    }

    #[test]
    fn default_value_for_strings_handles_escaped_quotes_and_backslashes() {
        // Entry contains a backslash + a quote; both have to come back
        // through the cstyle unescape pass.
        let dv = DefaultValue::from_serialized(OptType::Strings, "\"a\\\\b\\\"c\"");
        let DefaultValue::Vector { values } = dv else {
            panic!("expected Vector");
        };
        assert_eq!(values, vec!["a\\b\"c"]);
    }

    #[test]
    fn enum_values_surface_on_option_summary() {
        // Without enum_values on the wire, DropdownInput renders an
        // empty <select> and can't be opened. Pin the surface using
        // seam_position — a Process-bucket Enum that the panel
        // routinely shows.
        ensure_ffi();
        let opts = panel_option_summaries(Some("seam_position".into()));
        let opt = opts
            .iter()
            .find(|o| o.key == "seam_position")
            .expect("seam_position in schema");
        assert!(
            !opt.enum_values.is_empty(),
            "enum option must surface enum_values",
        );
        let keys: Vec<&str> = opt.enum_values.iter().map(|(k, _)| k.as_str()).collect();
        // libslic3r ships at least these four. The full list can grow
        // upstream; we only pin the well-known ones.
        for expected in ["nearest", "aligned", "back", "random"] {
            assert!(
                keys.contains(&expected),
                "seam_position missing value {expected:?} (got {keys:?})",
            );
        }
    }

    #[test]
    fn small_area_default_surfaces_as_split_vector_on_live_schema() {
        // End-to-end: the live FFI default for the regression key lands
        // as a Vector of 10 entries, each carrying readable text.
        ensure_ffi();
        let opts = panel_option_summaries(Some("small_area_infill_flow_compensation_model".into()));
        let opt = opts
            .iter()
            .find(|o| o.key == "small_area_infill_flow_compensation_model")
            .expect("regression key present in libslic3r schema");
        let dv = opt.default_value.as_ref().expect("compile-time default");
        let DefaultValue::Vector { values } = dv else {
            panic!("coStrings must surface as Vector, got {dv:?}");
        };
        assert_eq!(
            values.len(),
            10,
            "10-entry default per upstream PrintConfig.cpp"
        );
        assert_eq!(values[0], "0,0", "first entry is the (0,0) anchor pair");
        assert!(opt.multiline, "regression field is a multiline textarea");
    }

    #[test]
    fn printer_aware_view_completes_within_render_budget() {
        ensure_ffi();
        // The panel surfaces Process-bucket options only (~345 keys
        // vs ~624 without bucket filtering). Per the FR-UI
        // 50 ms panel re-render budget, the full capability evaluation
        // pass must not dominate; debug allows 10× headroom.
        let start = std::time::Instant::now();
        let opts = slicer_options_for_printer(a1_mini(), None);
        let elapsed = start.elapsed();
        assert!(
            opts.len() >= 300,
            "expected ≥ 300 process-bucket options from libslic3r, got {}",
            opts.len(),
        );
        assert!(
            elapsed < std::time::Duration::from_millis(500),
            "slicer_options_for_printer took {elapsed:?} (debug budget 500 ms)",
        );
    }
}
