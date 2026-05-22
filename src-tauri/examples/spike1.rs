//! Spike PR-0.5-1: end-to-end cascade resolver + adapter + slice.
//!
//! Reads examples/cascades/bambu-a1-mini-spike1.toml, resolves it
//! against a hardcoded context (PLA on Textured PEI), expands the
//! logical `bed_temp` into libslic3r's per-plate-type vector keys,
//! pushes the result into a `slic3r_ffi::Config`, slices OrcaCube_v2.3mf,
//! and writes /tmp/spike1.gcode.
//!
//! Throwaway. Not production code. The Phase 1 resolver gets a
//! clean-room reimplementation informed by the findings doc this
//! example feeds into.
//!
//! Run from the workspace root:
//!   cargo run -p n3o-slic3r --release --example spike1

use serde::Deserialize;
use slic3r_ffi::{init, slice, Config, Error, ErrorKind, Model};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
struct Cascade {
    #[serde(rename = "rule")]
    rules: Vec<Rule>,
}

#[derive(Debug, Deserialize)]
struct Rule {
    #[serde(default = "empty_value")]
    when: toml::Value,
    set: BTreeMap<String, String>,
}

fn empty_value() -> toml::Value {
    toml::Value::Table(toml::map::Map::new())
}

/// Flat dotted-key context (e.g. "filament.type" -> "PLA").
type Context = BTreeMap<String, String>;

fn flatten_when(value: &toml::Value, prefix: &str, out: &mut Context) {
    match value {
        toml::Value::Table(map) => {
            for (k, v) in map {
                let nested = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                flatten_when(v, &nested, out);
            }
        }
        toml::Value::String(s) => {
            out.insert(prefix.to_string(), s.clone());
        }
        other => {
            // Spike scope: only string-equality predicates.
            panic!("predicate {prefix} has non-string value {other:?}");
        }
    }
}

/// Returns the rule's predicate count (= specificity). 0 means
/// unconditional (default rule).
fn rule_specificity(rule: &Rule) -> usize {
    let mut flat = Context::new();
    flatten_when(&rule.when, "", &mut flat);
    flat.len()
}

fn rule_matches(rule: &Rule, ctx: &Context) -> bool {
    let mut flat = Context::new();
    flatten_when(&rule.when, "", &mut flat);
    flat.iter().all(|(k, v)| ctx.get(k) == Some(v))
}

/// Two-pass over the cascade: lowest specificity first, ties broken by
/// source order (later rule wins). This implements the authored-cascade
/// half of the two-phase resolution in docs/profiles.md; the spike
/// doesn't model the `!important` override tiers.
fn resolve(cascade: &Cascade, ctx: &Context) -> Context {
    let mut matching: Vec<(usize, usize, &Rule)> = cascade
        .rules
        .iter()
        .enumerate()
        .filter_map(|(idx, r)| rule_matches(r, ctx).then_some((rule_specificity(r), idx, r)))
        .collect();
    matching.sort_by_key(|(spec, idx, _)| (*spec, *idx));
    let mut resolved = Context::new();
    for (_, _, rule) in &matching {
        for (k, v) in &rule.set {
            resolved.insert(k.clone(), v.clone());
        }
    }
    resolved
}

/// libslic3r's per-plate-type bed temperature keys. The cascade carries
/// a single logical `bed_temp` resolved against the active plate type;
/// the adapter writes the resolved value into every plate-temp key so
/// libslic3r's `curr_bed_type` selector picks the right one at slice
/// time. (See docs/profiles.md "Translating to libslic3r" → bed temp
/// dimensional expansion.)
const PLATE_TEMP_KEYS: &[&str] = &[
    "hot_plate_temp",
    "hot_plate_temp_initial_layer",
    "cool_plate_temp",
    "cool_plate_temp_initial_layer",
    "eng_plate_temp",
    "eng_plate_temp_initial_layer",
    "textured_plate_temp",
    "textured_plate_temp_initial_layer",
    "textured_cool_plate_temp",
    "textured_cool_plate_temp_initial_layer",
    "supertack_plate_temp",
    "supertack_plate_temp_initial_layer",
    "smooth_plate_temp",
    "smooth_plate_temp_initial_layer",
];

