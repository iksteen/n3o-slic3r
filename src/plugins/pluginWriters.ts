// Level-specific `PluginWriters` for the three plugin surfaces.
//
// Each surface mounts <PluginManager> with writers bound to the right
// backend tier:
//   - global           → plugin_set_global_enabled / _setting
//   - printer-instance → PrinterInstance.config_overrides
//                        (printer_instance_set_plugin_override)
//   - project          → Project.user_overrides (scene_user_override_*)
//   - plate            → Plate.project_overrides (scene_project_override_*)
//
// Activation "inherit" (undefined) clears the override at the
// project/plate tiers; the global root is binary so inherit can't occur
// there. Setting values store as the flat string vocabulary at the
// override tiers, and as typed scalars at the global tier (the command
// types them per the manifest).

import type { PlateId } from "../viewport/types";
import type { PluginWriters } from "./PluginManager";
import { enabledKey, serializeSettingValue, settingKey } from "./pluginCascade";
import * as cmd from "./pluginCommands";

export function globalPluginWriters(): PluginWriters {
  return {
    setActivation: (plugin, value) => {
      // Global is the binary root; "inherit" never reaches here.
      if (value !== undefined) cmd.setGlobalEnabled(plugin.name, value === "on");
    },
    setSetting: (plugin, setting, value) =>
      cmd.setGlobalSetting(plugin.name, setting.key, value),
    clearSettings: () => {
      // The global tier IS the baseline — clearing to "inherit" has no
      // meaning; the manager hides the reset button at the root.
    },
    reload: (plugin) => cmd.reloadPlugin(plugin.name),
  };
}

export function instancePluginWriters(instanceId: string): PluginWriters {
  return {
    setActivation: (plugin, value) => {
      const key = enabledKey(plugin.name);
      if (value === undefined) cmd.clearInstanceOverride(instanceId, key);
      else cmd.setInstanceOverride(instanceId, key, value === "on" ? "true" : "false");
    },
    setSetting: (plugin, setting, value) =>
      cmd.setInstanceOverride(
        instanceId,
        settingKey(plugin.name, setting.key),
        serializeSettingValue(value),
      ),
    clearSettings: (plugin) => {
      for (const s of plugin.settings)
        cmd.clearInstanceOverride(instanceId, settingKey(plugin.name, s.key));
    },
    reload: (plugin) => cmd.reloadPlugin(plugin.name),
  };
}

export function projectPluginWriters(): PluginWriters {
  return {
    setActivation: (plugin, value) => {
      const key = enabledKey(plugin.name);
      if (value === undefined) cmd.clearUserOverride(key);
      else cmd.setUserOverride(key, value === "on" ? "true" : "false");
    },
    setSetting: (plugin, setting, value) =>
      cmd.setUserOverride(settingKey(plugin.name, setting.key), serializeSettingValue(value)),
    clearSettings: (plugin) => {
      for (const s of plugin.settings) cmd.clearUserOverride(settingKey(plugin.name, s.key));
    },
    reload: (plugin) => cmd.reloadPlugin(plugin.name),
  };
}

export function platePluginWriters(plateId: PlateId): PluginWriters {
  return {
    setActivation: (plugin, value) => {
      const key = enabledKey(plugin.name);
      if (value === undefined) cmd.clearProjectOverride(plateId, key);
      else cmd.setProjectOverride(plateId, key, value === "on" ? "true" : "false");
    },
    setSetting: (plugin, setting, value) =>
      cmd.setProjectOverride(plateId, settingKey(plugin.name, setting.key), serializeSettingValue(value)),
    clearSettings: (plugin) => {
      for (const s of plugin.settings)
        cmd.clearProjectOverride(plateId, settingKey(plugin.name, s.key));
    },
    reload: (plugin) => cmd.reloadPlugin(plugin.name),
  };
}
