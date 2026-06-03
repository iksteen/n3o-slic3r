// `usePrinterInstances` — live list of registered `PrinterInstance`s.
//
// Used by App.tsx to decide whether to render the empty-state onboarding (when
// the list is empty) and by `PrinterPicker` to populate its dropdown.
//
// State-layer spike: reads the shared `printer_instances` query instead of its
// own invoke + listen. Any other consumer of that query shares the same fetch
// and the `printer:instance_changed` invalidation.

import type { PrinterInstance } from "./printerInstance";
import { listPrinterInstances } from "./printerInstance";
import { defineQuery, useQuery } from "../state/queryCache";

/** Stable empty reference for the pre-first-fetch window, so consumers that
 *  key off the array identity (App → useDriverConnections' snapshot cache)
 *  don't see a fresh `[]` every render while loading. */
const NO_INSTANCES: PrinterInstance[] = [];

export const printerInstancesQuery = defineQuery<PrinterInstance[]>({
  key: "printer_instances",
  fetch: () => listPrinterInstances(),
  invalidateOn: ["printer:instance_changed"],
});

export interface PrinterInstancesState {
  /** All registered instances in declaration order. Empty before the first
   *  fetch resolves; the empty array is a sentinel for "no printers yet" only
   *  after `loading` flips to false. */
  instances: PrinterInstance[];
  loading: boolean;
  error: string | null;
}

export function usePrinterInstances(): PrinterInstancesState {
  const { data, loading, error } = useQuery(printerInstancesQuery);
  return { instances: data ?? NO_INSTANCES, loading, error };
}
