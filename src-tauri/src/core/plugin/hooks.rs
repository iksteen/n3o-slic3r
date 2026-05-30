//! Concrete pipeline hooks the host dispatches. Each bridges a host
//! payload (typed G-code, …) to the [`Hook`] fold by marshalling it
//! across the Lua boundary.

use mlua::{IntoLua, Lua, Result as LuaResult, Value};

use super::bindings::GcodeHandle;
use super::error::PluginError;
use super::host::Hook;
use super::manifest::HookKind;
use super::runtime::PluginRuntime;
use crate::core::gcode::Line;

/// Read-only context about the plate being processed, handed to a hook
/// as its second Lua argument (a table).
#[derive(Debug, Clone)]
pub struct PlateMeta {
    pub plate_id: u32,
    pub printer_model: String,
    pub bed_type: Option<String>,
    /// `None` when the source doesn't track it (the orchestrator slices
    /// a model file and doesn't currently count objects).
    pub object_count: Option<usize>,
}

impl IntoLua for PlateMeta {
    fn into_lua(self, lua: &Lua) -> LuaResult<Value> {
        let t = lua.create_table()?;
        t.set("plate_id", self.plate_id)?;
        t.set("printer_model", self.printer_model)?;
        if let Some(bed) = self.bed_type {
            t.set("bed_type", bed)?;
        }
        if let Some(n) = self.object_count {
            t.set("object_count", n)?;
        }
        Ok(Value::Table(t))
    }
}

/// The post-slice hook: each plugin's `on_post_slice(gcode, plate)`
/// receives the plate's typed G-code (mutable) plus its metadata.
///
/// Isolation is clean-by-copy: each plugin gets a fresh [`GcodeHandle`]
/// over a clone of the current lines, and its edits are adopted only if
/// the call succeeds — a plugin that errors part-way leaves the prior
/// lines untouched (the fold continues with them).
pub struct PostSliceHook {
    pub plate: PlateMeta,
}

impl Hook for PostSliceHook {
    type Value = Vec<Line>;

    fn kind(&self) -> HookKind {
        HookKind::PostSlice
    }

    fn invoke(
        &self,
        runtime: &PluginRuntime,
        lines: Vec<Line>,
    ) -> (Vec<Line>, Option<PluginError>) {
        let handle = GcodeHandle::new(lines.clone());
        let cell = handle.cell();
        match runtime.call::<_, ()>("on_post_slice", (handle, self.plate.clone())) {
            Ok(_) => {
                // Recover the guard if a panic mid-edit poisoned the
                // cell, so one bad plugin can't wedge the host.
                let edited = cell
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .clone();
                (edited, None)
            }
            Err(e) => (lines, Some(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::gcode::{parse_str, to_string};
    use crate::core::plugin::{PluginHost, MANIFEST_FILE};
    use std::path::Path;

    fn write_plugin(root: &Path, name: &str, lua: &str) {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(MANIFEST_FILE),
            format!("name=\"{name}\"\nversion=\"1.0.0\"\nentry=\"main.lua\"\nhooks=[\"post_slice\"]\n"),
        )
        .unwrap();
        std::fs::write(dir.join("main.lua"), lua).unwrap();
    }

    fn plate() -> PlateMeta {
        PlateMeta {
            plate_id: 1,
            printer_model: "Test Printer".into(),
            bed_type: Some("Textured PEI".into()),
            object_count: Some(1),
        }
    }

    const GCODE: &str = "G1 X0 Y0 F1200\nG1 X10 Y0 E0.5\n";

    #[test]
    fn post_slice_hook_appends_through_the_host() {
        let tmp = tempfile::tempdir().unwrap();
        write_plugin(
            tmp.path(),
            "tagger",
            r#"function on_post_slice(g, plate)
                 g:append("; sliced for " .. plate.printer_model)
               end"#,
        );
        let mut host = PluginHost::load(&[tmp.path().to_path_buf()]);
        let hook = PostSliceHook { plate: plate() };
        let out = to_string(&host.dispatch(&hook, parse_str(GCODE)));
        assert!(out.starts_with(GCODE));
        assert!(out.ends_with("; sliced for Test Printer\n"));
    }

    #[test]
    fn erroring_post_slice_plugin_leaves_gcode_untouched() {
        let tmp = tempfile::tempdir().unwrap();
        write_plugin(
            tmp.path(),
            "broken",
            r#"function on_post_slice(g, plate) g:append("X"); error("boom") end"#,
        );
        let mut host = PluginHost::load(&[tmp.path().to_path_buf()]);
        let hook = PostSliceHook { plate: plate() };
        let out = to_string(&host.dispatch(&hook, parse_str(GCODE)));
        // Clean-by-copy: the partial append is discarded on error.
        assert_eq!(out, GCODE);
    }
}
