// Pure cascade helpers for plugin enablement + settings.
//
// No React, no `invoke` — just the read/resolve logic that maps a
// `PluginSummary` plus the three override sources into "what's the
// effective state of this plugin as seen at level X". The components
// render whatever these return; the writers (in PluginManager) push
// changes back to the right backend surface.
//
// THE CASCADE
//   global → project → plate, lower overrides higher. Each level's
//   raw activation is tri-state: "on" / "off" / undefined (inherit).
//   `global` is the root: it's binary (its undefined collapses to the
//   backend's resolved `globally_enabled`, never "inherit").
//
//   Settings overlay ONLY at levels that are explicitly on, coarse →
//   fine, over the manifest defaults. A level set to inherit/off
//   contributes no settings — you can only override a plugin's config
//   where you've deliberately turned it on.

import type { PluginSummary, SettingSummary } from "./pluginCommands";

export type PluginLevel = "global" | "project" | "plate";

export const PLUGIN_LEVEL_ORDER: readonly PluginLevel[] = [
  "global",
  "project",
  "plate",
];

export interface PluginLevelMeta {
  label: string;
  short: string;
  blurb: string;
  hue: number;
}

export const PLUGIN_LEVEL_META: Record<PluginLevel, PluginLevelMeta> = {
  global: { label: "Global", short: "G", blurb: "Every project on this machine", hue: 215 },
  project: { label: "Project", short: "P", blurb: "This .3mf file", hue: 285 },
  plate: { label: "Plate", short: "PL", blurb: "Just the active plate", hue: 340 },
};

/** Raw per-level activation. `undefined` = inherit. */
export type RawActivation = "on" | "off" | undefined;

/** The three override sources, supplied by whatever surface mounts
 *  the manager. `globally_enabled` / `global_settings` come straight
 *  off the `PluginSummary`; the project + plate maps are the relevant
 *  override records (`Project.user_overrides`,
 *  `Plate.project_overrides`). The plate maps are absent on
 *  global / project surfaces. */
export interface CascadeSources {
  /** `SceneSnapshot.user_overrides`. */
  projectOverrides: Record<string, string>;
  /** Active plate's `project_overrides`, or `undefined` when no plate
   *  is in scope (global / project surfaces). */
  plateOverrides?: Record<string, string>;
}

// ── Override key encoding ─────────────────────────────────────────

/** `plugin.<name>.enabled` */
export function enabledKey(name: string): string {
  return `plugin.${name}.enabled`;
}

/** `plugin.<name>.<settingKey>` */
export function settingKey(name: string, key: string): string {
  return `plugin.${name}.${key}`;
}

// ── Level participation ───────────────────────────────────────────

/** The cascade levels a plugin participates in, in cascade order. */
export function pluginLevels(plugin: PluginSummary): PluginLevel[] {
  return PLUGIN_LEVEL_ORDER.filter((l) => plugin.scopes.includes(l));
}

/** Is `level` the plugin's root (the highest level it's available
 *  at)? The root is binary — no "inherit" segment. */
export function isRootLevel(plugin: PluginSummary, level: PluginLevel): boolean {
  return pluginLevels(plugin)[0] === level;
}

// ── Raw reads, per level + source ─────────────────────────────────

function parseEnabledRaw(value: string | undefined): RawActivation {
  if (value === "true") return "on";
  if (value === "false") return "off";
  return undefined;
}

/** Read a level's raw activation from the right source. `global` is
 *  binary: it reports "on"/"off" from `globally_enabled`, never
 *  undefined. */
export function readActivation(
  plugin: PluginSummary,
  level: PluginLevel,
  sources: CascadeSources,
): RawActivation {
  if (level === "global") {
    return plugin.globally_enabled ? "on" : "off";
  }
  const key = enabledKey(plugin.name);
  if (level === "project") {
    return parseEnabledRaw(sources.projectOverrides[key]);
  }
  return parseEnabledRaw(sources.plateOverrides?.[key]);
}

/** Read a level's raw stored value for one setting (string or
 *  undefined). `global` reads the resolved `global_settings`. */
export function readSettingRaw(
  plugin: PluginSummary,
  level: PluginLevel,
  setting: SettingSummary,
  sources: CascadeSources,
): string | undefined {
  if (level === "global") {
    return plugin.global_settings[setting.key];
  }
  const key = settingKey(plugin.name, setting.key);
  if (level === "project") {
    return sources.projectOverrides[key];
  }
  return sources.plateOverrides?.[key];
}

// ── Resolution ────────────────────────────────────────────────────

export interface ResolvedActivation {
  enabled: boolean;
  /** The level that decided it, or "default" (= global's resolved
   *  value when no level above plate carried an explicit choice). */
  source: PluginLevel | "default";
}

