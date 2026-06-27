import { useMemo } from "react";
import type { PrinterInstance } from "./printerInstance";
import { usePlugins } from "../plugins/usePlugins";
import { PluginManager } from "../plugins/PluginManager";
import { instancePluginWriters } from "../plugins/pluginWriters";
import { pluginSupportsPrinter } from "../plugins/pluginCascade";

/** Printer-instance plugin tier — the per-printer default that sits just
 *  above Global in the plugin cascade. Mounts the shared <PluginManager> at
 *  the "printer-instance" level, reading/writing the instance's
 *  `config_overrides`. Toggles persist live (each fires a backend command);
 *  the `printer:instance_changed` event refreshes the `instance` prop so the
 *  rows reflect the new state. Project/plate aren't in scope here. */
export function PluginsSection({
  instance,
  printerModel,
}: {
  instance: PrinterInstance;
  /** This printer's model, for compatibility filtering. */
  printerModel: string | null;
}): React.JSX.Element {
  const { plugins } = usePlugins();
  // Only plugins compatible with this printer — a U1's list omits an
  // A1-mini-only plugin like platecycler.
  const compatible = useMemo(
    () => plugins.filter((p) => pluginSupportsPrinter(p, printerModel)),
    [plugins, printerModel],
  );
  const sources = useMemo(
    () => ({
      instanceOverrides: instance.config_overrides,
      projectOverrides: {},
    }),
    [instance.config_overrides],
  );
  const writers = useMemo(
    () => instancePluginWriters(instance.id),
    [instance.id],
  );
  return (
    <div className="psm-section">
      <PluginManager
        level="printer-instance"
        plugins={compatible}
        sources={sources}
        writers={writers}
      />
    </div>
  );
}
