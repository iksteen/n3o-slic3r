// `usePrinterCatalog` — the bundled printer catalog.
//
// State-layer: a query with no invalidation event — the catalog is static
// bundled data, so it fetches once and the query cache returns the cached list
// to every later caller (replacing this hook's hand-rolled module promise).

import { printerCatalog, type PrinterCatalogEntry } from "./printerCommands";
import { defineQuery, useQuery } from "../state/queryCache";

/** Stable empty reference for the pre-first-fetch window. */
const NO_ENTRIES: PrinterCatalogEntry[] = [];

const printerCatalogQuery = defineQuery<PrinterCatalogEntry[]>({
  key: "printer_catalog",
  fetch: () => printerCatalog(),
  invalidateOn: [],
});

export interface CatalogState {
  entries: PrinterCatalogEntry[];
  loading: boolean;
  error: string | null;
}

export function usePrinterCatalog(): CatalogState {
  const { data, loading, error } = useQuery(printerCatalogQuery);
  return { entries: data ?? NO_ENTRIES, loading, error };
}