/** Resolve a plugin's effective enablement as seen *at* `uptoLevel`,
 *  walking the cascade. Mirrors the mockup's `resolvePlugin`. */
export function resolveActivation(
  plugin: PluginSummary,
  uptoLevel: PluginLevel,
  sources: CascadeSources,
): ResolvedActivation {
  const order = pluginLevels(plugin);
  const cap = PLUGIN_LEVEL_ORDER.indexOf(uptoLevel);
  let enabled = false;
  let source: PluginLevel | "default" = "default";
  order.forEach((lvl, i) => {
    if (PLUGIN_LEVEL_ORDER.indexOf(lvl) > cap) return;
    const raw = readActivation(plugin, lvl, sources);
    if (raw === "on") {
      enabled = true;
      source = lvl;
    } else if (raw === "off") {
      enabled = false;
      source = lvl;
    } else if (i === 0) {
      // Root level, inherit → built-in default. `global` is binary so
      // it never lands here; a non-global root (e.g. plate-only
      // plugin) defaults off.
      enabled = false;
      source = "default";
    }
    // else: inherit from above — carry forward.
  });
  return { enabled, source };
}

/** The deepest level at/above `uptoLevel` where the plugin is
 *  explicitly on — the level that "owns" the effective config. `null`
 *  if none are on. */
export function configOwnerLevel(
  plugin: PluginSummary,
  uptoLevel: PluginLevel,
  sources: CascadeSources,
): PluginLevel | null {
  const order = pluginLevels(plugin);
  const cap = PLUGIN_LEVEL_ORDER.indexOf(uptoLevel);
  let owner: PluginLevel | null = null;
  order.forEach((lvl) => {
    if (
      PLUGIN_LEVEL_ORDER.indexOf(lvl) <= cap &&
      readActivation(plugin, lvl, sources) === "on"
    ) {
      owner = lvl;
    }
  });
  return owner;
}

/** Resolve a plugin's effective settings as seen at `uptoLevel`:
 *  manifest defaults, overlaid by each explicitly-on level's stored
 *  values, coarse → fine. Returns string values (display form);
 *  callers convert via `typedSettingValue` where they need a scalar. */
export function resolveSettings(
  plugin: PluginSummary,
  uptoLevel: PluginLevel,
  sources: CascadeSources,
): Record<string, string> {
  const order = pluginLevels(plugin);
  const cap = PLUGIN_LEVEL_ORDER.indexOf(uptoLevel);
  const result: Record<string, string> = {};
  for (const setting of plugin.settings) {
    result[setting.key] = setting.default;
  }
  order.forEach((lvl) => {
    if (PLUGIN_LEVEL_ORDER.indexOf(lvl) > cap) return;
    if (readActivation(plugin, lvl, sources) !== "on") return;
    for (const setting of plugin.settings) {
      const raw = readSettingRaw(plugin, lvl, setting, sources);
      if (raw !== undefined) result[setting.key] = raw;
    }
  });
  return result;
}

/** Does any level BELOW `level` carry an explicit activation override
 *  for this plugin? Returns that level, or `null`. */
export function downstreamOverride(
  plugin: PluginSummary,
  level: PluginLevel,
  sources: CascadeSources,
): PluginLevel | null {
  const order = pluginLevels(plugin);
  const idx = order.indexOf(level);
  for (let i = idx + 1; i < order.length; i++) {
    const lvl = order[i];
    if (readActivation(plugin, lvl, sources) !== undefined) return lvl;
  }
  return null;
}

/** Count plugins resolved-on at `level` (for the modal footer). */
export function countActiveAtLevel(
  plugins: PluginSummary[],
  level: PluginLevel,
  sources: CascadeSources,
): number {
  return plugins
    .filter((p) => p.scopes.includes(level))
    .filter((p) => resolveActivation(p, level, sources).enabled).length;
}

// ── Typed setting value conversion ────────────────────────────────

export type TypedSettingValue = string | number | boolean;

/** Map a setting's serialized string to a typed JS scalar per its
 *  `kind`. `enum` / `string` stay strings; `number` → number;
 *  `bool` → boolean. */
export function typedSettingValue(
  setting: SettingSummary,
  raw: string,
): TypedSettingValue {
  switch (setting.kind) {
    case "number": {
      const n = Number(raw);
      return Number.isFinite(n) ? n : 0;
    }
    case "bool":
      return raw === "true" || raw === "1";
    case "string":
    case "enum":
      return raw;
  }
}

/** Serialize a typed scalar back to the string form stored in the
 *  override maps + passed to global-setting writes. Booleans become
 *  "true"/"false" to match the `enabled`-key convention. */
export function serializeSettingValue(value: TypedSettingValue): string {
  if (typeof value === "boolean") return value ? "true" : "false";
  return String(value);
}
