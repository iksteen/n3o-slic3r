# PR-3-7 — G-code serializer (byte-equivalent round-trip)

Status: ❌ open.

**Scope.** Inverse of PR-3-6's parser: given a sequence of typed
`Line` values, emit G-code byte-for-byte identical to what the
parser was fed. The round-trip equality is the project's
**independent oracle** for the slice loop (Execution Plan §5 exit
criteria); we use it instead of needing a reference slicer.

Lives in `core/gcode/serializer.rs`.

**Acceptance criteria.**

- `pub fn write_lines<W: Write>(lines: &[Line], out: W) ->
  io::Result<()>` and `pub fn to_string(lines: &[Line]) -> String`.

- Byte-equivalent round-trip: for every spike fixture under
  `examples/spike{1,2,3}/*.gcode`, the round-trip
  `to_string(parse_str(input))` is byte-for-byte equal to `input`
  (modulo a documented set of normalizations — see below).

- Allowed normalizations (must be empty for the Phase 3 exit
  smoke):
  - **None for MVP.** If the round-trip turns up a discrepancy
    the parser legitimately cannot represent (e.g., trailing
    whitespace, mixed line endings), enumerate it inline in the
    ticket PR description and document the trade-off. Don't
    silently swallow.

- `Line::Other` re-emits the exact raw bytes captured by the
  parser. `Line::Comment` re-emits using the original string
  preserved on the model — the `SemanticComment` tag is *only* an
  inspector, never the round-trip source.

- Move emission order matches the input parameter order. PR-3-5's
  `Move` carries the original raw_offset; if two moves have
  different parameter orderings, the serializer must respect the
  per-move ordering captured at parse time. (Implementation hint:
  capture a `param_order: SmallVec<[char; 6]>` on `Move` during
  PR-3-6's parse, consumed here.)

- Tests:
  - Round-trip every spike fixture; assert exact byte equality
    via `sha256` of input vs round-tripped output.
  - Hand-crafted edge cases: `G1 F100 X10 Y10` vs `G1 X10 Y10
    F100` (different parameter order, same semantics) — both
    re-emit in their original order.
  - Synthetic 50 MB fixture: round-trip through parser+serializer
    must complete in < 5 s on the dev rig (a budget separate from
    PR-3-6's parse-only 3 s).

- Document one fact prominently in the rustdoc: **byte-equivalence
  is load-bearing for the test oracle.** A "small" loss
  (whitespace, comment formatting) seems harmless but breaks the
  validation strategy that lets us ship without a reference
  slicer. If a discrepancy is genuinely unavoidable, the model
  needs to expand to preserve it, not the serializer needs to
  smooth it over.

**Effort.** ~2 days. The mechanical write paths are ~1 day; the
parameter-order preservation and the round-trip test harness for
the spike fixtures is the rest.

**Dependencies.** PR-3-5 (model), PR-3-6 (parser, so the
round-trip test has something to compare).

**Out of scope.** Pretty-printing or canonicalizing the G-code —
the round-trip is *not* a normalizer. The serializer must produce
exactly what the parser consumed, byte-for-byte. Phase 8's plugin
hooks may legitimately mutate the model (insert layers, change
extrusion); those mutations break the round-trip on purpose and
exit through a different test path.
