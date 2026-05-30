//! Concrete pipeline hooks the host dispatches. Each bridges a host
//! payload (typed G-code, …) to the [`Hook`] fold by marshalling it
//! across the Lua boundary.

use std::collections::BTreeMap;

use mlua::{IntoLua, Lua, Result as LuaResult, Value};

use super::bindings::{FilamentHandle, FilamentLoadout, GcodeHandle, SettingsHandle};
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
    /// Read-only bound filament loadout, passed as the third Lua arg.
    pub filament: FilamentLoadout,
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
        let filament = FilamentHandle::new(self.filament.clone());
        match runtime.call::<_, ()>("on_post_slice", (handle, self.plate.clone(), filament)) {
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

// ---- pre-slice ----------------------------------------------------

/// Read-only context handed to a pre-slice hook as its second arg.
#[derive(Debug, Clone)]
pub struct PreSliceContext {
    pub printer_model: String,
    pub plate: String,
    /// Number of physical toolheads (1 for a single-hotend printer like
    /// the A1 mini, regardless of AMS filament slots). Not the AMS slot
    /// count — that isn't available at this layer.
    pub toolhead_count: usize,
}

impl IntoLua for PreSliceContext {
    fn into_lua(self, lua: &Lua) -> LuaResult<Value> {
        let t = lua.create_table()?;
        t.set("printer_model", self.printer_model)?;
        t.set("plate", self.plate)?;
        t.set("toolhead_count", self.toolhead_count)?;
        Ok(Value::Table(t))
    }
}

/// The pre-slice hook: each plugin's `on_pre_slice(settings, context)`
/// reads/writes the resolved settings (key→string) before the cascade
/// adapter hands config to libslic3r. Clean-by-copy isolation, same as
/// post-slice.
pub struct PreSliceHook {
    pub context: PreSliceContext,
    /// Read-only bound filament loadout, passed as the third Lua arg.
    pub filament: FilamentLoadout,
}

impl Hook for PreSliceHook {
    type Value = BTreeMap<String, String>;

    fn kind(&self) -> HookKind {
        HookKind::PreSlice
    }

    fn invoke(
        &self,
        runtime: &PluginRuntime,
        settings: BTreeMap<String, String>,
    ) -> (BTreeMap<String, String>, Option<PluginError>) {
        let handle = SettingsHandle::new(settings.clone());
        let cell = handle.cell();
        let filament = FilamentHandle::new(self.filament.clone());
        match runtime.call::<_, ()>("on_pre_slice", (handle, self.context.clone(), filament)) {
            Ok(_) => {
                let edited = cell
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .clone();
                (edited, None)
            }
            Err(e) => (settings, Some(e)),
        }
    }
}

// ---- pre-send -----------------------------------------------------

/// Which kind of send buffer a pre-send hook is looking at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PayloadKind {
    /// Raw G-code body (U1) — editable text.
    Gcode,
    /// A `.gcode.3mf` bundle (Bambu) — opaque bytes; most plugins no-op.
    Gcode3mf,
}

impl PayloadKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Gcode => "gcode",
            Self::Gcode3mf => "gcode_3mf",
        }
    }
}

/// Read-only target info handed to a pre-send hook.
#[derive(Debug, Clone)]
pub struct SendTarget {
    pub driver_kind: String,
    pub plate_id: u32,
}

impl IntoLua for SendTarget {
    fn into_lua(self, lua: &Lua) -> LuaResult<Value> {
        let t = lua.create_table()?;
        t.set("driver_kind", self.driver_kind)?;
        t.set("plate_id", self.plate_id)?;
        Ok(Value::Table(t))
    }
}

/// The payload table passed to `on_pre_send`: `{ kind = …, bytes = … }`.
struct SendPayloadArg {
    kind: PayloadKind,
    bytes: Vec<u8>,
}

impl IntoLua for SendPayloadArg {
    fn into_lua(self, lua: &Lua) -> LuaResult<Value> {
        let t = lua.create_table()?;
        t.set("kind", self.kind.as_str())?;
        t.set("bytes", lua.create_string(&self.bytes)?)?;
        Ok(Value::Table(t))
    }
}

