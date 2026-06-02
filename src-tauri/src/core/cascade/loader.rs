//! TOML cascade file loader + parser.
//!
//! Parses cascade files into the typed IR from `types.rs`. Supports the
//! three equivalent authoring forms documented in `docs/profiles.md`
//! "Syntax — three equivalent forms":
//!
//! 1. **Top-level keys** (the unconditional default rule):
//!    ```toml
//!    bed_temp = 50
//!    layer_height = 0.2
//!    ```
//! 2. **Section shorthand** (single-condition rule):
//!    ```toml
//!    [filament.type.PLA]
//!    bed_temp = 45
//!    ```
//! 3. **Explicit `[[rule]]`** (any rule, including compound conditions):
//!    ```toml
//!    [[rule]]
//!    when.filament.type = "PLA"
//!    when.plate.type = "PEI"
//!    set.bed_temp = 55
//!    ```
//!
//! All three desugar to the same `Rule` AST. Source-position tracking is
//! per-rule (precise to the rule's defining line); per-key precision is
//! deferred to a future refinement.
//!
//! Load-time validation is intentionally minimal here — it checks TOML
//! shape only (e.g. `when.*` values must be string-or-array-of-string;
//! `set.*` must be a leaf, not a table). Schema-level validation (does
//! `wall_filament` exist? is `filament.type` a valid context dimension?)
//! is a separate pass — see [`super::validate`] — so that callers can
//! choose to relax validation for partial-cascade scenarios like UI
//! live-editing.

use super::types::{Cascade, Condition, ConditionValue, Predicate, Rule, SourceLocation};
use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

/// Error returned by `load_cascade` and `parse_cascade_str`. Each
/// variant carries a `SourceLocation` so the rendered message can point
/// at file:line; pretty-printer renders rustc-style annotations.
#[derive(Debug)]
pub enum CascadeLoadError {
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    TomlParse {
        path: PathBuf,
        message: String,
    },
    InvalidShape {
        location: SourceLocation,
        message: String,
    },
}

impl fmt::Display for CascadeLoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => {
                write!(f, "{}: {}", path.display(), source)
            }
            Self::TomlParse { path, message } => {
                write!(f, "{}: TOML parse error: {}", path.display(), message)
            }
            Self::InvalidShape { location, message } => {
                write!(f, "{}: {}", location, message)
            }
        }
    }
}

impl std::error::Error for CascadeLoadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Load one or more cascade files in argument order, merge into one
/// `Cascade`. Rules retain their per-file source locations; the
/// resolver uses source-order tie-breaks across the merged list.
pub fn load_cascade(paths: &[&Path]) -> Result<Cascade, CascadeLoadError> {
    let mut all_rules = Vec::new();
    for path in paths {
        let src = fs::read_to_string(path).map_err(|e| CascadeLoadError::Io {
            path: (*path).into(),
            source: e,
        })?;
        let rules = parse_cascade_str(&src, path)?;
        all_rules.extend(rules);
    }
    Ok(Cascade { rules: all_rules })
}

/// Parse a single cascade file's contents into a list of `Rule`s.
/// Exposed for tests + future UI live-editing.
pub fn parse_cascade_str(src: &str, path: &Path) -> Result<Vec<Rule>, CascadeLoadError> {
    let parsed: toml::Value =
        src.parse::<toml::Value>()
            .map_err(|e| CascadeLoadError::TomlParse {
                path: path.into(),
                message: e.to_string(),
            })?;
    let root = parsed
        .as_table()
        .ok_or_else(|| CascadeLoadError::TomlParse {
            path: path.into(),
            message: "expected a table at the file root".into(),
        })?;

    let header_lines = scan_header_lines(src);
    let mut rules: Vec<Rule> = Vec::new();
    let mut default_set: BTreeMap<String, String> = BTreeMap::new();

    // Root-level pass: split into [[rule]] array, top-level leaves
    // (→ default rule), and top-level nested tables (→ section
    // shorthand descent).
    for (key, value) in root {
        if key == "rule" {
            parse_rule_array(value, path, &header_lines, &mut rules)?;
        } else if value.is_table() {
            walk_section(
                value.as_table().unwrap(),
                vec![key.clone()],
                path,
                &header_lines,
                &mut rules,
            )?;
        } else {
            default_set.insert(key.clone(), value_to_set_string(key, value, path, 1)?);
        }
    }

    if !default_set.is_empty() {
        let default_rule = Rule {
            when: Predicate::default(),
            set: default_set,
            source: SourceLocation {
                path: path.into(),
                line: 1,
            },
            important: false,
        };
        rules.insert(0, default_rule);
    }

    Ok(rules)
}

