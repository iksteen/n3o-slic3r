# PR-1-4 — Override tiers (user + project, `!important` style)

Status: ✅ done. `src-tauri/src/core/cascade/overrides.rs` ships `OverrideTiers`, `FlatOverrides`, stricter override-file loader (rejects `[[rule]]` and `[section]` headers with file:line errors), and `resolve_with_overrides()` that applies user → project tiers on top of the authored cascade. `cascade_fallback` retained per overridden key. 8 unit tests covering project-beats-spec-2, user-vs-project tiering, override-only keys, both rejection paths.

**Scope.** Phase 2 of the two-phase resolution: user profile and
project file applied as absolute-override tiers that win over the
authored cascade regardless of specificity. Project tier ranks
higher than user tier; later-loaded source within a tier wins
ties between same-tier files. Per `docs/dev/profiles.md` "Resolution
semantics — two phases".

**Acceptance criteria.**

- `pub struct OverrideTiers { user: Vec<FlatOverrides>, project:
  Vec<FlatOverrides> }` where `FlatOverrides` is a TOML file
  containing top-level `key = value` entries (no `[[rule]]`, no
  `when.*`). Loaded via PR-1-2's parser in a stricter mode that
  rejects predicates.

- `pub fn resolve_with_overrides(cascade: &Cascade, overrides:
  &OverrideTiers, ctx: &Context) -> ResolvedWithTrace` returns
  the same `Resolved` shape from PR-1-3 plus an
  `override_source: Option<OverrideSource>` per key when an
  override is active. `override_source` carries
  `tier: OverrideTier (User | Project), file: SourceLocation`.

- Application order: (1) PR-1-3's authored-cascade resolve, (2)
  user-tier overrides applied on top (any matching `key = value`
  replaces the resolved value), (3) project-tier overrides applied
  on top of (2). Each layer fully overrides the previous for any
  key it touches.

- The `Resolved` value's `cascade_fallback: Option<String>`
  field records what the authored cascade would have resolved to
  if an override is active. Drives the "Reset to cascade" UI in
  Phase 4.

- Same-tier source order: when two user files (or two project
  files) both override the same key, the later-loaded one wins.
  This is a warning condition (similar to PR-1-3's cross-file
  same-specificity warning) — emit via `tracing::warn!`.

- Tests:
  - Project override `set.bed_temp = 50` beats a filament+plate
    rule at specificity 2 that resolves to `set.bed_temp = 55`.
    Resolved value is `50`, `override_source.tier = Project`,
    `cascade_fallback = Some("55")`.
  - User override behaves identically except `tier = User`.
  - Project + user both override the same key: project wins,
    user override is preserved in trace metadata for diagnostic
    purposes.
  - Override file with a `[[rule]]` block or `when.*` predicate
    is rejected at load time with a clear error.

**Effort.** ~2 days.

**Dependencies.** PR-1-2 (parser; extended for override-file
stricter mode), PR-1-3 (authored cascade resolution comes first).

**Out of scope.** Per-object overrides — those are Phase 3+ work
(needs a project model with per-object metadata). UI for setting
overrides (Phase 4). Persistence format for the user profile
(JSON vs TOML vs IPC blob) — leave the path-based loading
interface and decide format with UI work.