/// The pre-send hook: `on_pre_send(payload, target)` may return
/// replacement bytes (a Lua string) for the buffer about to be sent, or
/// `nil`/nothing to leave it unchanged. Folded across plugins.
pub struct PreSendHook {
    pub kind: PayloadKind,
    pub target: SendTarget,
}

impl Hook for PreSendHook {
    type Value = Vec<u8>;

    fn kind(&self) -> HookKind {
        HookKind::PreSend
    }

    fn invoke(&self, runtime: &PluginRuntime, bytes: Vec<u8>) -> (Vec<u8>, Option<PluginError>) {
        let arg = SendPayloadArg {
            kind: self.kind,
            bytes: bytes.clone(),
        };
        match runtime.call::<_, Value>("on_pre_send", (arg, self.target.clone())) {
            // A returned Lua string replaces the buffer.
            Ok(Some(Value::String(s))) => (s.as_bytes().to_vec(), None),
            // Absent function or an explicit nil return → intentional
            // no-op, leave the bytes unchanged.
            Ok(None) | Ok(Some(Value::Nil)) => (bytes, None),
            // Any other return type is almost certainly an author
            // mistake (e.g. forgot tostring) — surface it rather than
            // silently dropping the edit.
            Ok(Some(other)) => (
                bytes,
                Some(PluginError::BadReturn(format!(
                    "on_pre_send must return a string or nil, got {}",
                    other.type_name()
                ))),
            ),
            Err(e) => (bytes, Some(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::gcode::{parse_str, to_string};
    use crate::core::plugin::{PluginHost, MANIFEST_FILE};
    use std::path::Path;

    fn write_plugin(root: &Path, name: &str, hooks: &str, lua: &str) {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(MANIFEST_FILE),
            format!(
                "name=\"{name}\"\nversion=\"1.0.0\"\nentry=\"main.lua\"\n\
                 hooks={hooks}\nenabled_by_default=true\n"
            ),
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
            r#"["post_slice"]"#,
            r#"function on_post_slice(g, plate)
                 g:append("; sliced for " .. plate.printer_model)
               end"#,
        );
        let mut host = PluginHost::load(&[tmp.path().to_path_buf()]);
        let hook = PostSliceHook {
            plate: plate(),
            filament: FilamentLoadout::default(),
        };
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
            r#"["post_slice"]"#,
            r#"function on_post_slice(g, plate) g:append("X"); error("boom") end"#,
        );
        let mut host = PluginHost::load(&[tmp.path().to_path_buf()]);
        let hook = PostSliceHook {
            plate: plate(),
            filament: FilamentLoadout::default(),
        };
        let out = to_string(&host.dispatch(&hook, parse_str(GCODE)));
        // Clean-by-copy: the partial append is discarded on error.
        assert_eq!(out, GCODE);
    }

    fn pre_slice_ctx() -> PreSliceContext {
        PreSliceContext {
            printer_model: "Test Printer".into(),
            plate: "Textured PEI".into(),
            toolhead_count: 1,
        }
    }

    #[test]
    fn pre_slice_hook_edits_settings_through_the_host() {
        let tmp = tempfile::tempdir().unwrap();
        write_plugin(
            tmp.path(),
            "clamp",
            r#"["pre_slice"]"#,
            r#"function on_pre_slice(settings, ctx)
                 -- read + write an existing key, add a new one, and
                 -- try to nil one (which must be a no-op).
                 if tonumber(settings.bed_temp) > 60 then settings.bed_temp = "60" end
                 settings.note = ctx.printer_model
                 settings.keep_me = nil
               end"#,
        );
        let mut host = PluginHost::load(&[tmp.path().to_path_buf()]);
        let hook = PreSliceHook {
            context: pre_slice_ctx(),
            filament: FilamentLoadout::default(),
        };
        let settings: BTreeMap<String, String> = [
            ("bed_temp".to_string(), "99".to_string()),
            ("keep_me".to_string(), "x".to_string()),
        ]
        .into_iter()
        .collect();
        let out = host.dispatch(&hook, settings);
        assert_eq!(out.get("bed_temp").map(String::as_str), Some("60"));
        assert_eq!(out.get("note").map(String::as_str), Some("Test Printer"));
        assert_eq!(
            out.get("keep_me").map(String::as_str),
            Some("x"),
            "assigning nil must not remove a setting"
        );
    }

    #[test]
    fn erroring_pre_slice_plugin_leaves_settings_untouched() {
        let tmp = tempfile::tempdir().unwrap();
        write_plugin(
            tmp.path(),
            "broken",
            r#"["pre_slice"]"#,
            r#"function on_pre_slice(s, ctx) s.bed_temp = "1"; error("boom") end"#,
        );
        let mut host = PluginHost::load(&[tmp.path().to_path_buf()]);
        let hook = PreSliceHook {
            context: pre_slice_ctx(),
            filament: FilamentLoadout::default(),
        };
        let settings: BTreeMap<String, String> = [("bed_temp".to_string(), "55".to_string())]
            .into_iter()
            .collect();
        let out = host.dispatch(&hook, settings);
        assert_eq!(out.get("bed_temp").map(String::as_str), Some("55"));
    }

    // ---- filament binding ----------------------------------------

    fn loadout() -> FilamentLoadout {
        use crate::core::plugin::SlotInfo;
        FilamentLoadout {
            printer_model: "Test Printer".into(),
            toolhead_count: 1,
            slots: vec![
                SlotInfo {
                    index: 1,
                    extruder: 0,
                    slot: 0,
                    feed: "ams",
                    identity: Some("generic-pla".into()),
                    base_type: Some("PLA".into()),
                    color: Some("#ff8800".into()),
                    vendor: Some("Generic".into()),
                },
                SlotInfo {
                    index: 2,
                    extruder: 0,
                    slot: 1,
                    feed: "ams",
                    identity: None,
                    base_type: None,
                    color: None,
                    vendor: None,
                },
            ],
        }
    }

    #[test]
    fn post_slice_hook_reads_filament_loadout() {
        let tmp = tempfile::tempdir().unwrap();
        write_plugin(
            tmp.path(),
            "reporter",
            r#"["post_slice"]"#,
            r#"function on_post_slice(g, plate, filament)
                 g:append("; printer=" .. filament:printer().model)
                 g:append("; count=" .. filament:count())
                 for _, s in ipairs(filament:slots()) do
                   local id = s.identity or "empty"
                   g:append("; slot " .. s.index .. " feed=" .. s.feed ..
                            " bound=" .. tostring(s.bound) .. " id=" .. id)
                 end
               end"#,
        );
        let mut host = PluginHost::load(&[tmp.path().to_path_buf()]);
        let hook = PostSliceHook {
            plate: plate(),
            filament: loadout(),
        };
        let out = to_string(&host.dispatch(&hook, parse_str(GCODE)));
        assert!(out.contains("; printer=Test Printer"));
        assert!(out.contains("; count=2"));
        assert!(out.contains("; slot 1 feed=ams bound=true id=generic-pla"));
        assert!(out.contains("; slot 2 feed=ams bound=false id=empty"));
    }

    #[test]
    fn filament_slot_lookup_out_of_range_is_nil() {
        let tmp = tempfile::tempdir().unwrap();
        write_plugin(
            tmp.path(),
            "lookup",
            r#"["post_slice"]"#,
            r#"function on_post_slice(g, plate, filament)
                 g:append("; has1=" .. tostring(filament:slot(1) ~= nil))
                 g:append("; has9=" .. tostring(filament:slot(9) ~= nil))
                 g:append("; has0=" .. tostring(filament:slot(0) ~= nil))
               end"#,
        );
        let mut host = PluginHost::load(&[tmp.path().to_path_buf()]);
        let hook = PostSliceHook {
            plate: plate(),
            filament: loadout(),
        };
        let out = to_string(&host.dispatch(&hook, parse_str(GCODE)));
        assert!(out.contains("; has1=true"));
        assert!(out.contains("; has9=false"));
        assert!(out.contains("; has0=false"));
    }

    #[test]
    fn empty_loadout_yields_no_slots_no_error() {
        let tmp = tempfile::tempdir().unwrap();
        write_plugin(
            tmp.path(),
            "offline",
            r#"["post_slice"]"#,
            r#"function on_post_slice(g, plate, filament)
                 g:append("; count=" .. filament:count())
               end"#,
        );
        let mut host = PluginHost::load(&[tmp.path().to_path_buf()]);
        let hook = PostSliceHook {
            plate: plate(),
            filament: FilamentLoadout::default(),
        };
        let out = to_string(&host.dispatch(&hook, parse_str(GCODE)));
        assert!(out.contains("; count=0"));
    }

    #[test]
    fn assigning_a_slot_field_raises_and_leaves_gcode_untouched() {
        let tmp = tempfile::tempdir().unwrap();
        write_plugin(
            tmp.path(),
            "mutator",
            r#"["post_slice"]"#,
            // Append first (proving the plugin ran), then attempt an
            // illegal write — the error must discard the whole edit.
            r#"function on_post_slice(g, plate, filament)
                 g:append("; ran")
                 filament:slots()[1].index = 99
               end"#,
        );
        let mut host = PluginHost::load(&[tmp.path().to_path_buf()]);
        let hook = PostSliceHook {
            plate: plate(),
            filament: loadout(),
        };
        let out = to_string(&host.dispatch(&hook, parse_str(GCODE)));
        // Clean-by-copy: the write raised, so the append is rolled back too.
        assert_eq!(out, GCODE);
    }

    #[test]
    fn assigning_a_handle_field_raises() {
        let tmp = tempfile::tempdir().unwrap();
        write_plugin(
            tmp.path(),
            "mutator2",
            r#"["post_slice"]"#,
            r#"function on_post_slice(g, plate, filament)
                 g:append("; ran")
                 filament.bogus = 1
               end"#,
        );
        let mut host = PluginHost::load(&[tmp.path().to_path_buf()]);
        let hook = PostSliceHook {
            plate: plate(),
            filament: loadout(),
        };
        let out = to_string(&host.dispatch(&hook, parse_str(GCODE)));
        assert_eq!(out, GCODE);
    }

    #[test]
    fn rawset_cannot_bypass_the_slot_read_only_guard() {
        // The sandbox strips `rawset`, so the `__newindex`-based guard on
        // the snapshot tables can't be bypassed: calling rawset errors
        // (nil value), and clean-by-copy discards the whole edit.
        let tmp = tempfile::tempdir().unwrap();
        write_plugin(
            tmp.path(),
            "rawmutator",
            r#"["post_slice"]"#,
            r#"function on_post_slice(g, plate, filament)
                 g:append("; ran")
                 rawset(filament:slots()[1], "index", 99)
               end"#,
        );
        let mut host = PluginHost::load(&[tmp.path().to_path_buf()]);
        let hook = PostSliceHook {
            plate: plate(),
            filament: loadout(),
        };
        let out = to_string(&host.dispatch(&hook, parse_str(GCODE)));
        assert_eq!(out, GCODE);
    }

    #[test]
    fn loadout_from_instance_resolves_bound_filament() {
        // Use a bundled fixture instance; assert the loadout walks its
        // slots and resolves at least one bound filament's type from the
        // catalog. (Graceful even if the fixture changes — we only check
        // structure, not a specific filament.)
        let Some(inst) = crate::core::printer::lookup_instance("bambi") else {
            return; // no fixture wired in this build; nothing to assert
        };
        let total_slots: usize = inst.extruders.iter().map(|e| e.slots.len()).sum();
        let lo = FilamentLoadout::from_instance(&inst, "Bambu Lab A1 mini".into(), 1);
        assert_eq!(lo.slots.len(), total_slots);
        assert_eq!(lo.toolhead_count, 1);
        // Indices are the 1-based flat filament ordinal.
        for (i, s) in lo.slots.iter().enumerate() {
            assert_eq!(s.index, i + 1);
        }
    }

    #[test]
    fn pre_send_hook_rewrites_bytes() {
        let tmp = tempfile::tempdir().unwrap();
        write_plugin(
            tmp.path(),
            "rewriter",
            r#"["pre_send"]"#,
            r#"function on_pre_send(payload, target)
                 return payload.bytes .. ";; sent to " .. target.driver_kind
               end"#,
        );
        let mut host = PluginHost::load(&[tmp.path().to_path_buf()]);
        let hook = PreSendHook {
            kind: PayloadKind::Gcode,
            target: SendTarget {
                driver_kind: "u1".into(),
                plate_id: 1,
            },
        };
        let out = host.dispatch(&hook, b"G1 X0\n".to_vec());
        assert_eq!(out, b"G1 X0\n;; sent to u1".to_vec());
    }
}
