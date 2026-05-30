# PR-8-3 — `PluginHost` + hook dispatch + error isolation

Status: ✅ done.

**Implementation notes.**
- Plugins are keyed in a `BTreeMap<name, _>`, which gives both the
  deterministic lexical dispatch order and **last-root-wins** dedup —
  a user plugin overrides a bundled one of the same name (roots are
  loaded bundled-first, user-second).
- The `Hook` trait's `invoke` returns `(Value, Option<PluginError>)`
  and hands back the *input* value on error, so the host folds without
  needing a `Clone` bound and skips a failed plugin's transform cleanly.
- `plugin:changed` is emitted by the mutating commands
  (`set_enabled` / `reload`). A mid-dispatch auto-disable mutates host
  state but does **not** emit yet — the orchestrator that calls
  `dispatch` (PR-8-5) will trigger the panel refresh.
- mlua's `send` feature is enabled so `Lua: Send` and the host can live
  in `State<Arc<Mutex<PluginHost>>>`; all registered closures are Send.
- Roots are wired in `lib.rs`: bundled `plugins/` (a `profiles/`
  sibling; `N3O_PLUGIN_ROOT` dev override mirrors `N3O_PROFILE_ROOT`) +
  `~/.local/share/n3o-slic3r/plugins`.

**Scope.** The multi-plugin host: take the discovered plugins, load
each into a sandboxed `PluginRuntime`, and dispatch a named hook to
every plugin that declares it — with strict error isolation so one
bad plugin can never crash the host or break the pipeline. This ticket
establishes the dispatch chain and its failure semantics with a
trivial hook payload; the real hook *data* (typed G-code, settings,
filament) and the *wiring into the slice/send pipeline* come in the
following tickets.

Owns the **FR-PL-3** dispatch infrastructure and the
"errors caught and surfaced without crashing the host" exit criterion.

**Acceptance criteria.**

- `PluginHost` (`core/plugin/host.rs`):
  - Built from a discovery result; loads each `Ok` plugin's entry Lua
    into a `PluginRuntime`. A plugin whose Lua fails to load is kept
    in the host in a **disabled/errored** state (not dropped), so it
    shows up in the panel with its error.
  - `LoadedPlugin { manifest, runtime: Option<PluginRuntime>, enabled:
    bool, last_error: Option<String> }`.
  - Dispatch order is deterministic: manifest-name lexical order
    (document it; plugins must not rely on cross-plugin ordering for
    correctness, but the order must be stable).

- Hook dispatch:
  - `dispatch<H: Hook>(&mut self, hook: H) -> H::Output` — calls the
    Lua function named for `H` (`on_pre_slice` / `on_post_slice` /
    `on_pre_send`) on each enabled plugin that declares that hook in
    its manifest, threading the payload through.
  - For **transform** hooks (post-slice, pre-send, pre-slice each
    mutate a value), the output of plugin N feeds plugin N+1 — a
    fold over the chain. The `Hook` trait abstracts "marshal the
    payload into Lua / unmarshal the result," so the per-hook data
    types plug in later without touching dispatch.
  - This ticket ships a **stub hook** (e.g. a string the chain may
    rewrite) to prove the fold + isolation end-to-end; the typed
    payloads land in PR-8-4..8-6.

- **Error isolation** (the load-bearing requirement):
  - A plugin hook that errors (Lua error, `Timeout`, bad return) is
    caught; the plugin's `last_error` is set and it is **auto-disabled
    for the rest of the session** (so a broken plugin doesn't re-fail
    on every plate). The chain continues with the *pre-hook* value for
    that plugin (its transform is skipped, not applied half-way).
  - A panic inside a Rust callback surfaces as a plugin error, not a
    host crash (mlua converts Rust panics to Lua errors; verify).
  - The host never returns an error to the pipeline caller for a
    plugin failure — the pipeline proceeds as if the failed plugin
    weren't there. Plugin failures are reported out-of-band (events +
    `last_error`), per "surfaced without crashing the host."

- Tauri command + event surface (`core/plugin/commands.rs`):
  - `plugin_list() -> Vec<PluginSummary>` — name, version, hooks,
    enabled, last_error.
  - `plugin_set_enabled(name, enabled) -> Result<(), String>`.
  - `plugin_reload(name) -> Result<(), String>` — re-load one
    plugin's Lua from disk (manual; the watcher in PR-8-10 calls the
    same path).
  - `plugin:changed` event when the host's plugin set / states change,
    so the panel (PR-8-9) refreshes.
  - `PluginHost` stored as `tauri::State<Arc<Mutex<PluginHost>>>`
    alongside the other registries.

- Tests:
  - Host with three stub plugins (two declare the hook, one doesn't):
    dispatch hits exactly the two, in lexical order, folding the value.
  - An erroring plugin is isolated: the chain still produces the other
    plugins' result, the bad plugin is disabled with `last_error` set,
    and re-dispatch skips it.
  - A `Timeout`-ing plugin (runaway loop) is isolated the same way.
  - `plugin_set_enabled(false)` removes a plugin from dispatch;
    `true` restores it (if it loads).

**Effort.** ~2 days. The fold + isolation semantics and the command
surface are the substance.

**Dependencies.** PR-8-1 (`PluginRuntime`), PR-8-2 (discovery +
manifest).

**Out of scope.**

- Marshalling the typed G-code / settings / filament into the hook
  payload → PR-8-4, PR-8-6, PR-8-8.
- Wiring dispatch into the actual slice / send pipeline → PR-8-5
  (post-slice), PR-8-6 (pre-slice + pre-send).
- Hot reload on file change → PR-8-10 (this ticket ships only the
  manual `plugin_reload`).
- The Plugins panel UI → PR-8-9 (this ticket ships the commands it
  consumes).
