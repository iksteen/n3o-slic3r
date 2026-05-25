// Tauri bridge for the Phase 4 settings panel (PR-4-4).
//
// Wraps `cascade_resolve` + `slicer_options_for_printer` with the
// `invoke()` plumbing the panel needs at the React boundary. The
// resolve hook caches per (handle, context) tuple to keep cascade
// re-renders cheap.

import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { PrinterAwareOptionSummary } from "./types";

/** Mirror of `core::cascade::commands::ContextJson` on the wire.
 *  Same shape the slice flow already builds.
 *
 *  PR-5-7: `object_overrides` is the per-object cascade-tier map.
 *  When the panel's Object tab is active, the host passes the
 *  selected object's authored overrides here; otherwise the map
 *  is empty. The resolver applies them as the highest-priority
 *  tier (Object > Project > User > Cascade). */
export type ContextJson = {
  printer: PrinterProfileJson;
  plate: BuildPlateJson;
  filaments: FilamentProfileJson[];
  active_slot: number;
  user_overrides: OverrideFileSpec[];
  project_overrides: OverrideFileSpec[];
  object_overrides: Record<string, string>;
};

export type PrinterProfileJson = {
  model: string;
  slot_count: number;
  supported_build_plates: string[];
  toolheads: {
    nozzle_diameter: number;
    hotend_type: string;
    max_temp: number;
    slot_indices: number[];
  }[];
  build_volume: { min: [number, number, number]; max: [number, number, number] };
  exclusion_zones: { min: [number, number, number]; max: [number, number, number] }[];
};

export type BuildPlateJson = {
  identity: string;
  libslic3r_curr_bed_type: string;
};

export type FilamentProfileJson = {
  identity: string;
  base_type: string;
  vendor: string | null;
  color: string | null;
};

export type OverrideFileSpec = { label: string; content: string };

/** Per-key cascade resolution from `cascade_resolve`. */
export type ResolvedEntry = {
  value: string;
  winning_specificity: number;
  cascade_fallback: string | null;
};

export type ResolvedMap = Record<string, ResolvedEntry>;

/** Fetch the printer-aware option list. Cached per printer model
 *  + slot_count + toolhead count since those are the inputs the
 *  capability predicates read; switching printers with the same
 *  capability shape would still trigger a refetch via the
 *  composite key but the result is identical. */
export function usePrinterOptions(
  printer: PrinterProfileJson | null,
): { options: PrinterAwareOptionSummary[]; loading: boolean } {
  const [options, setOptions] = useState<PrinterAwareOptionSummary[]>([]);
  const [loading, setLoading] = useState(true);
  // Reduce the printer to a stable JSON key so React's effect-deps
  // don't refire on referentially-different but value-identical
  // PrinterProfile objects.
  const key = useMemo(() => JSON.stringify(printer ?? null), [printer]);
  useEffect(() => {
    if (printer == null) {
      setOptions([]);
      setLoading(false);
      return;
    }
    let cancelled = false;
    setLoading(true);
    invoke<PrinterAwareOptionSummary[]>("slicer_options_for_printer", {
      printer,
      filter: null,
    })
      .then((opts) => {
        if (!cancelled) {
          setOptions(opts);
          setLoading(false);
        }
      })
      .catch((err) => {
        console.error("[settings] slicer_options_for_printer failed", err);
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
    // We re-key by the JSON serialization above to dedupe.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key]);
  return { options, loading };
}

/** Run `cascade_resolve` against the current handle + context. */
export function useCascadeResolve(
  handle: number | null,
  context: ContextJson | null,
): { resolved: ResolvedMap; loading: boolean; error: string | null } {
  const [resolved, setResolved] = useState<ResolvedMap>({});
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Same JSON-key trick as usePrinterOptions — the context object
  // is rebuilt per render in the parent.
  const key = useMemo(() => JSON.stringify({ handle, context }), [handle, context]);

  useEffect(() => {
    if (handle == null || context == null) {
      setResolved({});
      setLoading(false);
      setError(null);
      return;
    }
    let cancelled = false;
    setLoading(true);
    setError(null);
    invoke<{ entries: ResolvedMap }>("cascade_resolve", { handle, context })
      .then((res) => {
        if (!cancelled) {
          setResolved(res.entries);
          setLoading(false);
        }
      })
      .catch((err) => {
        if (!cancelled) {
          setError(String(err));
          setLoading(false);
        }
      });
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key]);

  return { resolved, loading, error };
}