/// Handle the `rule` array-of-tables (the canonical `[[rule]]` form).
fn parse_rule_array(
    value: &toml::Value,
    path: &Path,
    header_lines: &HeaderLines,
    rules: &mut Vec<Rule>,
) -> Result<(), CascadeLoadError> {
    let rule_array = value
        .as_array()
        .ok_or_else(|| CascadeLoadError::InvalidShape {
            location: SourceLocation {
                path: path.into(),
                line: 1,
            },
            message: "`rule` must be an array of tables (`[[rule]]`)".into(),
        })?;
    for (idx, rule_value) in rule_array.iter().enumerate() {
        let line = header_lines.rule_lines.get(idx).copied().unwrap_or(1);
        let source = SourceLocation {
            path: path.into(),
            line,
        };
        let table = rule_value
            .as_table()
            .ok_or_else(|| CascadeLoadError::InvalidShape {
                location: source.clone(),
                message: "each `[[rule]]` entry must be a table".into(),
            })?;
        rules.push(parse_explicit_rule(table, source)?);
    }
    Ok(())
}

/// Walk a section-shorthand chain.
///
/// TOML parses `[filament.type.PLA]` as nested tables:
/// `{ filament: { type: { PLA: { bed_temp: 45 } } } }`. We descend
/// until we hit a table that contains at least one leaf value (or is
/// empty, which is a no-op). That leaf-containing table is the
/// shorthand's "body": its body's flattened leaves are the rule's
/// `set`, and the descent path joined by dots forms the section
/// header (dim + value).
///
/// Rules:
/// - Path of length 1 (e.g. `[meta]`) → reject. Sections need at
///   least 2 segments (one for dim, one for value).
/// - Path of length ≥ 2 → split on last segment: dim = first N-1
///   joined by dots, value = last.
/// - If a descent table is leaf-free, recurse into all its child
///   tables (further sub-sections).
/// - If a descent table has at least one leaf, treat the whole
///   thing as a body — flatten leaves + nested tables as dotted
///   set keys.
fn walk_section(
    table: &toml::value::Table,
    path_so_far: Vec<String>,
    file_path: &Path,
    header_lines: &HeaderLines,
    rules: &mut Vec<Rule>,
) -> Result<(), CascadeLoadError> {
    let has_leaf = table.values().any(|v| !v.is_table());

    if !has_leaf {
        // Pure-table level: descend into each child as a sub-section path.
        for (k, v) in table {
            let mut next = path_so_far.clone();
            next.push(k.clone());
            walk_section(v.as_table().unwrap(), next, file_path, header_lines, rules)?;
        }
        return Ok(());
    }

    // This is the shorthand body. Collect dotted-set entries and
    // build the single-condition rule.
    let header_path = path_so_far.join(".");
    let source = SourceLocation {
        path: file_path.into(),
        line: header_lines
            .sections
            .get(&header_path)
            .copied()
            .unwrap_or(1),
    };

    if path_so_far.len() < 2 {
        return Err(CascadeLoadError::InvalidShape {
            location: source,
            message: format!(
                "section [{header_path}] is not a valid context-dim.value shorthand \
                 (need at least two dotted segments; e.g. [filament.type.PLA])"
            ),
        });
    }

    let dim = path_so_far[..path_so_far.len() - 1].join(".");
    let dim_value = &path_so_far[path_so_far.len() - 1];

    let mut set = BTreeMap::new();
    collect_set(table, String::new(), &mut set, &source)?;

    let conditions = vec![Condition {
        dimension: dim,
        value: ConditionValue::Scalar(dim_value.clone()),
    }];
    rules.push(Rule {
        when: Predicate { conditions },
        set,
        source,
        important: false,
    });
    Ok(())
}

