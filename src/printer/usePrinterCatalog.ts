// `usePrinterCatalog` — fetches + caches the bundled printer
// catalog (PR-5-4). The catalog is static data; refetching after
// the first mount serves no purpose, so this is a one-shot fetch
// that returns the cached list to every subsequent caller.

import { useEffect, useState } from "react";
import {
  printerCatalog,
  type PrinterCatalogEntry,
} from "./printerCommands";

let cachedPromise: Promise<PrinterCatalogEntry[]> | null = null;

function loadCatalog(): Promise<PrinterCatalogEntry[]> {
  if (cachedPromise === null) {
    cachedPromise = printerCatalog().catch((err) => {
      // Reset on failure so a transient hiccup doesn't poison
      // subsequent mounts.
      cachedPromise = null;
      throw err;
    });
  }
  return cachedPromise;
}

export interface CatalogState {
  entries: PrinterCatalogEntry[];
  loading: boolean;
  error: string | null;
}

export function usePrinterCatalog(): CatalogState {
  const [entries, setEntries] = useState<PrinterCatalogEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let mounted = true;
    setLoading(true);
    loadCatalog()
      .then((list) => {
        if (mounted) {
          setEntries(list);
          setLoading(false);
        }
      })
      .catch((err) => {
        if (mounted) {
          setError(String(err));
          setLoading(false);
        }
      });
    return () => {
      mounted = false;
    };
  }, []);

  return { entries, loading, error };
}
