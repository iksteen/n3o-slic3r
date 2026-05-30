//! Typed G-code Lua binding.
//!
//! Exposes a plate's parsed `Vec<gcode::Line>` to a plugin as a
//! `Gcode` userdata, so a post-slice hook reads and edits a structured
//! sequence — `move` / `comment` / `layer_change` / `tool_change` /
//! `other` — instead of raw strings.
//!
//! The lines live behind an `Arc<Mutex<…>>` shared between the userdata
//! and the host: the host creates a [`GcodeHandle`], keeps its
//! [`GcodeHandle::cell`], passes the handle into Lua, and reads the
//! (possibly mutated) lines back from the cell after the hook — no need
//! to extract anything out of the Lua state.
//!
//! Line *views* are plain Lua tables materialized on access (`g:line(i)`
//! / `g:lines()`), which is friendlier for plugin authors than method-
//! per-field userdata and only allocates for lines actually touched.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use mlua::{Lua, MetaMethod, Result as LuaResult, Table, UserData, UserDataMethods, Value};

use crate::core::gcode::{parse_str, Line};
use crate::core::gcode::model::{
    Comment, CommentStyle, LayerSource, MoveCommand, Other, SemanticComment,
};

/// Shared, mutable line buffer behind the userdata.
pub type GcodeCell = Arc<Mutex<Vec<Line>>>;

/// Userdata handed to a plugin's hook. Cheap to clone-by-Arc; the host
/// holds [`GcodeHandle::cell`] to read edits back.
pub struct GcodeHandle {
    lines: GcodeCell,
}

impl GcodeHandle {
    pub fn new(lines: Vec<Line>) -> Self {
        Self {
            lines: Arc::new(Mutex::new(lines)),
        }
    }

    /// The shared cell, for the host to read lines back after a hook.
    pub fn cell(&self) -> GcodeCell {
        self.lines.clone()
    }
}

impl UserData for GcodeHandle {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        // #g and g:len()
        methods.add_meta_method(MetaMethod::Len, |_, this, ()| Ok(this.lines.lock().unwrap().len()));
        methods.add_method("len", |_, this, ()| Ok(this.lines.lock().unwrap().len()));

        // g:line(i) — 1-based; nil when out of range (reads are lenient).
        methods.add_method("line", |lua, this, i: usize| {
            let lines = this.lines.lock().unwrap();
            match i.checked_sub(1).filter(|&z| z < lines.len()) {
                Some(z) => Ok(Some(line_to_table(lua, &lines[z])?)),
                None => Ok(None),
            }
        });

        // g:lines() — stateful iterator: `for line in g:lines() do …`.
        methods.add_method("lines", |lua, this, ()| {
            let lines = this.lines.clone();
            let cursor = Arc::new(AtomicUsize::new(0));
            lua.create_function(move |lua, ()| {
                let guard = lines.lock().unwrap();
                let i = cursor.fetch_add(1, Ordering::Relaxed);
                if i < guard.len() {
                    Ok(Some(line_to_table(lua, &guard[i])?))
                } else {
                    Ok(None)
                }
            })
        });

        // g:layers() — iterator over layer spans segmented on LayerChange.
        methods.add_method("layers", |lua, this, ()| {
            let layers = Arc::new(compute_layers(&this.lines.lock().unwrap()));
            let cursor = Arc::new(AtomicUsize::new(0));
            lua.create_function(move |lua, ()| {
                let i = cursor.fetch_add(1, Ordering::Relaxed);
                match layers.get(i) {
                    Some(layer) => {
                        let t = lua.create_table()?;
                        t.set("index", layer.index)?;
                        set_opt_num(&t, "z", layer.z)?;
                        t.set("first_line", layer.first_line)?;
                        t.set("last_line", layer.last_line)?;
                        Ok(Some(t))
                    }
                    None => Ok(None),
                }
            })
        });

        // Mutations. A `line` arg is a raw G-code string (parsed, may
        // expand to several lines) or a constructed table.
        methods.add_method("append", |_, this, value: Value| {
            let new = value_to_lines(value)?;
            this.lines.lock().unwrap().extend(new);
            Ok(())
        });