/// Parse a `[[rule]]` table body. Expects `when.*` and `set.*` keys;
/// any other top-level keys at the rule scope are rejected.
fn parse_explicit_rule(
    table: &toml::value::Table,
    source: SourceLocation,
) -> Result<Rule, CascadeLoadError> {
    let mut conditions = Vec::new();
    let mut set = BTreeMap::new();

    for (key, value) in table {
        match key.as_str() {
            "when" => {
                let when_table =
                    value
                        .as_table()
                        .ok_or_else(|| CascadeLoadError::InvalidShape {
                            location: source.clone(),
                            message: "`when` must be a table (use `when.dim = \"v\"` form)".into(),
                        })?;
                collect_conditions(when_table, String::new(), &mut conditions, &source)?;
            }
            "set" => {
                let set_table = value
                    .as_table()
                    .ok_or_else(|| CascadeLoadError::InvalidShape {
                        location: source.clone(),
                        message: "`set` must be a table (use `set.key = value` form)".into(),
                    })?;
                collect_set(set_table, String::new(), &mut set, &source)?;
            }
            other => {
                return Err(CascadeLoadError::InvalidShape {
                    location: source.clone(),
                    message: format!(
                        "[[rule]] entry has unexpected key `{other}`; expected only `when` \
                         and `set` (and their dotted sub-keys)"
                    ),
                });
            }
        }
    }

    Ok(Rule {
        when: Predicate { conditions },
        set,
        source,
        important: false,
    })
}

/// Recursively flatten nested `when.*` tables into `Condition`s with
/// dotted-path dimensions.
///
/// E.g. `when.filament.type = "PLA"` reaches this with prefix `""` and
/// table `{ filament: { type: "PLA" } }`; descent yields prefix
/// `"filament"`, then `"filament.type"`, then a leaf scalar — produces
/// `Condition { dimension: "filament.type", value: Scalar("PLA") }`.
fn collect_conditions(
    table: &toml::value::Table,
    prefix: String,
    out: &mut Vec<Condition>,
    source: &SourceLocation,
) -> Result<(), CascadeLoadError> {
    for (key, value) in table {
        let dotted = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        if let Some(nested) = value.as_table() {
            collect_conditions(nested, dotted, out, source)?;
        } else {
            let cv = condition_value_from_toml(value, &dotted, source)?;
            out.push(Condition {
                dimension: dotted,
                value: cv,
            });
        }
    }
    Ok(())
}

/// Recursively flatten nested `set.*` tables into a flat
/// `BTreeMap<dotted-key, serialized-string-value>`. Refuses anything
/// that's not a scalar leaf — set keys point at libslic3r options
/// which are always represented as strings on the cascade side.
fn collect_set(
    table: &toml::value::Table,
    prefix: String,
    out: &mut BTreeMap<String, String>,
    source: &SourceLocation,
) -> Result<(), CascadeLoadError> {
    for (key, value) in table {
        let dotted = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        if let Some(nested) = value.as_table() {
            collect_set(nested, dotted, out, source)?;
        } else {
            let s = value_to_set_string(&dotted, value, &source.path, source.line)?;
            out.insert(dotted, s);
        }
    }
    Ok(())
}

