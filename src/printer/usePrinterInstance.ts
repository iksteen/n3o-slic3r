// `usePrinterInstance` — the live `PrinterInstance` for one bound id.
//
// A parameterized ("family") query: each instance id is its own cache entry,
// fetched via `printer_instance_get` and refetched on `printer:instance_changed`
// but ONLY when the event's payload names this id (so one printer's edit
// doesn't refetch every other printer's cached instance).
//
// State-layer spike: the settings-panel host and the slot-binding panel both
// pulled the active plate's instance with the same fetch + filtered-listen
// dance. Sharing this family collapses their duplicate fetch of the same id
// into one, and one `printer:instance_changed` into one refetch.

import type { PrinterInstance } from "./printerInstance";
import { getPrinterInstance } from "./printerInstance";
import { defineQuery, useQuery, type QueryDef } from "../state/queryCache";

/** Build the query def for a specific instance id. The id is baked into the
 *  cache key and the invalidation predicate. */
export function printerInstanceQuery(
  id: string,
): QueryDef<PrinterInstance | null> {
  return defineQuery({
    key: `printer_instance:${id}`,
    fetch: () => getPrinterInstance(id),
    invalidateOn: ["printer:instance_changed"],
    shouldInvalidate: (event) => event.payload === id,
  });
}

/** The bound instance, or `null` when no id is bound or before the first
 *  fetch resolves. Mounts the shared family entry for `id`. */
export function usePrinterInstance(id: string | null): PrinterInstance | null {
  return useQuery(id ? printerInstanceQuery(id) : null).data;
}
