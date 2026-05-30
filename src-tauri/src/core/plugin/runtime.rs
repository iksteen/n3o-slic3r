//! Single-plugin Lua runtime: load a chunk, call its functions under
//! instruction + memory bounds.
//!
//! This is the primitive the multi-plugin host will own one-per-
//! plugin. It deliberately knows nothing about manifests, hooks, or
//! host data — it just runs sandboxed Lua safely.

use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Arc;

use mlua::{FromLuaMulti, HookTriggers, IntoLuaMulti, Lua, Value, VmState};

use super::error::PluginError;
use super::sandbox;

/// Max Lua VM instructions a single `call` may execute before it's
/// aborted as a runaway. ~50M is generous for a G-code transform pass
/// while still bounding an accidental infinite loop to a fraction of a
/// second.
const DEFAULT_INSTRUCTION_BUDGET: i64 = 50_000_000;

/// How often (in VM instructions) the budget hook fires. Coarse enough
/// that the per-tick overhead is negligible, fine enough that a tight
/// loop is caught promptly.
const INSTRUCTION_CHECK_INTERVAL: u32 = 100_000;

/// Max bytes the plugin's Lua heap may grow to. A transform plugin
/// holds the typed G-code; 64 MiB is comfortable headroom without
/// letting a buggy plugin exhaust host memory.
const DEFAULT_MEMORY_LIMIT: usize = 64 * 1024 * 1024;

/// A loaded, sandboxed plugin chunk. Top-level code has already run
/// (so its global functions are defined); [`PluginRuntime::call`]
/// invokes them.
pub struct PluginRuntime {
    lua: Lua,
    /// Instructions remaining in the *current* call. Reset at the top
    /// of every `call`; the budget hook decrements it and aborts when
    /// it crosses zero. Shared with the hook closure inside `lua`.
    remaining: Arc<AtomicI64>,
    /// Set by the hook when it aborts a call for budget exhaustion, so
    /// `call` can distinguish a timeout from an ordinary Lua error.
    timed_out: Arc<AtomicBool>,
}

impl PluginRuntime {
    /// Build a sandboxed runtime, install the resource bounds, and run
    /// `source` so its top-level definitions take effect. Compile,
    /// sandbox, and top-level-execution errors all surface here.
    pub fn load(source: &str, name: &str) -> Result<Self, PluginError> {
        let lua = sandbox::build_sandbox()?;
        lua.set_memory_limit(DEFAULT_MEMORY_LIMIT)
            .map_err(|e| PluginError::Load(format!("set memory limit: {e}")))?;

        let remaining = Arc::new(AtomicI64::new(DEFAULT_INSTRUCTION_BUDGET));
        let timed_out = Arc::new(AtomicBool::new(false));

        let hook_remaining = remaining.clone();
        let hook_timed_out = timed_out.clone();
        lua.set_hook(
            HookTriggers::new().every_nth_instruction(INSTRUCTION_CHECK_INTERVAL),
            move |_lua, _debug| {
                let left =
                    hook_remaining.fetch_sub(INSTRUCTION_CHECK_INTERVAL as i64, Ordering::Relaxed);
                if left <= 0 {
                    hook_timed_out.store(true, Ordering::Relaxed);
                    Err(mlua::Error::RuntimeError(
                        "plugin exceeded its instruction budget".to_string(),
                    ))
                } else {
                    Ok(VmState::Continue)
                }
            },
        )
        .map_err(|e| PluginError::Load(format!("set hook: {e}")))?;

        // Run the chunk's top level. Budget already primed above, so a
        // pathological top level is bounded too.
        lua.load(source)
            .set_name(name)
            .exec()
            .map_err(|e| PluginError::Load(format!("{name}: {e}")))?;

        Ok(Self {
            lua,
            remaining,
            timed_out,
        })
    }

    /// Build a value in this runtime's Lua via `build` and install it as
    /// the global `settings` for the next hook call. Keeps the runtime
    /// ignorant of manifest/host types — the host supplies the builder
    /// (which knows the plugin's declared settings + resolved values).
    pub fn install_settings<F>(&self, build: F) -> Result<(), PluginError>
    where
        F: FnOnce(&Lua) -> mlua::Result<mlua::Table>,
    {
        let table =
            build(&self.lua).map_err(|e| PluginError::Runtime(format!("build settings: {e}")))?;
        self.lua
            .globals()
            .set("settings", table)
            .map_err(|e| PluginError::Runtime(format!("set settings global: {e}")))?;
        Ok(())
    }

