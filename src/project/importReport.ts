import { useEffect } from "react";
import { message as messageDialog } from "@tauri-apps/plugin-dialog";
import { onEvents } from "../state/eventRouter";

// OrcaSlicer / Bambu Studio project import report — the summary the
// backend emits after importing a foreign project via Open project, and
// the human-readable formatting of it. Surfaced in a dialog so lossy
// mapping is never silent.

/** Summary emitted by the backend after importing a foreign
 * (OrcaSlicer / Bambu Studio) project via Open project. */
interface ImportReport {
  objects: number;
  plates: number;
  printer_instance: string | null;
  printer_instance_name: string | null;
  printer_model: string | null;
  printer_fallback: boolean;
  filaments_matched: number;
  filaments_unmatched: number;
  settings_applied: number;
  settings_redundant: number;
  settings_incompatible: number;
  settings_machine_dropped: number;
  settings_filament_dropped: number;
  settings_unmapped: number;
  settings_from_change_list: boolean;
}

/** Build the multi-line body shown in the import-summary dialog. */
function formatImportReport(r: ImportReport): string {
  const lines = [
    `${r.objects} object(s) across ${r.plates} plate(s).`,
    `Printer: ${r.printer_model ?? "?"} → ${r.printer_instance_name ?? "(none)"}${
      r.printer_fallback ? " — no exact match; bound a fallback, rebind if needed" : ""
    }`,
    `Filaments: ${r.filaments_matched} matched, ${r.filaments_unmatched} not found.`,
    `Settings: ${r.settings_applied} applied${
      r.settings_from_change_list
        ? " (only the changes the project made to its preset)"
        : ""
    }${
      r.settings_redundant ? `, ${r.settings_redundant} already at default` : ""
    }${
      r.settings_incompatible ? `, ${r.settings_incompatible} with values this engine doesn't recognize (reset to default)` : ""
    }.`,
    `Not imported: ${r.settings_machine_dropped} printer/machine settings (your printer's own), ${r.settings_filament_dropped} filament settings (taken from the slot each material binds to), ${r.settings_unmapped} settings this engine doesn't support (Bambu Studio extras).`,
  ];
  return lines.join("\n");
}

/** Subscribe to `project:imported` and show the import summary dialog —
 *  a foreign project imported via Open project shows what mapped and what
 *  was dropped. */
export function useImportReportDialog(): void {
  useEffect(() => {
    return onEvents<{ data: { report: ImportReport } }>(
      ["project:imported"],
      (e) => {
        void messageDialog(formatImportReport(e.payload.data.report), {
          title: "Imported from OrcaSlicer / Bambu Studio",
        });
      },
    );
  }, []);
}
