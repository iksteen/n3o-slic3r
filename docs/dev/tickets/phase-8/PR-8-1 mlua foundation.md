# PR-8-1 — mlua foundation + sandbox

Status: ✅ done.

**Spike findings.** `mlua 0.11` with `features = ["lua54",
"vendored"]` builds Lua 5.4 from source and links cleanly alongside
the cmake-built `libslic3r_ffi.so` — no symbol clash, no extra system
dep. Sandbox API used: `Lua::new_with(StdLib::STRING | TABLE | MATH |
COROUTINE, LuaOptions::default())` loads only the safe libs; the base
library is always present so `load`/`loadstring`/`loadfile`/`dofile`
are stripped explicitly. Instruction budget via `set_hook(HookTriggers
::new().every_nth_instruction(N), …)` returning `Err` to abort
(callback returns `Result<VmState>`); memory cap via
`set_memory_limit`. All nine tests pass, including the sandbox-escape
battery and the runaway-loop timeout.

**Scope.** Stand up the embedded Lua runtime that the rest of Phase 8
builds on: add `mlua` (Lua 5.4, vendored so we don't depend on a system
Lua and stay self-contained per the standalone-at-runtime principle),
wrap it in a sandbox that denies filesystem / process / dynamic-load
access by default, and prove a `.lua` chunk can be loaded and a named
function called from Rust. This ticket **doubles as the mlua-integration
spike** (PRD §11.2): it validates the exact sandboxing API and the
build/link story before any hook plumbing commits to it. No hooks, no
manifests, no G-code yet — just "Rust can run untrusted Lua safely."

Owns **FR-PL-1** (sandboxed Lua runtime).

**Acceptance criteria.**

- `mlua` added to `src-tauri/Cargo.toml`:
  ```toml
  mlua = { version = "0.11", features = ["lua54", "vendored"] }
  ```
  `vendored` builds Lua 5.4 from source via the crate — confirm it
  compiles in the workspace's `cargo build` without a system Lua, and
  that the resulting binary still links (this is the spike's primary
  build risk).

- New `core/plugin/` module fleshed out from today's stub:
  - `sandbox.rs` — constructs the restricted `Lua` runtime.
  - `runtime.rs` — `PluginRuntime` wrapper: owns one `Lua`, loads a
    chunk, looks up and calls a named global function with typed
    args/returns.
  - `error.rs` — `PluginError` (typed: `Load`, `Runtime`, `Sandbox`,
    `Timeout`, `BadReturn`) with a user-facing `Display`.
  - `mod.rs` re-exports; drop the "implementation lands in Phase 8"
    note now that it does.

- **Sandbox policy.** Build the runtime with only the safe standard
  libraries loaded — `string`, `table`, `math`, `os.time`/`os.clock`
  (time only), and `coroutine`. **Denied by default:** `io`,
  `os.execute`/`os.getenv`/`os.remove`/`os.rename`/`os.exit`,
  `package`/`require`, `dofile`/`loadfile`/`load` of arbitrary strings,
  `debug`. Prefer mlua's standard-library selection
  (`Lua::new_with(StdLib::…, LuaOptions::…)`) over loading everything
  then deleting globals; if a needed function rides in an otherwise-
  unsafe lib (e.g. `os.time`), install a hand-curated shim table rather
  than exposing the whole lib. Document the final allow/deny list in
  `sandbox.rs`.

- **Resource bounds.** A plugin chunk that runs away does not hang the
  host: install an instruction-count interrupt (mlua hook /
  `set_interrupt`) that aborts a call after a configurable budget,
  surfaced as `PluginError::Timeout`. Memory limit set via mlua's
  `set_memory_limit` to a sane default (document the number).

- `PluginRuntime` API (shape, not final):
  ```rust
  impl PluginRuntime {
      /// Build a sandboxed runtime and load `source` (a plugin's Lua
      /// body). Compile/sandbox errors surface here, not at call time.
      fn load(source: &str, name: &str) -> Result<Self, PluginError>;
      /// Call a global Lua function by name. `A`/`R` are mlua-
      /// convertible. Absent function → Ok(None) so optional hooks are
      /// cheap to probe.
      fn call<A: IntoLuaMulti, R: FromLuaMulti>(
          &self, func: &str, args: A,
      ) -> Result<Option<R>, PluginError>;
  }
  ```

- Tests (the spike's evidence):
  - Load a trivial chunk, call `function add(a,b) return a+b end`,
    assert `add(2,3) == 5`.
  - **Sandbox-escape battery** — each asserts `PluginError`/`nil`, not
    a successful effect: `os.execute("touch /tmp/x")`, `io.open(...)`,
    `require("os")`, `loadfile("/etc/passwd")`, `os.getenv("HOME")`,
    `debug.getinfo(...)`. Confirm none of the denied globals are even
    present (`assert(io == nil)` from Lua side).
  - Runaway `while true do end` aborts via the interrupt within the
    budget and returns `PluginError::Timeout` (gate the wall-clock
    assertion loosely to avoid flakiness).
  - Calling an absent function returns `Ok(None)`.

- No Tauri command surface and no frontend in this ticket — pure core
  module + tests. (Commands arrive with the host in PR-8-3.)

**Effort.** ~1.5 days. The vendored-Lua build + nailing the exact
sandbox API is the spike cost; the wrapper itself is small.

**Dependencies.** None — first ticket of Phase 8. Independent of
Phase 7.

**Out of scope.**

- Plugin manifests / folder discovery → PR-8-2.
- The `PluginHost` that owns *multiple* plugins and dispatches hooks →
  PR-8-3. This ticket is a single-runtime primitive.
- Any hook wiring, G-code bindings, or filament bindings.
- Exposing host data *into* Lua beyond the trivial test args — the
  typed-G-code and filament userdata land in PR-8-4 / PR-8-8.

**Spike notes to capture in the PR description.** Whether `vendored`
Lua 5.4 builds clean on the Linux toolchain; the exact mlua API used
for stdlib restriction and the interrupt; binary size delta; any
surprise in linking vendored Lua alongside the libslic3r FFI `.so`.