    /// Call a global Lua function by name under a fresh instruction
    /// budget. Returns `Ok(None)` when the function isn't defined, so
    /// optional hooks are cheap to probe.
    pub fn call<A, R>(&self, func: &str, args: A) -> Result<Option<R>, PluginError>
    where
        A: IntoLuaMulti,
        R: FromLuaMulti,
    {
        let value: Value = self
            .lua
            .globals()
            .get(func)
            .map_err(|e| PluginError::Runtime(format!("lookup `{func}`: {e}")))?;
        let function = match value {
            Value::Function(f) => f,
            Value::Nil => return Ok(None),
            other => {
                return Err(PluginError::BadReturn(format!(
                    "global `{func}` is a {}, not a function",
                    other.type_name()
                )))
            }
        };

        self.remaining
            .store(DEFAULT_INSTRUCTION_BUDGET, Ordering::Relaxed);
        self.timed_out.store(false, Ordering::Relaxed);

        match function.call::<R>(args) {
            Ok(ret) => Ok(Some(ret)),
            Err(e) => {
                if self.timed_out.load(Ordering::Relaxed) {
                    Err(PluginError::Timeout)
                } else {
                    Err(PluginError::Runtime(format!("`{func}`: {e}")))
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calls_a_defined_function() {
        let rt = PluginRuntime::load("function add(a, b) return a + b end", "add").unwrap();
        let r: Option<i64> = rt.call("add", (2, 3)).unwrap();
        assert_eq!(r, Some(5));
    }

    #[test]
    fn absent_function_returns_none() {
        let rt = PluginRuntime::load("", "empty").unwrap();
        let r: Option<i64> = rt.call("nope", ()).unwrap();
        assert_eq!(r, None);
    }

    #[test]
    fn non_function_global_is_bad_return() {
        let rt = PluginRuntime::load("answer = 42", "x").unwrap();
        let r: Result<Option<i64>, _> = rt.call("answer", ());
        assert!(matches!(r, Err(PluginError::BadReturn(_))));
    }

    /// The whole point of the sandbox: the dangerous globals aren't
    /// even present, so a plugin can't reach the filesystem / process
    /// / dynamic loader. Asserted from the Lua side.
    #[test]
    fn unsafe_globals_are_absent() {
        let rt = PluginRuntime::load(
            r#"
            function probe()
                return io == nil
                    and package == nil
                    and require == nil
                    and debug == nil
                    and load == nil
                    and loadstring == nil
                    and loadfile == nil
                    and dofile == nil
                    and os.execute == nil
                    and os.getenv == nil
                    and os.remove == nil
                    and os.exit == nil
                    and rawset == nil
                    and getmetatable == nil
                    and setmetatable == nil
            end
            "#,
            "probe",
        )
        .unwrap();
        let all_absent: Option<bool> = rt.call("probe", ()).unwrap();
        assert_eq!(all_absent, Some(true), "a denied global leaked into the sandbox");
    }

    #[test]
    fn os_execute_cannot_run_a_command() {
        let marker = std::env::temp_dir().join("n3o_sandbox_breach_marker");
        let _ = std::fs::remove_file(&marker);
        let src = format!(
            r#"function go() os.execute("touch {}") end"#,
            marker.display()
        );
        let rt = PluginRuntime::load(&src, "breach").unwrap();
        // os.execute is nil → indexing/calling it errors; never runs.
        let r: Result<Option<()>, _> = rt.call("go", ());
        assert!(r.is_err());
        assert!(
            !marker.exists(),
            "sandbox let a plugin execute a shell command"
        );
    }

    #[test]
    fn io_open_is_denied() {
        let rt =
            PluginRuntime::load(r#"function go() return io.open("/etc/passwd", "r") end"#, "io")
                .unwrap();
        let r: Result<Option<bool>, _> = rt.call("go", ());
        assert!(r.is_err(), "io.open should be unreachable");
    }

    #[test]
    fn runaway_loop_times_out() {
        let rt = PluginRuntime::load("function spin() while true do end end", "spin").unwrap();
        let r: Result<Option<()>, PluginError> = rt.call("spin", ());
        assert!(
            matches!(r, Err(PluginError::Timeout)),
            "runaway loop should hit the instruction budget, got {r:?}"
        );
    }

    #[test]
    fn os_time_shim_returns_a_plausible_timestamp() {
        let rt = PluginRuntime::load("function now() return os.time() end", "time").unwrap();
        let r: Option<i64> = rt.call("now", ()).unwrap();
        // After 2023-11-14; just a sanity floor that the shim is wired.
        assert!(r.unwrap() > 1_700_000_000);
    }

    #[test]
    fn safe_stdlib_is_available() {
        let rt = PluginRuntime::load(
            r#"function go() return string.upper("hi") .. tostring(math.max(1, 2)) end"#,
            "std",
        )
        .unwrap();
        let r: Option<String> = rt.call("go", ()).unwrap();
        assert_eq!(r.as_deref(), Some("HI2"));
    }
}
