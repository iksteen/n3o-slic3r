# PR-1-10 — Resolver benchmarks

Status: ✅ done. `src-tauri/tests/cascade_perf.rs` ships three perf gates as plain `#[test]`s — kept inside `cargo test --release` rather than wired through `criterion`, so CI gets the regression gate for free. Each test runs N=100 iterations with warm-up, asserts mean latency under the FR-CAS-11 budget: resolve A1 mini PLA/PEI < 10 ms; resolve 4-slot synthetic < 15 ms; resolve + adapter expansion < 100 ms. Today's small reference cascade resolves in microseconds — the budget is a regression gate, not a real target. If Phase 4's UI surfaces the resolver as a hot path, swap in criterion for statistical-quality numbers.

**Scope.** Performance budget for the resolver + adapter pipeline,
per FR-CAS-11: full 4-slot resolution under 10 ms, plus adapter
expansion under 100 ms total. Establishes a regression guard via
`criterion` benches that run in CI.

**Acceptance criteria.**

- `crates/n3o-slic3r/benches/cascade.rs` (or `src-tauri/benches/`)
  benches:
  - `resolve_a1_mini_pla_pei` — A1 mini + Textured PEI + PLA in
    slot 0; full resolver pipeline (cascade load amortized to
    setup, then `resolve` per iteration).
  - `resolve_u1_multi_filament` — U1 + PEI + PLA slot 0 + PETG
    slot 1; tests per-slot resolution overhead.
  - `adapt_a1_mini_pla_pei` — resolver + adapter end-to-end.

- Each bench documents its budget in a `// budget: <ms>` comment
  and the bench code `assert!(elapsed < budget)` so a regression
  fails the bench rather than silently slowing down.

- Budget targets:
  - `resolve_a1_mini_pla_pei` < 10 ms (FR-CAS-11 target).
  - `resolve_u1_multi_filament` < 15 ms (4 slots = 4 resolves).
  - `adapt_a1_mini_pla_pei` < 100 ms (FR-CAS-11 secondary target).

- A separate CI job runs the benches in `--profile release` and
  surfaces results in the PR check summary (cargo-criterion
  output suffices for v1; a comparison-to-main mode is
  nice-to-have for later).

- Note: CI's `cargo test --workspace --release` doesn't include
  benches by default; the bench job is a separate `cargo bench`
  invocation. Add it as a non-blocking CI step initially (passes
  if benches complete; fails only on the `assert!` budget
  violations).

**Effort.** ~1.5 days.

**Dependencies.** PR-1-3 (resolver), PR-1-6 (adapter), PR-1-8
(reference profiles to bench against).

**Out of scope.** Memory profiling. Multi-threaded resolver
(today's resolver is single-threaded; if benchmarks reveal a
threading opportunity, defer to a future ticket). Bench-results
historical tracking across commits.
