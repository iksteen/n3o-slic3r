// `usePrinterInstances` — live list of registered `PrinterInstance`s.
//
// Used by App.tsx to decide whether to render the empty-state
// onboarding (when the list is empty) and by `PrinterPicker` to
// populate its dropdown.
//
// Refreshes on `printer:instance_changed` so creates/deletes/bed
// edits propagate without callers wiring their own listeners.

import { useEffect, useState } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { listPrinterInstances, type PrinterInstance } from "./printerInstance";

export interface PrinterInstancesState {
  /** All registered instances in declaration order. Empty before
   *  the first fetch resolves; the empty array is a sentinel for
   *  "no printers yet" only after `loading` flips to false. */
  instances: PrinterInstance[];
  loading: boolean;
  error: string | null;
}

export function usePrinterInstances(): PrinterInstancesState {
  const [instances, setInstances] = useState<PrinterInstance[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    let unlisten: UnlistenFn | null = null;

    const fetch = (): void => {
      void listPrinterInstances()
        .then((list) => {
          if (!cancelled) {
            setInstances(list);
            setLoading(false);
            setError(null);
          }
        })
        .catch((err) => {
          if (!cancelled) {
            setError(String(err));
            setLoading(false);
          }
        });
    };

    fetch();
    void listen("printer:instance_changed", () => fetch()).then((u) => {
      if (cancelled) u();
      else unlisten = u;
    });

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  return { instances, loading, error };
}