        methods.add_method("insert", |_, this, (i, value): (usize, Value)| {
            let new = value_to_lines(value)?;
            let mut lines = this.lines.lock().unwrap();
            let idx = i.checked_sub(1).ok_or_else(|| index_err(i))?;
            if idx > lines.len() {
                return Err(index_err(i));
            }
            for (k, line) in new.into_iter().enumerate() {
                lines.insert(idx + k, line);
            }
            Ok(())
        });

        methods.add_method("replace", |_, this, (i, value): (usize, Value)| {
            let new = value_to_lines(value)?;
            let mut lines = this.lines.lock().unwrap();
            let idx = i
                .checked_sub(1)
                .filter(|&z| z < lines.len())
                .ok_or_else(|| index_err(i))?;
            lines.remove(idx);
            for (k, line) in new.into_iter().enumerate() {
                lines.insert(idx + k, line);
            }
            Ok(())
        });

        methods.add_method("remove", |_, this, i: usize| {
            let mut lines = this.lines.lock().unwrap();
            let idx = i
                .checked_sub(1)
                .filter(|&z| z < lines.len())
                .ok_or_else(|| index_err(i))?;
            lines.remove(idx);
            Ok(())
        });
    }
}

fn index_err(i: usize) -> mlua::Error {
    mlua::Error::RuntimeError(format!("gcode index {i} out of range"))
}

/// Build the read-only Lua table view of one line.
fn line_to_table(lua: &Lua, line: &Line) -> LuaResult<Table> {
    let t = lua.create_table()?;
    match line {
        Line::Move(m) => {
            t.set("kind", "move")?;
            set_opt_num(&t, "x", m.target.x)?;
            set_opt_num(&t, "y", m.target.y)?;
            set_opt_num(&t, "z", m.target.z)?;
            set_opt_num(&t, "e", m.target.e)?;
            t.set("f", m.feedrate)?; // Option<u32> -> integer or nil
            t.set(
                "command",
                match m.command {
                    MoveCommand::Rapid => "G0",
                    MoveCommand::Linear => "G1",
                    MoveCommand::ArcCw => "G2",
                    MoveCommand::ArcCcw => "G3",
                },
            )?;
            // A move is a travel when it carries no positive extrusion.
            let travel = m.target.e.map(|e| e <= 0.0).unwrap_or(true);
            t.set("travel", travel)?;
        }
        Line::Comment(c) => {
            t.set("kind", "comment")?;
            // `text` is the canonical raw comment (delimiter + leading
            // whitespace included); plugins string-match on it.
            t.set("text", c.raw.clone())?;
            t.set(
                "style",
                match c.style {
                    CommentStyle::Semicolon => "semicolon",
                    CommentStyle::Parens => "parens",
                },
            )?;
            if let Some(s) = &c.semantic {
                t.set("semantic", semantic_name(s))?;
            }
        }
        Line::LayerChange(l) => {
            t.set("kind", "layer_change")?;
            t.set("index", l.index)?;
            set_opt_num(&t, "z", l.z)?;
            t.set(
                "source",
                match l.source {
                    LayerSource::Marker => "marker",
                    LayerSource::Heuristic => "heuristic",
                },
            )?;
        }
        Line::ToolChange(tc) => {
            t.set("kind", "tool_change")?;
            t.set("tool", tc.extruder)?;
        }
        Line::Other(o) => {
            t.set("kind", "other")?;
            t.set("raw", o.raw.clone())?;
        }
    }
    Ok(t)
}

fn semantic_name(s: &SemanticComment) -> &'static str {
    match s {
        SemanticComment::FeatureType(_) => "feature_type",
        SemanticComment::Layer(_) => "layer",
        SemanticComment::Z(_) => "z",
        SemanticComment::EstimatedTime(_) => "estimated_time",
        SemanticComment::FilamentUsed(_) => "filament_used",
        SemanticComment::LayerCount(_) => "layer_count",
        SemanticComment::PrinterModel(_) => "printer_model",
        SemanticComment::ExtruderTemp(_) => "extruder_temp",
        SemanticComment::BedTemp(_) => "bed_temp",
    }
}

