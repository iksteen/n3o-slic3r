# PRD Addendum: Seeded Profiles

Quality presets reframed as bundled user-tier profiles

*Post-MVP — not part of MVP scope*

| **Field** | **Value** |
| --- | --- |
| Document version | 0.1 (draft) |
| Status | Post-MVP design; deferred to a later phase |
| Relationship | Extends the main PRD without modifying it |
| Prerequisite | MVP cascade (PRD §6.1, FR-CAS-1 through FR-CAS-17) must be implemented |

# 1. Purpose and scope

Existing slicers (OrcaSlicer, PrusaSlicer, Bambu Studio) ship a category of settings bundles known as 'quality profiles' or 'print presets' — named configurations like '0.2mm Standard' or '0.16mm Quality' that override layer height, wall count, infill density, and related settings. They are hardcoded into the printer profile bundles, conceptually separate from user-saved profiles, and live in a different UI silo.

This addendum reframes quality profiles as seeded user-tier profiles: pre-provided TOML files that are loaded into the same cascade tier as user-created profiles, surfaced in the same UI affordance, and behave identically once selected. The goals are to unify the mental model (one kind of profile, not two), preserve composability (any source — printer profile, nozzle profile, community pack — can ship seeded profiles), and let the existing cascade UX handle the entire workflow without inventing new patterns.

This work is deferred to a phase after the MVP ships. The MVP cascade explicitly supports user-tier profiles (PRD FR-CAS-4); seeded profiles slot into that same mechanism with only schema and UI additions — no resolver changes.

# 2. Concept

## 2.1 Unified profile model

A profile is a flat TOML file with set.* entries, optional metadata for picker filtering, and no when.* predicates. Profiles load into the user !important tier of the cascade. There is no distinction in the resolver between a profile that shipped with a printer profile bundle and a profile the user authored — both are flat unconditional sets at the same tier.

What distinguishes them is provenance and editability, not behavior. Seeded profiles are read-only on disk (they ship with a bundle and can be re-extracted from the bundle if deleted). User profiles are read-write in the user's profile directory. Both appear in the same UI picker, in clearly labeled sections.

## 2.2 Terminology change

The term 'quality profile' is retired. These profiles change layer height, wall count, infill, top/bottom layers, and speeds — not just 'quality.' The replacement vocabulary:

- **Profile.** A flat user-tier TOML file with set.* entries. The unit of saved configuration.

- **Seeded profile.** A profile that ships bundled with a printer profile, nozzle profile, filament profile, or community pack. Read-only by default.

- **User profile.** A profile created by the user via the UI's 'Save as...' action. Stored in the user's profile directory. Read-write.

Existing slicer users will recognize the 'preset' terminology from OrcaSlicer/PrusaSlicer; the new vocabulary uniformly says 'profile' and trusts users to recognize that profiles can come from multiple sources.

## 2.3 Why seeded profiles, not a separate tier

Other slicers treat quality presets as a distinct concept with its own UI, its own storage location, and rules about how it composes with filament presets and user overrides. The result is well-known friction: users can't delete or rename a quality preset, can't shadow it with a personal copy of the same name, and end up with 'Standard (modified)' as the most common UI state — visually meaningless and conceptually murky.

Treating quality presets as seeded user profiles eliminates this friction. There is exactly one tier in the cascade for user-selected configuration bundles; everything else falls out of the existing cascade mechanics.

# 3. Data model

## 3.1 Profile file shape

A profile is a TOML file with three sections:

name = "0.2mm Strength"

description = "Stronger parts at standard print speed. Increases walls and infill."

compatible_with = { "nozzle.diameter" = 0.4, "layer_height_range" = [0.16, 0.24] }

set.layer_height = 0.2

set.wall_loops = 4

set.sparse_infill_density = 25

set.top_shell_layers = 5

set.bottom_shell_layers = 5

The shape is the same for seeded and user profiles. The only difference is where the file lives and whether the app considers it writable. compatible_with is optional; profiles without it are always available in the picker.

## 3.2 compatible_with semantics

compatible_with is a UI-availability filter — it controls picker visibility, not cascade resolution. It declares the context dimensions under which a profile is considered relevant; the picker hides profiles whose compatible_with doesn't match the current configuration.

Critically, compatible_with is NOT a when.* predicate. when.* participates in rule resolution by contributing specificity. compatible_with does no such thing — once a profile is selected, its set.* entries apply unconditionally to the user tier, regardless of compatible_with. The two mechanisms share a similar shape (predicates over context dimensions) but live in entirely separate stages of the pipeline.