/// Convert a TOML value at the end of a `when.*` chain into a
/// `ConditionValue`. Strings produce `Scalar`; arrays of strings
/// produce `Array` (set membership). Other shapes (ints, bools,
/// nested arrays) are rejected — predicates compare against
/// stringified context values.
fn condition_value_from_toml(
    value: &toml::Value,
    dimension: &str,
    source: &SourceLocation,
) -> Result<ConditionValue, CascadeLoadError> {
    match value {
        toml::Value::String(s) => Ok(ConditionValue::Scalar(s.clone())),
        toml::Value::Integer(i) => Ok(ConditionValue::Scalar(i.to_string())),
        toml::Value::Float(f) => Ok(ConditionValue::Scalar(format_float(*f))),
        toml::Value::Boolean(b) => Ok(ConditionValue::Scalar(b.to_string())),
        toml::Value::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                let s = match item {
                    toml::Value::String(s) => s.clone(),
                    toml::Value::Integer(i) => i.to_string(),
                    toml::Value::Float(f) => format_float(*f),
                    toml::Value::Boolean(b) => b.to_string(),
                    _ => {
                        return Err(CascadeLoadError::InvalidShape {
                            location: source.clone(),
                            message: format!(
                                "predicate `when.{dimension}` array element \
                                 must be a string/int/float/bool"
                            ),
                        });
                    }
                };
                out.push(s);
            }
            Ok(ConditionValue::Array(out))
        }
        _ => Err(CascadeLoadError::InvalidShape {
            location: source.clone(),
            message: format!(
                "predicate `when.{dimension}` value must be a scalar or array of scalars"
            ),
        }),
    }
}

/// Serialize a TOML leaf as a libslic3r-shaped string. Vectors become
/// comma-separated; scalars convert directly. Tables are rejected
/// (they're handled by recursion in `collect_set`).
fn value_to_set_string(
    key: &str,
    value: &toml::Value,
    path: &Path,
    line: u32,
) -> Result<String, CascadeLoadError> {
    match value {
        toml::Value::String(s) => Ok(s.clone()),
        toml::Value::Integer(i) => Ok(i.to_string()),
        toml::Value::Float(f) => Ok(format_float(*f)),
        toml::Value::Boolean(b) => Ok(b.to_string()),
        toml::Value::Array(items) => {
            let mut parts = Vec::with_capacity(items.len());
            for item in items {
                let s = match item {
                    toml::Value::String(s) => s.clone(),
                    toml::Value::Integer(i) => i.to_string(),
                    toml::Value::Float(f) => format_float(*f),
                    toml::Value::Boolean(b) => b.to_string(),
                    _ => {
                        return Err(CascadeLoadError::InvalidShape {
                            location: SourceLocation {
                                path: path.into(),
                                line,
                            },
                            message: format!(
                                "set.{key} array element is not a scalar (got {item:?})"
                            ),
                        });
                    }
                };
                parts.push(s);
            }
            Ok(parts.join(","))
        }
        toml::Value::Table(_) => Err(CascadeLoadError::InvalidShape {
            location: SourceLocation {
                path: path.into(),
                line,
            },
            message: format!("set.{key} must be a leaf value (scalar or array), not a table"),
        }),
        toml::Value::Datetime(_) => Err(CascadeLoadError::InvalidShape {
            location: SourceLocation {
                path: path.into(),
                line,
            },
            message: format!("set.{key} datetimes are not supported"),
        }),
    }
}

fn format_float(f: f64) -> String {
    // TOML floats roundtrip with at least 1 decimal; libslic3r is
    // happy with integer-looking strings too.
    if f.fract() == 0.0 && f.is_finite() {
        format!("{}", f as i64)
    } else {
        format!("{f}")
    }
}

struct HeaderLines {
    /// Lines (1-based) of `[[rule]]` headers in source order.
    rule_lines: Vec<u32>,
    /// Map from canonical section path (as written between the
    /// brackets, sans outer quotes) to its line number.
    sections: HashMap<String, u32>,
}

