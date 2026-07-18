// Tauri bridge for the settings panel.
//
// Wraps `cascade_resolve` + `slicer_options_for_printer` with the
// `invoke()` plumbing the panel needs at the React boundary. The
// resolve hook caches per (handle, context) tuple to keep cascade
// re-renders cheap.

import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { PrinterAwareOptionSummary } from "./types";

export type PrinterProfileJson = {
  model: string;
  /** Manufacturer name ("Bambu Lab", "Snapmaker"). Drives the
   * add-printer modal's brand grouping + brand-tinted cards.
   * `""` when the profile predates the brand field. */
  brand: string;
  /** Short brand glyph for cards/chips ("B" for Bambu Lab). */
  brand_short: string;
  /** Maximum number of AMS-style swap units this printer accepts.
   * `0` means no AMS support (direct-feed only / toolchanger). */
  ams_max: number;
  /** User-visible AMS family name ("AMS Lite", "AMS 2 Pro").
   * `null` when ams_max is 0. */
  ams_type: string | null;
  /** Filament slots one AMS unit holds (4 for AMS Lite / AMS). The
   * AMS picker previews slot counts off this instead of hardcoding. */
  ams_slots_per_unit: number;
  supported_build_plates: string[];
  /** Nozzle diameters the printer ships per-nozzle fragments for
   *  (e.g. `["0.2", "0.4", "0.6", "0.8"]` for the A1 mini).
   *  String symbols — see [NozzleSku.diameter] for why. */
  available_nozzle_diameters: string[];
  /** Default `curr_bed_type` enum value the upstream Orca profile
   * declares for this printer. Frontend uses this when displaying
   * "the canonical default" hint; create_instance on the backend
   * uses it to seed a fresh PrinterInstance's bed. `null` for
   * legacy profiles that omit the field. */
  default_bed: string | null;
  toolheads: {
    default_nozzle_diameter: string;
    hotend_type: string;
    max_temp: number;
  }[];
  build_volume: { min: [number, number, number]; max: [number, number, number] };
  exclusion_zones: { min: [number, number, number]; max: [number, number, number] }[];
  /** Which driver implementation talks to this printer (`null` when
   *  n3o ships no driver for it). Authored in each printer's
   *  `model.toml` (`driver_kind = "bambu" | "u1"`). Drives the
   *  picker's Connection-tab visibility. */
  driver_kind: "bambu" | "u1" | null;
};

/** Per-key cascade resolution from `plate_cascade_resolve`. The value
 *  is the cascade-resolved value (fragments only — override tiers are
 *  drawn from the panel's own override maps), `source_layer` is the
 *  cascade `CascadeLayer` id it won from (drives the ladder row it lands
 *  in), and `cascade_fallback` is unused by this path (always null —
 *  there's no override folded in to revert from). */
type ResolvedEntry = {
  value: string;
  source_layer: string | null;
  cascade_fallback: string | null;
};

export type ResolvedMap = Record<string, ResolvedEntry>;

/** Fetch the printer-aware option list. Cached per printer model
 *  + toolhead count since those are the inputs the
 *  capability predicates read; switching printers with the same
 *  capability shape would still trigger a refetch via the
 *  composite key but the result is identical. */
export function usePrinterOptions(
  printer: PrinterProfileJson | null,
): { options: PrinterAwareOptionSummary[]; loading: boolean } {
  return usePrinterAwareOptions("slicer_options_for_printer", printer);
}

/** The printer-bucket ("machine settings") analogue of
 *  [`usePrinterOptions`] — same shape, sourced from
 *  `slicer_machine_options_for_printer`. */
export function useMachineOptions(
  printer: PrinterProfileJson | null,
): { options: PrinterAwareOptionSummary[]; loading: boolean } {
  return usePrinterAwareOptions("slicer_machine_options_for_printer", printer);
}

/** Per-extruder Printer-bucket options (one value per toolhead), for the
 *  printer panel's per-extruder tabs. */