The name 'compatible_with' was chosen deliberately to avoid the verb tense in 'applies_when,' which would imply a temporal binding between predicate match and value application. There is no such binding. The profile is metadata-tagged with the contexts it is compatible with; the picker uses that tag to filter; the user makes the selection; the cascade does the rest.

This vocabulary aligns with OrcaSlicer/PrusaSlicer's existing compatible_printers and compatible_prints conventions, which makes profile migration from those slicers conceptually straightforward.

## 3.3 Storage layout

Seeded profiles ship inside the profile bundle they belong to. Possible bundle locations:

- **Inside a printer profile.** Profiles that depend on the printer's overall capabilities (e.g. a draft mode that uses higher print speeds the printer can actually achieve).

- **Inside a nozzle profile.** Profiles tied to a specific nozzle size. '0.2mm Strength' only makes sense paired with a 0.4mm nozzle.

- **Inside a filament profile.** Less common, but a high-flow filament might ship with an 'extra fast' profile that exploits its flow characteristics.

- **As a standalone bundle.** A community-distributed pack of profiles for a specific use case ('Voron 2.4 tuned setups').

User profiles live in a dedicated user-profile directory in the app's data folder. The directory structure mirrors how the UI groups them (per-printer, per-nozzle, etc.) but the exact layout is an implementation detail.

## 3.4 Naming and shadowing

Profile names are not globally unique. A user can save a profile with the same name as a seeded profile. When both are loaded into the picker, the user profile appears in the 'My profiles' section and the seeded one in 'Built-in,' visually disambiguated by section header rather than by name munging.

When the user selects a profile in the picker by name, the selection is unambiguous because they pick from a labeled section. There is no precedence rule needed: the resolver doesn't care whether the active profile is seeded or user-authored; it just loads the named profile from wherever the picker said it lives. 'Save as...' with an existing user profile's name overwrites that profile (with confirmation). 'Save as...' with a seeded profile's name creates a new user profile that shadows it in the dropdown.

# 4. UX behavior

## 4.1 Profile picker

A single picker control surfaces all available profiles for the current context. The picker filters by compatible_with against the currently configured printer, nozzle, filament binding, etc. Profiles are grouped into sections:

- Built-in — seeded profiles from currently loaded bundles. Read-only.

- My profiles — user-created profiles. Right-click to rename, delete, duplicate.

- Recently used — top 3–5 recently selected profiles for quick switching.

Profiles without a compatible_with match are hidden by default. A 'Show all' toggle reveals hidden profiles for power users who want to apply something unusual; selecting one issues a non-blocking warning explaining the mismatch.

## 4.2 Selection and editing

- **Select a profile.** Its set.* entries load into the user-tier of the cascade. The settings panel updates immediately.

- **Edit a setting after selection.** The change lands in the project tier (PRD FR-CAS-5), not the profile itself. The profile remains untouched on disk.

- **Picker indicator.** When project-tier overrides exist alongside an active profile, the picker shows the profile name followed by a small badge ('modified', or a dot indicator with hover-text). This is distinct from PrusaSlicer/Orca's '(modified)' suffix in that the modification lives in the project, not in a phantom shadow of the profile.

- **Save as...** Bundles the currently effective user-tier and project-tier values into a new user profile in the user's directory. Project-tier overrides are absorbed into the new profile; the project tier returns to empty.

- **Save over.** Overwrites an existing user profile (with confirmation). Disabled for seeded profiles. To 'edit' a seeded profile, the user does 'Save as...' under the same or different name; the resulting user profile shadows or coexists with the seeded one.

- **Reset project.** Clears the project tier. The cascade falls back to the selected profile + authored cascade. Used when 'I tweaked some things for this print but don't want to keep them.'

- **Switch profile.** Replaces the active user-tier profile. Project-tier overrides survive (they are a separate tier). User sees their tweaks persist across profile switches — usually what they want.

## 4.3 Cascade ladder interaction

The cascade ladder (PRD FR-CAS-7) shows the active profile's contribution at the user tier. A setting overridden by the active profile shows 'profile: 0.2mm Strength' at the user tier; if a project override is also active, the ladder shows both, with the project layer winning. Removing the project override (single click in the ladder) drops back to the profile value; removing the profile (selecting a different one in the picker) drops back to the authored cascade.

# 5. Functional requirements

These extend the FR-CAS family in the main PRD. They are not active until this addendum is incorporated into a future phase.

- **FR-PROF-1.** Profile file format: TOML with optional name, description, compatible_with metadata; required set.* entries; no when.* predicates. Identical shape for seeded and user-created profiles.