fn set_opt_num(t: &Table, key: &str, v: Option<f32>) -> LuaResult<()> {
    if let Some(x) = v {
        t.set(key, x as f64)?;
    }
    Ok(())
}

/// Convert a mutation argument (raw G-code string or constructed table)
/// into one or more typed lines, normalized to end in a newline so
/// inserted content can't merge with its neighbour.
fn value_to_lines(value: Value) -> LuaResult<Vec<Line>> {
    match value {
        Value::String(s) => {
            let mut lines = parse_str(&s.to_str()?);
            for line in &mut lines {
                ensure_newline(line);
            }
            Ok(lines)
        }
        Value::Table(t) => Ok(vec![table_to_line(&t)?]),
        other => Err(mlua::Error::RuntimeError(format!(
            "gcode line must be a string or a table, got {}",
            other.type_name()
        ))),
    }
}

fn table_to_line(t: &Table) -> LuaResult<Line> {
    let kind: String = t.get("kind")?;
    match kind.as_str() {
        "comment" => {
            let raw = match t.get::<Option<String>>("raw")? {
                Some(raw) => raw,
                None => {
                    let text: String = t.get("text").map_err(|_| {
                        mlua::Error::RuntimeError(
                            "comment line needs a `text` or `raw` field".to_string(),
                        )
                    })?;
                    format!("; {text}")
                }
            };
            let style = if raw.trim_start().starts_with('(') {
                CommentStyle::Parens
            } else {
                CommentStyle::Semicolon
            };
            Ok(Line::Comment(Comment {
                raw,
                style,
                semantic: None,
                raw_offset: 0,
                line_ending: "\n".to_string(),
            }))
        }
        "other" => {
            let raw: String = t.get("raw").map_err(|_| {
                mlua::Error::RuntimeError("other line needs a `raw` field".to_string())
            })?;
            Ok(Line::Other(Other {
                raw,
                raw_offset: 0,
                line_ending: "\n".to_string(),
            }))
        }
        other => Err(mlua::Error::RuntimeError(format!(
            "cannot build a `{other}` line from a table; pass move / tool / layer lines as a raw G-code string"
        ))),
    }
}

fn ensure_newline(line: &mut Line) {
    if !line.line_ending().is_empty() {
        return;
    }
    match line {
        Line::Move(m) => m.line_ending = "\n".into(),
        Line::Comment(c) => c.line_ending = "\n".into(),
        Line::LayerChange(l) => l.line_ending = "\n".into(),
        Line::ToolChange(t) => t.line_ending = "\n".into(),
        Line::Other(o) => o.line_ending = "\n".into(),
    }
}

struct LayerInfo {
    index: u32,
    z: Option<f32>,
    /// 1-based line number where the layer's `LayerChange` marker sits.
    first_line: usize,
    /// 1-based line number of the layer's last line (just before the
    /// next `LayerChange`, or the end of the file).
    last_line: usize,
}

