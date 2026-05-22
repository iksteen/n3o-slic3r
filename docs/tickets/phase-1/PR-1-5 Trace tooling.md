# PR-1-5 — Trace tooling

Status: ❌ open.

**Scope.** "Why is `bed_temp` = 55?" — given a resolved key,
return a structured trace covering winner, matching-but-losing
rules, override source (when applicable), and cascade fallback.
Drives FR-CAS-7 (the cascade source badge in the Settings UI's
Phase 4 panel) and the future debug command.

**Acceptance criteria.**

- `pub fn trace(resolved: &ResolvedWithTrace, key: &str) ->
  Option<Trace>` returns:
  ```rust
  pub struct Trace {
      pub key: String,
      pub effective_value: String,
      pub source: TraceSource,           // Cascade | Override
      pub cascade_winner: Option<TraceRule>,    // Some unless absent from cascade
      pub cascade_losers: Vec<TraceRule>,       // empty when only one rule matched
      pub override_source: Option<OverrideTraceEntry>,  // Some when Override
      pub cascade_fallback: Option<String>,     // mirrors override behavior
  }
  pub struct TraceRule {
      pub source: SourceLocation,
      pub specificity: usize,
      pub when_summary: String,    // human-readable, e.g. `filament.type = "PLA" + plate.type = "PEI"`
      pub set_value: String,
  }
  pub struct OverrideTraceEntry {
      pub tier: OverrideTier,
      pub source: SourceLocation,
      pub value: String,
  }
  ```

- `cascade_losers` requires PR-1-3 to retain the full list of
  matching rules during resolution (not just the winner). The
  resolver's intermediate state needs to track this; surface
  through the trace API only.

- Tests:
  - Three rules at specificity 0/1/2 all matching: trace reports
    the spec-2 rule as winner, the spec-1 and spec-0 rules as
    losers, each with correct `set_value` and source location.
  - Override active: `source = Override`, `cascade_fallback`
    matches what the cascade would have resolved to.
  - Override active + key absent from cascade: `cascade_winner =
    None`, `cascade_fallback = None`.
  - Key absent everywhere (typo): `trace(resolved, "lyer_height")
    -> None`.

- Pretty-printer for human consumption — used by the CLI test
  harness in PR-1-11 and the future debug command in PR-1-9:
  ```
  bed_temp = 55 (cascade)
    winner:  spec=2 filament.type = "PLA" + plate.type = "PEI"
             at A1mini-pla-pei.toml:14 → set.bed_temp = 55
    loser:   spec=1 plate.type = "PEI"
             at A1mini-plates.toml:6  → set.bed_temp = 60
    loser:   spec=0 default
             at A1mini-defaults.toml:8 → set.bed_temp = 50

  bed_temp = 50 (project override)
    override: tier=project at user-project.toml:3 → set.bed_temp = 50
    cascade fallback: 55 (same trace as above, suppressed)
  ```

**Effort.** ~2 days. The data structure is straightforward; the
PR-1-3 internal-state changes to retain losers are the larger
cost.

**Dependencies.** PR-1-3 (resolver retains the matching-rules
list), PR-1-4 (override info available in `ResolvedWithTrace`).

**Out of scope.** UI rendering of traces (Phase 4). Per-extruder
trace decomposition (when a vector key resolves differently per
slot) — Phase 5 if it surfaces.

**Cut candidate.** Dropping `cascade_losers` (winner-only trace)
saves ~1 day. Hurts FR-CAS-7 UX but the source badge still works.