- **FR-PROF-2.** Seeded profiles ship inside printer, nozzle, filament, or standalone community bundles. The bundle declares its profiles via a seeded_profiles/ directory or equivalent manifest entry. Seeded profiles are read-only at runtime.

- **FR-PROF-3.** User profiles live in the user's profile directory, are read-write, and are created via 'Save as...' actions in the UI.

- **FR-PROF-4.** The profile picker surfaces all available profiles in grouped sections (Built-in / My profiles / Recently used), filtered by compatible_with against current context. A 'Show all' toggle reveals filtered-out profiles with mismatch warnings.

- **FR-PROF-5.** compatible_with semantics: UI-availability filtering only. Does not affect cascade resolution. A selected profile's set.* entries apply unconditionally to the user tier regardless of whether compatible_with currently matches.

- **FR-PROF-6.** Selecting a profile loads its set.* entries into the user tier (PRD FR-CAS-4 mechanism, unchanged). Selecting a different profile replaces the previous selection. The project tier (FR-CAS-5) is unaffected by profile selection.

- **FR-PROF-7.** Save as... bundles the user tier (active profile) plus the project tier into a new user profile, then clears the project tier. The user names the profile; the name need not be unique against seeded profiles.

- **FR-PROF-8.** Save over an existing user profile overwrites it after confirmation. Save over a seeded profile is disabled; the UI offers 'Save as...' instead.

- **FR-PROF-9.** User profiles can be renamed, deleted, duplicated via context menu in the picker. Seeded profiles offer 'Duplicate to my profiles' as an entry point for users who want to customize a seeded starting point.

- **FR-PROF-10.** Cascade ladder (FR-CAS-7) attributes user-tier values to the active profile by name, distinguishing 'profile: 0.2mm Strength' from raw project overrides.

- **FR-PROF-11.** Recently-used tracking: the picker remembers the last 3–5 profiles selected in this app installation, surfaced in a 'Recent' subsection.

# 6. Relationship to the MVP

The MVP ships with user profiles (FR-CAS-4) but no seeded profiles and no picker UI beyond a simple list of user-saved profiles. Power users in the MVP create their own profiles from scratch via Save as... after configuring settings; there is no built-in starting library.

This addendum adds, in a later phase:

- The compatible_with field in the profile schema (extends FR-CAS-4).

- Bundle manifest support for seeded_profiles/ in printer, nozzle, and filament profiles.

- Picker UI with sections and filtering (replaces the MVP's simple profile list).

- Seeded profile content authored for the MVP printers (A1 mini with 0.4mm nozzle; U1 with its toolhead variants).

- The cascade ladder enhancement to attribute user-tier values to named profiles.

Nothing in this addendum requires changes to the cascade resolver, the translation adapter, or the rule mechanism. It is purely additive: a new field in profile files, a new UI pattern, content authoring. The MVP's architecture supports it without modification.

# 7. Open questions for the implementation phase

- compatible_with predicate vocabulary: which context dimensions are filterable? Almost certainly nozzle.diameter, filament.type, printer.id at minimum. Should layer_height_range be a special case (range match, not equality) or are all dimensions equality-only with range encoded in dimension values?

- Seeded profile updates: when a printer-profile bundle ships an updated 0.2mm Strength with different values, do existing projects that selected that profile silently pick up the new values, or pin to the old version? Probably the former for built-in updates, with a notification.

- Profile import from OrcaSlicer / Bambu Studio: their 'preset' bundles are conceptually the same thing. An importer that converts an Orca preset JSON to our TOML profile shape is straightforward and would meaningfully ease migration.

- Filament profiles as seeded profiles: should built-in filaments (Bambu PLA Basic, Generic PLA) use the same seeded-profile mechanism as quality presets, or stay as authored-cascade rules? Current PRD has them as authored-cascade rules (with when.filament.type predicates); arguably they could be either. Worth deciding consistently in the implementation phase.

- Per-project default profile: should a project remember which profile was selected, or always start with no profile selected? Likely the former — the project file stores the active profile name; opening the project restores the selection.

# 8. Out of scope for this addendum

This addendum scopes to seeded profiles as user-tier configuration bundles. Several related ideas are not part of this work:

- A profile marketplace or sharing service. Profiles are files; users can share them by exchanging files. Anything beyond that is a separate product question.

- Profile composition or inheritance ('this profile extends that profile'). The MVP cascade explicitly does not support multi-profile composition; profiles are single, flat units. If composition is needed, it would be a future addendum.

- AI-suggested profiles. Not in scope.

- Per-object profile selection. Profiles apply at the project (or potentially plate) level; per-object setting overrides remain in the per-object tier (FR-3D-3, FR-CAS-3) and are not bundled as profiles.