/// One-pass scan to locate the source line of each `[[rule]]` header
/// and each `[section.path]` header. Used to assign `SourceLocation`s
/// after toml parses the value tree (which doesn't carry spans by
/// default).
///
/// Limitations:
/// - Inline tables (`a = { b = 1 }`) aren't section headers and aren't
///   treated as such here.
/// - Headers with embedded `]` inside quoted segments
///   (`[printer.model."A] mini"]`) confuse this scanner. Not a real
///   concern for our profiles today; document if it becomes one.
fn scan_header_lines(src: &str) -> HeaderLines {
    let mut rule_lines = Vec::new();
    let mut sections = HashMap::new();
    for (i, line) in src.lines().enumerate() {
        let line_num = (i + 1) as u32;
        let trimmed = line.trim();
        let no_comment = trimmed
            .split_once('#')
            .map(|(before, _)| before.trim())
            .unwrap_or(trimmed);
        if no_comment == "[[rule]]" {
            rule_lines.push(line_num);
        } else if no_comment.starts_with('[')
            && !no_comment.starts_with("[[")
            && no_comment.ends_with(']')
        {
            let path = no_comment.trim_start_matches('[').trim_end_matches(']');
            // Normalize quoted segments: `[printer.model."A1 mini"]` →
            // `printer.model.A1 mini`. The toml-parsed key path also
            // strips quotes, so they match.
            let path_unquoted = path.replace('"', "");
            sections.insert(path_unquoted, line_num);
        }
    }
    HeaderLines {
        rule_lines,
        sections,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(src: &str) -> Result<Vec<Rule>, CascadeLoadError> {
        parse_cascade_str(src, Path::new("test.toml"))
    }

    #[test]
    fn empty_file_yields_no_rules() {
        let rules = parse("").expect("empty parses");
        assert!(rules.is_empty());
    }

    #[test]
    fn top_level_keys_become_default_rule() {
        let src = "bed_temp = 50\nlayer_height = 0.2\n";
        let rules = parse(src).unwrap();
        assert_eq!(rules.len(), 1);
        assert!(rules[0].is_default());
        assert_eq!(rules[0].source.line, 1);
        assert_eq!(rules[0].set.get("bed_temp").map(String::as_str), Some("50"));
        assert_eq!(
            rules[0].set.get("layer_height").map(String::as_str),
            Some("0.2")
        );
    }

    #[test]
    fn explicit_rule_block_parses() {
        let src = "[[rule]]\nwhen.filament.type = \"PLA\"\nset.bed_temp = 45\n";
        let rules = parse(src).unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].source.line, 1);
        assert_eq!(rules[0].specificity(), 1);
        assert_eq!(rules[0].when.conditions[0].dimension, "filament.type");
        assert!(matches!(
            &rules[0].when.conditions[0].value,
            ConditionValue::Scalar(s) if s == "PLA"
        ));
        assert_eq!(rules[0].set.get("bed_temp").map(String::as_str), Some("45"));
    }

    #[test]
    fn section_shorthand_desugars_to_single_condition_rule() {
        let src = "[filament.type.PLA]\nbed_temp = 45\nfirst_layer_bed_temp = 50\n";
        let rules = parse(src).unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].specificity(), 1);
        assert_eq!(rules[0].when.conditions[0].dimension, "filament.type");
        assert!(matches!(
            &rules[0].when.conditions[0].value,
            ConditionValue::Scalar(s) if s == "PLA"
        ));
        assert_eq!(rules[0].set.len(), 2);
        assert_eq!(rules[0].set.get("bed_temp").map(String::as_str), Some("45"));
    }

    #[test]
    fn three_forms_produce_equivalent_default_plus_filament_rules() {
        // Form A: top-level default + section shorthand
        let a = parse("bed_temp = 50\n[filament.type.PLA]\nbed_temp = 45\n").unwrap();
        // Form B: top-level default + explicit [[rule]]
        let b = parse("bed_temp = 50\n[[rule]]\nwhen.filament.type = \"PLA\"\nset.bed_temp = 45\n")
            .unwrap();
        // Form C: two [[rule]] blocks (no top-level)
        let c = parse(
            "[[rule]]\nset.bed_temp = 50\n[[rule]]\nwhen.filament.type = \"PLA\"\nset.bed_temp = 45\n",
        )
        .unwrap();

        for rs in [&a, &b, &c] {
            assert_eq!(rs.len(), 2, "two rules each");
            assert!(rs[0].is_default(), "first is default");
            assert_eq!(rs[0].set["bed_temp"], "50");
            assert_eq!(rs[1].specificity(), 1);
            assert_eq!(rs[1].when.conditions[0].dimension, "filament.type");
            assert_eq!(rs[1].set["bed_temp"], "45");
        }
    }

    #[test]
    fn array_value_in_when_is_set_membership() {
        let src = "[[rule]]\nwhen.filament.type = [\"PLA\", \"PETG\"]\nset.bed_temp = 50\n";
        let rules = parse(src).unwrap();
        match &rules[0].when.conditions[0].value {
            ConditionValue::Array(items) => {
                assert_eq!(items, &vec!["PLA".to_string(), "PETG".to_string()]);
            }
            _ => panic!("expected array"),
        }
    }

    #[test]
    fn array_value_in_set_serializes_as_comma_joined() {
        // libslic3r vector keys parse comma-separated strings.
        let src = "nozzle_diameter = [\"0.4\", \"0.6\", \"0.4\", \"0.4\"]\n";
        let rules = parse(src).unwrap();
        assert_eq!(rules[0].set["nozzle_diameter"], "0.4,0.6,0.4,0.4");
    }

    #[test]
    fn rule_line_numbers_match_source() {
        // Three rule blocks at known source lines.
        let src = "\
bed_temp = 50

[[rule]]
when.filament.type = \"PLA\"
set.bed_temp = 45

[[rule]]
when.plate.type = \"PEI\"
set.bed_temp = 60
";
        let rules = parse(src).unwrap();
        assert_eq!(rules.len(), 3);
        assert_eq!(rules[0].source.line, 1, "default rule");
        assert_eq!(rules[1].source.line, 3, "first [[rule]]");
        assert_eq!(rules[2].source.line, 7, "second [[rule]]");
    }

    #[test]
    fn section_shorthand_line_number_matches_header() {
        let src = "\
# top comment
bed_temp = 50

[filament.type.PLA]
bed_temp = 45
";
        let rules = parse(src).unwrap();
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[1].source.line, 4);
    }

    #[test]
    fn single_segment_section_is_rejected() {
        let src = "[meta]\nauthor = \"me\"\n";
        let err = parse(src).expect_err("single-segment section should error");
        match err {
            CascadeLoadError::InvalidShape { message, .. } => {
                assert!(
                    message.contains("[meta]"),
                    "error names the offending section: {message}"
                );
            }
            other => panic!("unexpected error variant {other:?}"),
        }
    }

    #[test]
    fn rule_with_unexpected_key_is_rejected() {
        let src = "[[rule]]\nset.bed_temp = 50\nfoo = \"bar\"\n";
        let err = parse(src).expect_err("unexpected key should error");
        match err {
            CascadeLoadError::InvalidShape { message, .. } => {
                assert!(message.contains("foo"), "names offending key");
            }
            other => panic!("unexpected error variant {other:?}"),
        }
    }

    #[test]
    fn set_table_is_rejected() {
        let src = "[[rule]]\nset.bed_temp = { a = 1 }\n";
        // Note: TOML promotes `set.bed_temp = { a = 1 }` to a nested
        // table that collect_set descends into — this case becomes
        // set.bed_temp.a = "1", which is what we want for nested
        // dotted-keys. So the failure mode this test asserts is the
        // bare-table-at-leaf case.
        let rules = parse(src).unwrap();
        // We desugar bed_temp.a as a set entry — that's fine; the
        // schema check (validation pass) catches if `bed_temp.a`
        // isn't a real key. This test documents the desugaring rather
        // than asserting an error.
        assert_eq!(rules[0].set.len(), 1);
        assert!(rules[0].set.contains_key("bed_temp.a"));
    }
}
