# PR-1-3 — Rule resolver (authored cascade)

Status: ✅ done (authored cascade only — override tiers come in PR-1-4). `src-tauri/src/core/cascade/resolver.rs` ships `Context` trait + `MapContext` impl, `Resolved`/`ResolvedValue` types with full matching-rules retention for PR-1-5 traces, and `resolve()` with specificity-ascending + source-order tiebreaks. Cross-file same-specificity tie emits a `tracing::warn!`. 7 unit tests including a tracing-capture assertion for the warning. Predicate operators today: scalar equality and array set-membership; numeric range / negation deferred to a later iteration as documented in the ticket.

**Scope.** Production resolver for the authored-cascade tier
(phase 1 of the two-phase resolution described in
`docs/profiles.md`). Given a parsed `Cascade` (from PR-1-2) and a
context object, returns the resolved logical settings per key with
specificity ordering and source-order tie-breaks. Emits
within-cascade tie-break warnings when two same-specificity rules
from different authored files both set the same key. Logs but
does *not* fail.

Replaces the stub resolver in `src-tauri/examples/spike1.rs`
(which can be deleted once the production resolver lives in
`src-tauri/src/core/cascade/`).

**Acceptance criteria.**

- `pub fn resolve(cascade: &Cascade, ctx: &Context) -> Resolved`
  where `Resolved` = `BTreeMap<String, ResolvedValue>` and
  `ResolvedValue` carries `value: String`, `winning_rule:
  &SourceLocation`, `winning_specificity: usize`.

- Predicate semantics covered:
  - **String equality** (the spike's only form):
    `when.filament.type = "PLA"`.
  - **Numeric range**: `when.nozzle.diameter = ">= 0.6"` —
    parser accepts `>=`, `<=`, `>`, `<`, `=` (default), and
    closed ranges `"0.4..0.8"`.
  - **Set membership**: `when.filament.type = ["PLA", "PETG"]`.
  - **Negation**: `when.plate.type != "Cool"`.
  - Predicate dispatch lives in a `Predicate::matches(ctx) -> bool`
    method; each operator is a variant. Specificity counts
    predicate dimensions, not predicate complexity (`when.filament
    .type = ["PLA", "PETG"]` is specificity 1).

- Specificity ranking: number of distinct context-dimension
  predicates in the rule's `when`. Default rule (no `when`) is
  specificity 0.

- Source-order tie-break: when two rules have the same specificity
  and both match, the later one (higher source position) wins.

- Within-cascade tie-break warning: when two rules at the same
  specificity from *different cascade files* both set the same key,
  log a warning via `tracing::warn!` with both source locations.
  The later rule still wins; the warning is informational. (Same
  file isn't warned — that's authored-on-purpose source order.)

- Tests:
  - Golden-file: A1 mini + Textured PEI + PLA in slot 0 → expected
    resolved map (~50 keys), each with `winning_rule` source-location
    assertions.
  - Specificity ladder: default → filament rule → plate rule →
    filament+plate rule. Confirm the highest-specificity rule wins
    for every overlapping key.
  - Source-order tie: two rules `when.filament.type = "PLA"`
    setting `nozzle_temperature`, latter wins.
  - Cross-file tie warning: load two cascade files where each has
    a rule for the same key + same specificity. Capture
    `tracing` output, assert the warning fired.
  - Property tests (if not cut): for any sequence of N rules and
    M overlapping keys, `resolve` is deterministic, never panics,
    and resolved values are always present in some matching rule.

**Effort.** ~4 days. Predicate evaluation + comparison parser is
the bulk; the cascade-tie warning is small.

**Dependencies.** PR-1-1 (schema for predicate validation against
known dimensions), PR-1-2 (input cascade).

**Out of scope.** Override tiers (PR-1-4). Trace tooling for "why
is X = 55?" (PR-1-5) — this ticket only emits the structured
`Resolved` map; the trace formatter is downstream. Dimensional
expansion (PR-1-6).
