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

/// Lock the shared line buffer, recovering the guard if the mutex was
/// poisoned by a panic mid-edit. The buffer is only ever touched from
/// one plugin call at a time, so recovering is safe — and it keeps a
/// stray panic from wedging every later access.
fn lock_lines(cell: &GcodeCell) -> std::sync::MutexGuard<'_, Vec<Line>> {
    cell.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl UserData for GcodeHandle {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        // #g and g:len()
        methods.add_meta_method(MetaMethod::Len, |_, this, ()| Ok(lock_lines(&this.lines).len()));
        methods.add_method("len", |_, this, ()| Ok(lock_lines(&this.lines).len()));

        // g:line(i) — 1-based; nil when out of range (reads are lenient).
        methods.add_method("line", |lua, this, i: usize| {
            let lines = lock_lines(&this.lines);
            match i.checked_sub(1).filter(|&z| z < lines.len()) {
                Some(z) => Ok(Some(line_to_table(lua, &lines[z])?)),
                None => Ok(None),
            }
        });

        // g:lines() — stateful iterator: `for line in g:lines() do …`.
        // Indexes the live buffer by cursor; treat it as a read
        // iterator (mutating the buffer mid-iteration shifts indices —
        // use g:layers() for mutate-while-iterating).
        methods.add_method("lines", |lua, this, ()| {
            let lines = this.lines.clone();
            let cursor = Arc::new(AtomicUsize::new(0));
            lua.create_function(move |lua, ()| {
                let guard = lock_lines(&lines);
                let i = cursor.fetch_add(1, Ordering::Relaxed);
                if i < guard.len() {
                    Ok(Some(line_to_table(lua, &guard[i])?))
                } else {
                    Ok(None)
                }
            })
        });

        // g:layers() — iterator over layers segmented on LayerChange.
        //
        // Recomputes the k-th layer's position against the LIVE buffer
        // on every step, so a plugin that inserts/removes while
        // iterating (e.g. a pause at several layers) still gets correct
        // `first_line`/`last_line` for later layers — the positions
        // shift as it edits. Cost is O(lines) per step; a plugin that
        // walks every layer of a huge file is O(lines × layers), but
        // the common "act on a few layers" case is cheap.
        methods.add_method("layers", |lua, this, ()| {
            let lines = this.lines.clone();
            let cursor = Arc::new(AtomicUsize::new(0));
            lua.create_function(move |lua, ()| {
                let guard = lock_lines(&lines);
                let k = cursor.fetch_add(1, Ordering::Relaxed);
                let Some(layer) = nth_layer(&guard, k) else {
                    return Ok(None);
                };
                let t = lua.create_table()?;
                t.set("index", layer.index)?;
                set_opt_num(&t, "z", layer.z)?;
                t.set("first_line", layer.first_line)?;
                t.set("last_line", layer.last_line)?;
                Ok(Some(t))
            })
        });

        // Mutations. A `line` arg is a raw G-code string (parsed, may
        // expand to several lines) or a constructed table.
        methods.add_method("append", |_, this, value: Value| {
            let new = value_to_lines(value)?;
            let mut lines = lock_lines(&this.lines);
            // The current last line gains a successor — make sure it's
            // newline-terminated so the two don't merge on serialize.
            let end = lines.len();
            ensure_terminated(&mut lines, end);
            lines.extend(new);
            Ok(())
        });

        methods.add_method("insert", |_, this, (i, value): (usize, Value)| {
            let new = value_to_lines(value)?;
            let mut lines = lock_lines(&this.lines);
            let idx = i.checked_sub(1).ok_or_else(|| index_err(i))?;
            if idx > lines.len() {
                return Err(index_err(i));
            }
            ensure_terminated(&mut lines, idx);
            for (k, line) in new.into_iter().enumerate() {
                lines.insert(idx + k, line);
            }
            Ok(())
        });

        methods.add_method("replace", |_, this, (i, value): (usize, Value)| {
            let new = value_to_lines(value)?;
            let mut lines = lock_lines(&this.lines);
            let idx = i
                .checked_sub(1)
                .filter(|&z| z < lines.len())
                .ok_or_else(|| index_err(i))?;
            lines.remove(idx);
            ensure_terminated(&mut lines, idx);
            for (k, line) in new.into_iter().enumerate() {
                lines.insert(idx + k, line);
            }
            Ok(())
        });

        methods.add_method("remove", |_, this, i: usize| {
            let mut lines = lock_lines(&this.lines);
            let idx = i
                .checked_sub(1)
                .filter(|&z| z < lines.len())
                .ok_or_else(|| index_err(i))?;
            lines.remove(idx);
            Ok(())
        });
    }
}

