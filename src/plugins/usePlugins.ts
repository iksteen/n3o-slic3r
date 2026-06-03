// `usePlugins` — the plugin list, refetched on `plugin:changed`.
//
// State-layer spike: reads the shared `plugins` query instead of running its
// own invoke + listen. This hook has three independent callers (App, the
// printer settings modal, the settings-panel host) — sharing the query
// collapses what was three fetches + three `plugin:changed` listeners into one.

import { useCallback, useEffect } from "react";
import { listPlugins, type PluginSummary } from "./pluginCommands";
import { defineQuery, invalidateQuery, useQuery } from "../state/queryCache";

/** Event the backend fires after any plugin store mutation. */
export const PLUGIN_CHANGED_EVENT = "plugin:changed";

/** Stable empty reference for the pre-first-fetch window. */
const NO_PLUGINS: PluginSummary[] = [];

export const pluginsQuery = defineQuery<PluginSummary[]>({
  key: "plugins",
  fetch: () => listPlugins(),
  invalidateOn: [PLUGIN_CHANGED_EVENT],
});

export interface UsePluginsResult {
  plugins: PluginSummary[];
  /** Imperative re-fetch (e.g. after a reload the caller wants to reflect
   *  immediately). Shared across all consumers via the query cache. */
  reload: () => void;
}

export function usePlugins(): UsePluginsResult {
  const { data } = useQuery(pluginsQuery);
  const reload = useCallback(() => invalidateQuery(pluginsQuery.key), []);
  return { plugins: data ?? NO_PLUGINS, reload };
}

/** Close-on-Escape helper. Stops propagation so a nested modal
 *  doesn't also dismiss an outer surface. */
export function useEscapeKey(onClose: () => void): void {
  useEffect(() => {
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === "Escape") {
        e.stopPropagation();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);
}
