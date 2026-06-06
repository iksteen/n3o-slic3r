# PR-1-2 — TOML cascade loader + parser + load-time validation

Status: ✅ done. `src-tauri/src/core/cascade/{types,loader,validate}.rs` parses all three syntactic forms (top-level keys, `[section.shorthand]`, `[[rule]]`) into a typed IR and validates `set.*` keys against the PR-1-1 schema with Levenshtein-distance suggestions. 18 unit tests (12 loader + 6 validate). Predicate-dimension validation accepts a `KnownDimensions` parameter for future PR-1-7 integration; `default_known_dimensions()` provides the canonical set for tests + early consumers.

**Scope.** Production cascade loader living in
`src-tauri/src/core/cascade/`. Parses TOML cascade files (one or
many) into an intermediate representation the resolver consumes.
Supports the three equivalent forms documented in
`docs/dev/profiles.md` "Syntax — three equivalent forms": top-level
keys for unconditional defaults, `[[rule]]` blocks for explicit
rules, and `[section.shorthand]` for single-condition rules.

Validates everything that can be checked statically at load time:
predicate dimensions and `set.*` keys must be in PR-1-1's schema;
scope-compatibility checks (object-scoped keys must appear in
object-applicable contexts only). Errors carry file:line attribution.

**Acceptance criteria.**

- `pub fn load_cascade(paths: &[&Path]) -> Result<Cascade,
  CascadeLoadError>` parses one or more cascade files in load
  order. Top-level keys from the first file become the
  specificity-0 default rule at source-position 0; subsequent
  files' top-level keys append at later source positions.

- All three syntactic forms desugar to the same in-memory `Rule`
  shape: `{ when: Predicate, set: BTreeMap<String, String>,
  source: SourceLocation }`. The desugaring is implemented as
  documented in `docs/dev/profiles.md` — top-level keys at source
  position 0; section headers (`[filament.type.PLA]`) at the
  position they appeared; `[[rule]]` blocks the same way.

- Load-time validation errors:
  - **Unknown predicate dimension** (`when.printr.model = ...`):
    "unknown predicate dimension 'printr' at file.toml:12"
  - **Unknown set key** (`set.layer_hieght = "0.2"`): "unknown
    option 'layer_hieght' at file.toml:14 (did you mean
    'layer_height'?)" — fuzzy suggestion is nice-to-have, exact
    match is required.
  - **Scope violation**: object-scoped settings (e.g.
    `support_filament`) in a rule whose `when` only carries
    print-scope dimensions (e.g. `when.printer.model = "A1 mini"`)
    raises a scope-violation error per FR-CAS-12 / FR-CAS-13.
  - **Cycle in inheritance / include**: not in the format today —
    flag for future-proofing.

- All errors carry a `SourceLocation { path, line, column }` and
  pretty-print with `rustc`-style annotations (file:line + caret).

- Tests:
  - Round-trip each syntactic form (top-level / section / `[[rule]]`)
    through the parser and confirm identical `Rule` lists.
  - Mixed-form input (one file with all three) parses into the
    expected source-order sequence.
  - Each validation error class has a test asserting both the
    error variant and the rendered message.

**Effort.** ~3 days.

**Dependencies.** PR-1-1 (schema is the validator's source of
truth).

**Out of scope.** Override-tier loading (PR-1-4 — user profile +
project file have a different shape). Predicate evaluation (PR-1-3).
Includes / imports across cascade files (not in the design today;
multi-file load just appends in argument order). YAML or JSON as
alternate formats (TOML only per `docs/dev/profiles.md`).