/// Push the resolved logical config into a `slic3r_ffi::Config`, applying
/// the dimensional expansion for `bed_temp` and silently skipping keys
/// libslic3r doesn't recognize (those are OrcaSlicer-specific metadata
/// that bled through the converter). Returns (config, list of skipped
/// (key, value, error) triples for the finding doc).
fn adapt(resolved: &Context) -> (Config, Vec<(String, String, String)>) {
    let mut config = Config::new().expect("config alloc");
    let mut skipped: Vec<(String, String, String)> = Vec::new();

    let bed_temp = resolved.get("bed_temp").cloned();

    for (key, value) in resolved {
        if key == "bed_temp" {
            // Logical key only — expanded below.
            continue;
        }
        if let Err(e) = config.set(key, value) {
            // UnknownKey: OrcaSlicer-specific metadata that doesn't map
            // 1:1 to libslic3r. ParseValue: the value's not in the form
            // libslic3r expects. Both are surfaced for the finding doc.
            skipped.push((key.clone(), value.clone(), format!("{e}")));
        }
    }

    if let Some(temp) = bed_temp {
        for plate_key in PLATE_TEMP_KEYS {
            if let Err(e) = config.set(plate_key, &temp) {
                skipped.push((
                    (*plate_key).into(),
                    temp.clone(),
                    format!("expanded from bed_temp: {e}"),
                ));
            }
        }
    }

    (config, skipped)
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Match log level 3 (warning) — same as the Tauri app's default.
    init(None, 3).map_err(|e| format!("libslic3r init: {e}"))?;

    let root = workspace_root();
    let cascade_path = root.join("examples/cascades/bambu-a1-mini-spike1.toml");
    let model_path = root.join("external/OrcaSlicer/resources/handy_models/OrcaCube_v2.3mf");
    let out_path = PathBuf::from("/tmp/spike1.gcode");

    eprintln!("cascade: {}", cascade_path.display());
    eprintln!("model:   {}", model_path.display());
    eprintln!("out:     {}\n", out_path.display());

    let cascade_src = std::fs::read_to_string(&cascade_path)?;
    let cascade: Cascade = toml::from_str(&cascade_src)?;
    eprintln!("rules in cascade: {}", cascade.rules.len());

    let ctx: Context = [("filament.type", "PLA"), ("plate.type", "Textured PEI")]
        .into_iter()
        .map(|(k, v)| (k.into(), v.into()))
        .collect();
    eprintln!("context: {ctx:?}");

    let resolved = resolve(&cascade, &ctx);
    eprintln!("resolved {} keys after cascade", resolved.len());
    eprintln!("  bed_temp resolved to: {:?}", resolved.get("bed_temp"));
    eprintln!(
        "  curr_bed_type resolved to: {:?}",
        resolved.get("curr_bed_type")
    );
    eprintln!(
        "  nozzle_temperature resolved to: {:?}",
        resolved.get("nozzle_temperature")
    );

    let (config, skipped) = adapt(&resolved);
    eprintln!(
        "\nadapter: pushed {} keys into Config, skipped {}",
        resolved.len() + PLATE_TEMP_KEYS.len() - skipped.len(),
        skipped.len()
    );

    let mut unknown: Vec<&str> = Vec::new();
    let mut parse_err: Vec<&str> = Vec::new();
    let mut other: Vec<(&str, &str)> = Vec::new();
    for (k, _v, err) in &skipped {
        if err.contains("UnknownKey") {
            unknown.push(k);
        } else if err.contains("ParseValue") {
            parse_err.push(k);
        } else {
            other.push((k, err));
        }
    }
    unknown.sort();
    parse_err.sort();
    eprintln!(
        "  UnknownKey: {}, ParseValue: {}, other: {}",
        unknown.len(),
        parse_err.len(),
        other.len()
    );
    if std::env::var_os("SPIKE_DUMP_GAPS").is_some() {
        eprintln!("\n  UnknownKey list (Orca keys not in libslic3r):");
        for k in &unknown {
            eprintln!("    {k}");
        }
        if !parse_err.is_empty() {
            eprintln!("\n  ParseValue list (Orca values libslic3r can't parse):");
            for k in &parse_err {
                eprintln!("    {k}");
            }
        }
        if !other.is_empty() {
            eprintln!("\n  Other errors:");
            for (k, err) in &other {
                eprintln!("    {k} — {err}");
            }
        }
    } else {
        eprintln!("  (set SPIKE_DUMP_GAPS=1 to list each skipped key)");
    }
    eprintln!();

    let mut model = Model::new()?;
    let mut model_config = Config::new()?;
    model.load_with_config(&model_path, &mut model_config)?;
    eprintln!("loaded model: {}", model_path.display());

    // The 3MF carries its own embedded config — overlay our cascade
    // result on top so the spike actually exercises our adapter rather
    // than just slicing with the 3MF's own settings.
    overlay(&config, &mut model_config, &resolved)?;

    eprintln!("\nslicing...");
    match slice(&model, &model_config, &out_path) {
        Ok(()) => {
            let bytes = std::fs::metadata(&out_path)?.len();
            eprintln!("ok — wrote {} ({} bytes)", out_path.display(), bytes);
            Ok(())
        }
        Err(e) => {
            eprintln!("slice failed: {e}");
            Err(e.into())
        }
    }
}

/// Re-set every key from our resolved config on top of the model's
/// embedded config. Cheap workaround for not having a Config::merge in
/// the FFI surface today — the finding doc captures this as a gap.
fn overlay(
    _from: &Config,
    into: &mut Config,
    resolved: &Context,
) -> Result<(), Error> {
    for (k, v) in resolved {
        if k == "bed_temp" {
            continue;
        }
        match into.set(k, v) {
            Ok(()) => {}
            Err(e) if e.kind == ErrorKind::UnknownKey => {}
            Err(e) if e.kind == ErrorKind::ParseValue => {}
            Err(e) => return Err(e),
        }
    }
    if let Some(t) = resolved.get("bed_temp") {
        for plate_key in PLATE_TEMP_KEYS {
            let _ = into.set(plate_key, t);
        }
    }
    Ok(())
}

#[allow(dead_code)]
fn _silence_unused_path_warning(_: &Path) {}