/// Ensure the line at `idx - 1` (the line that will precede freshly
/// inserted content) is newline-terminated, so inserting after an
/// unterminated final line can't merge the two on serialize.
fn ensure_terminated(lines: &mut [Line], idx: usize) {
    if idx == 0 {
        return;
    }
    if let Some(prev) = lines.get_mut(idx - 1) {
        if prev.line_ending().is_empty() {
            prev.set_line_ending("\n");
        }
    }
}

/// Resolve the `k`-th layer (0-based, by LayerChange occurrence order)
/// against the current buffer: its `LayerChange` position becomes
/// `first_line` (1-based), and the line just before the next
/// LayerChange (or the buffer end) becomes `last_line`.
struct LayerInfo {
    index: u32,
    z: Option<f32>,
    first_line: usize,
    last_line: usize,
}

fn nth_layer(lines: &[Line], k: usize) -> Option<LayerInfo> {
    let mut seen = 0usize;
    let mut found: Option<(usize, u32, Option<f32>)> = None;
    let mut next_pos: Option<usize> = None;
    for (i, line) in lines.iter().enumerate() {
        if let Line::LayerChange(lc) = line {
            if seen == k {
                found = Some((i, lc.index, lc.z));
            } else if seen == k + 1 {
                next_pos = Some(i);
                break;
            }
            seen += 1;
        }
    }
    let (pos, index, z) = found?;
    Some(LayerInfo {
        index,
        z,
        first_line: pos + 1,
        last_line: next_pos.unwrap_or(lines.len()),
    })
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
            Ok(Line::Comment(Comment::new(raw, style)))
        }
        "other" => {
            let raw: String = t.get("raw").map_err(|_| {
                mlua::Error::RuntimeError("other line needs a `raw` field".to_string())
            })?;
            Ok(Line::Other(Other::new(raw)))
        }
        other => Err(mlua::Error::RuntimeError(format!(
            "cannot build a `{other}` line from a table; pass move / tool / layer lines as a raw G-code string"
        ))),
    }
}

fn ensure_newline(line: &mut Line) {
    if line.line_ending().is_empty() {
        line.set_line_ending("\n");
    }
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
    fn inserts_at_multiple_layers_stay_aligned_under_mutation() {
        // The classic trap: insert at every layer while iterating.
        // g:layers() recomputes positions live, so the second insert
        // lands at the (shifted) layer-1 boundary, not a stale offset.
        let out = run(r#"function go(g)
            for layer in g:layers() do
                g:insert(layer.first_line, "; MARK " .. layer.index)
            end
        end"#);
        let lines: Vec<&str> = out.lines().collect();
        // Each MARK sits immediately after its own ;LAYER comment.
        for idx in [0, 1] {
            let layer = lines
                .iter()
                .position(|l| *l == format!(";LAYER:{idx}"))
                .unwrap();
            assert_eq!(
                lines[layer + 1],
                format!("; MARK {idx}"),
                "MARK {idx} should sit just after ;LAYER:{idx}"
            );
        }
    }

    #[test]
    fn append_does_not_merge_with_an_unterminated_last_line() {
        // A file whose final line has no trailing newline.
        let src = "G1 X0 Y0 F1200\nM84";
        let rt = PluginRuntime::load(r#"function go(g) g:append("M300 S440 P200") end"#, "t")
            .unwrap();
        let handle = GcodeHandle::new(parse_str(src));
        let cell = handle.cell();
        let _: Option<()> = rt.call("go", handle).unwrap();
        let out = to_string(&cell.lock().unwrap());
        assert!(out.contains("M84\n"), "prior line should gain a newline");
        assert!(!out.contains("M84M300"), "lines must not merge: {out:?}");
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
