// Plugin cascade — printer-instance tier resolution.
//
// The tier sits global < printer-instance < project < plate. A plate is
// bound to one instance so it inherits the instance's per-printer default;
// the project surface spans printers, so it simply doesn't supply
// `instanceOverrides` (proven by the last case).

import { describe, expect, it } from "vitest";
import {
  resolveActivation,
  enabledKey,
  pluginSupportsPrinter,
} from "../pluginCascade";
import type { PluginSummary } from "../pluginCommands";

function plugin(over: Partial<PluginSummary> = {}): PluginSummary {
  return {
    name: "p",
    version: "1",
    description: null,
    hooks: ["post_slice"],
    printers: null,
    scopes: ["global", "printer-instance", "project", "plate"],
    enabled: true,
    globally_enabled: false,
    enabled_by_default: false,
    settings: [],
    global_settings: {},
    last_error: null,
    ...over,
  };
}

describe("plugin cascade — printer-instance tier", () => {
  it("a plate inherits an instance-level enable (off everywhere else)", () => {
    const r = resolveActivation(plugin(), "plate", {
      instanceOverrides: { [enabledKey("p")]: "true" },
      projectOverrides: {},
    });
    expect(r.enabled).toBe(true);
    expect(r.source).toBe("printer-instance");
  });

  it("project off overrides an instance on", () => {
    const r = resolveActivation(plugin(), "plate", {
      instanceOverrides: { [enabledKey("p")]: "true" },
      projectOverrides: { [enabledKey("p")]: "false" },
    });
    expect(r.enabled).toBe(false);
    expect(r.source).toBe("project");
  });

  it("the project surface (no instanceOverrides) does NOT see the instance tier", () => {
    // Same instance-on intent, but the project surface omits the source —
    // so resolution falls back to the manifest default, not the instance.
    const r = resolveActivation(plugin(), "project", {
      projectOverrides: {},
    });
    // Instance "on" is invisible here, so it falls to global (binary, off).
    expect(r.enabled).toBe(false);
    expect(r.source).toBe("global");
  });
});

describe("pluginSupportsPrinter", () => {
  it("hides a printer-specific plugin from an incompatible printer", () => {
    const p = plugin({ printers: ["Bambu Lab A1 mini"] });
    expect(pluginSupportsPrinter(p, "Bambu Lab A1 mini")).toBe(true);
    expect(pluginSupportsPrinter(p, "Snapmaker U1")).toBe(false);
  });

  it("an any-printer plugin (printers: null) shows everywhere", () => {
    const p = plugin({ printers: null });
    expect(pluginSupportsPrinter(p, "Snapmaker U1")).toBe(true);
    expect(pluginSupportsPrinter(p, null)).toBe(true);
  });

  it("an unknown model keeps the plugin visible", () => {
    expect(pluginSupportsPrinter(plugin({ printers: ["X"] }), null)).toBe(true);
  });
});