export function useExtruderOptions(
  printer: PrinterProfileJson | null,
): { options: PrinterAwareOptionSummary[]; loading: boolean } {
  return usePrinterAwareOptions("slicer_extruder_options_for_printer", printer);
}

/** Filament-bucket options for the filament settings editor. Not
 *  printer-gated (a user filament isn't bound to a printer), so this is a
 *  plain one-shot fetch — the option *set* is static for the session. */
export function useFilamentOptions(): {
  options: PrinterAwareOptionSummary[];
  loading: boolean;
} {
  const [options, setOptions] = useState<PrinterAwareOptionSummary[]>([]);
  const [loading, setLoading] = useState(true);
  useEffect(() => {
    let cancelled = false;
    invoke<PrinterAwareOptionSummary[]>("slicer_filament_options", {
      filter: null,
    })
      .then((opts) => {
        if (!cancelled) {
          setOptions(opts);
          setLoading(false);
        }
      })
      .catch((err) => {
        console.error("[settings] slicer_filament_options failed", err);
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, []);
  return { options, loading };
}

/** Shared fetch for the two printer-aware option commands. Cached per
 *  printer model + toolhead count (the inputs the capability predicates
 *  read), keyed by a stable JSON serialization so referentially-different
 *  but value-identical PrinterProfile objects don't refire the effect. */
function usePrinterAwareOptions(
  command:
    | "slicer_options_for_printer"
    | "slicer_machine_options_for_printer"
    | "slicer_extruder_options_for_printer",
  printer: PrinterProfileJson | null,
): { options: PrinterAwareOptionSummary[]; loading: boolean } {
  const [options, setOptions] = useState<PrinterAwareOptionSummary[]>([]);
  const [loading, setLoading] = useState(true);
  const key = useMemo(() => JSON.stringify(printer ?? null), [printer]);
  useEffect(() => {
    if (printer == null) {
      setOptions([]);
      setLoading(false);
      return;
    }
    let cancelled = false;
    setLoading(true);
    invoke<PrinterAwareOptionSummary[]>(command, { printer, filter: null })
      .then((opts) => {
        if (!cancelled) {
          setOptions(opts);
          setLoading(false);
        }
      })
      .catch((err) => {
        console.error(`[settings] ${command} failed`, err);
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
    // Re-keyed by the JSON serialization above to dedupe.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [command, key]);
  return { options, loading };
}

/** Wire shape of one `plate_cascade_resolve` entry (no
 *  `cascade_fallback` — this path folds in no overrides). */
type PlateResolvedEntryWire = { value: string; source_layer: string | null };

/** Resolve a plate's cascade for the settings panel via the backend
 *  `plate_cascade_resolve` command: the bound instance's fragments,
 *  composed against the plate's effective process, each value tagged
 *  with the layer it won from. Refetches when `plateId` or `dep`
 *  changes — pass a `dep` derived from the plate's process + binding
 *  (e.g. `quality_profile|printer_instance_id`) so a process switch or
 *  rebind re-resolves. Returns `{}` for a null plate. */
export function usePlateCascadeResolve(
  plateId: number | null,
  dep: string,
): { resolved: ResolvedMap; loading: boolean; error: string | null } {
  const [resolved, setResolved] = useState<ResolvedMap>({});
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (plateId == null) {
      setResolved({});
      setLoading(false);
      setError(null);
      return;
    }
    let cancelled = false;
    setLoading(true);
    setError(null);
    invoke<{ entries: Record<string, PlateResolvedEntryWire> }>(
      "plate_cascade_resolve",
      { plateId },
    )
      .then((res) => {
        if (cancelled) return;
        const out: ResolvedMap = {};
        for (const [k, v] of Object.entries(res.entries)) {
          out[k] = {
            value: v.value,
            source_layer: v.source_layer,
            cascade_fallback: null,
          };
        }
        setResolved(out);
        setLoading(false);
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
    // `dep` captures the plate state the backend resolve depends on.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [plateId, dep]);

  return { resolved, loading, error };
}
