// `usePlugins` — fetches the plugin list on mount + on `plugin:changed`.
//
// Mirrors `useProjectSession`'s listen/unlisten dance: subscribe
// first so a mid-bootstrap event isn't dropped, then do the initial
// fetch. The backend emits `plugin:changed` after any
// enable/setting/reload mutation; we just re-pull the whole list.

import { useCallback, useEffect, useState } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { listPlugins, type PluginSummary } from "./pluginCommands";

/** Event the backend fires after any plugin store mutation. */
export const PLUGIN_CHANGED_EVENT = "plugin:changed";

export interface UsePluginsResult {
  plugins: PluginSummary[];
  /** Imperative re-fetch (e.g. after a reload the caller wants to
   *  reflect immediately). */
  reload: () => void;
}

export function usePlugins(): UsePluginsResult {
  const [plugins, setPlugins] = useState<PluginSummary[]>([]);

  const refetch = useCallback(async () => {
    try {
      const list = await listPlugins();
      setPlugins(list);
    } catch (err) {
      console.error("[plugins] plugin_list failed", err);
    }
  }, []);

  const reload = useCallback(() => {
    void refetch();
  }, [refetch]);

  useEffect(() => {
    let mounted = true;
    let unlisten: UnlistenFn | null = null;

    void (async () => {
      const un = await listen(PLUGIN_CHANGED_EVENT, () => {
        void refetch();
      });
      if (!mounted) {
        un();
        return;
      }
      unlisten = un;
      await refetch();
    })();

    return () => {
      mounted = false;
      if (unlisten) unlisten();
    };
  }, [refetch]);

  return { plugins, reload };
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