fn compute_layers(lines: &[Line]) -> Vec<LayerInfo> {
    let starts: Vec<(usize, u32, Option<f32>)> = lines
        .iter()
        .enumerate()
        .filter_map(|(i, line)| match line {
            Line::LayerChange(lc) => Some((i, lc.index, lc.z)),
            _ => None,
        })
        .collect();
    starts
        .iter()
        .enumerate()
        .map(|(k, &(pos, index, z))| LayerInfo {
            index,
            z,
            first_line: pos + 1,
            last_line: starts.get(k + 1).map(|n| n.0).unwrap_or(lines.len()),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::gcode::to_string;
    use crate::core::plugin::PluginRuntime;

    /// Two layers, a few moves, a comment, a tool change.
    const SAMPLE: &str = "\
;LAYER:0
G1 X0 Y0 F1200
G1 X10 Y0 E0.5
;LAYER:1
T1
G1 X10 Y10 E1.0
";

    /// Run `lua` against a handle wrapping `SAMPLE`, return the
    /// re-serialized G-code after the plugin's `go()` ran.
    fn run(lua: &str) -> String {
        let rt = PluginRuntime::load(lua, "t").unwrap();
        let handle = GcodeHandle::new(parse_str(SAMPLE));
        let cell = handle.cell();
        let _: Option<()> = rt.call("go", handle).unwrap();
        let lines = cell.lock().unwrap().clone();
        to_string(&lines)
    }

    #[test]
    fn no_op_is_byte_identical() {
        let out = run("function go(g) end");
        assert_eq!(out, SAMPLE);
    }

    #[test]
    fn counts_lines_and_layers() {
        let rt = PluginRuntime::load(
            r#"function count(g)
                 local layers = 0
                 for _ in g:layers() do layers = layers + 1 end
                 return #g, layers
               end"#,
            "t",
        )
        .unwrap();
        let handle = GcodeHandle::new(parse_str(SAMPLE));
        let (len, layers): (usize, usize) = rt.call("count", handle).unwrap().unwrap();
        // `#g` is exactly what the parser produced; the two `;LAYER:`
        // markers give two layers regardless of internal line layout.
        assert_eq!(len, parse_str(SAMPLE).len());
        assert_eq!(layers, 2);
    }

    #[test]
    fn reads_move_fields() {
        // Find the extruding move (G1 X10 … E0.5) by scanning, so the
        // test doesn't depend on the parser's exact line indices.
        let rt = PluginRuntime::load(
            r#"function probe(g)
                 for line in g:lines() do
                   if line.kind == "move" and line.e ~= nil and line.e > 0 then
                     return line.command, line.x, line.e, line.travel
                   end
                 end
               end"#,
            "t",
        )
        .unwrap();
        let handle = GcodeHandle::new(parse_str(SAMPLE));
        let (command, x, e, travel): (String, f64, f64, bool) =
            rt.call("probe", handle).unwrap().unwrap();
        assert_eq!(command, "G1");
        assert_eq!(x, 10.0);
        assert_eq!(e, 0.5);
        assert!(!travel);
    }

    #[test]
    fn appends_a_comment_and_round_trips() {
        let out = run(r#"function go(g) g:append({ kind = "comment", text = "n3o was here" }) end"#);
        assert!(out.starts_with(SAMPLE));
        assert!(out.ends_with("; n3o was here\n"));
    }

    #[test]
    fn appends_a_raw_command_string() {
        let out = run(r#"function go(g) g:append("M300 S440 P200") end"#);
        assert!(out.contains("M300 S440 P200\n"));
        assert!(out.starts_with(SAMPLE));
    }

    #[test]
    fn inserts_a_pause_at_a_layer_boundary() {
        // `layer.first_line` is layer 1's `LayerChange` marker, which
        // the parser places right after the visible `;LAYER:1` comment;
        // inserting there lands the pause at the layer boundary.
        let out = run(r#"function go(g)
            for layer in g:layers() do
                if layer.index == 1 then g:insert(layer.first_line, "M601") end
            end
        end"#);
        let lines: Vec<&str> = out.lines().collect();
        let pause = lines.iter().position(|l| *l == "M601").unwrap();
        let marker = lines.iter().position(|l| *l == ";LAYER:1").unwrap();
        assert_eq!(pause, marker + 1, "pause should sit at the layer boundary");
    }

    #[test]
    fn remove_then_serialize_drops_the_line() {
        // Find + remove the tool-change line by scanning.
        let out = run(r#"function go(g)
            for i = 1, #g do
                if g:line(i).kind == "tool_change" then g:remove(i); break end
            end
        end"#);
        assert!(!out.contains("T1"));
    }

    #[test]
    fn out_of_range_mutation_errors() {
        let rt = PluginRuntime::load(r#"function go(g) g:remove(999) end"#, "t").unwrap();
        let handle = GcodeHandle::new(parse_str(SAMPLE));
        let r: Result<Option<()>, _> = rt.call("go", handle);
        assert!(r.is_err());
    }
}
