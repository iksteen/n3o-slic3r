// PR-5-9 — Settings-panel host.
//
// Translates the App-level `ProjectSession` (cascade handle, printer
// profile, snapshot) into the SettingsPanel's prop shape, then
// renders the panel. Keeps all the projection logic out of App.tsx
// so the integration surface stays a one-liner.
//
// The host is intentionally thin: every callback either binds
// pre-existing invoke wrappers to the active plate / object, or
// projects a slice of the snapshot. No business logic of its own.

import { useEffect, useMemo, useState } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { ProjectSession } from "../project/useProjectSession";
import { SettingsPanel, type PlateObjectStub } from "./SettingsPanel";
import { buildContextJson } from "./buildContextJson";
import { makeObjectOverrideCallbacks } from "./overrideCommands";
import { makeProjectOverrideCallbacks } from "./projectOverrideCommands";
import { PrinterPicker } from "../printer/PrinterPicker";
import { usePrinterCatalog } from "../printer/usePrinterCatalog";
import {
  getPrinterInstance,
  setExtruderNozzleDiameter,
  setInstanceBed,
  type PrinterInstance,
} from "../printer/printerInstance";
import { SlotBindingPanel } from "../material/SlotBindingPanel";
import { BuildPlateSelector } from "./BuildPlateSelector";
import { NozzlePicker } from "./NozzlePicker";
import { chunkExtruders, nozzlesInline } from "./nozzleLayout";
import type { PlateSnapshot } from "../viewport/types";

/** Locate the active plate in a session snapshot, or `null`
 * when bootstrap hasn't completed. Exported for tests. */
export function activePlate(session: ProjectSession): PlateSnapshot | null {
  if (!session.snapshot) return null;
  return (
    session.snapshot.plates.find(
      (p) => p.plate_id === session.snapshot!.active_plate_id,
    ) ?? null
  );
}

/** First selected object on the plate, projected to the
 * SettingsPanel's `SelectedObjectStub` shape. `null` when nothing's
 * selected or the plate is empty. Exported for tests. */
export function selectedObject(
  plate: PlateSnapshot | null,
): { id: number; name: string } | null {
  if (!plate || plate.selection.length === 0) return null;
  const id = plate.selection[0];
  const obj = plate.objects.find((o) => o.id === id);
  return obj ? { id: obj.id, name: obj.name } : null;
}

/** Project the plate's objects to the SettingsPanel's
 * `PlateObjectStub[]` shape — drives the "N objects override" badge
 * on Project-tab rows (FR-CAS-7b). Exported for tests. */
export function allObjectsForPanel(
  plate: PlateSnapshot | null,
): PlateObjectStub[] {
  if (!plate) return [];
  return plate.objects.map((o) => ({
    id: o.id,
    name: o.name,
    color: null, // Filament-color dots arrive with PR-7c filament sync.
    overrides: plate.object_overrides[o.id] ?? {},
  }));
}

export interface SettingsPanelHostProps {
  session: ProjectSession;
  /** All registered PrinterInstances — populates the picker. */
  instances: PrinterInstance[];
  /** Open the add-printer modal (the picker's "+ New printer…"
   *  entry fires this; App.tsx owns the modal). */
  onAddPrinter: () => void;
  /** Active-slot index plumbed into `buildContextJson` for the
   * cascade resolve. The settings panel itself no longer surfaces a
   * slot picker (PR-S-2 filtered to Process bucket — no per-
   * extruder rows here), so this defaults to 0 and stays there
   * until per-extruder editing surfaces ship in their own panels. */
  activeSlot?: number;
}

export function SettingsPanelHost({
  session,
  instances,
  onAddPrinter,
  activeSlot = 0,
}: SettingsPanelHostProps) {
  const plate = useMemo(() => activePlate(session), [session]);
  const selected = useMemo(() => selectedObject(plate), [plate]);
  const catalog = usePrinterCatalog();

  const projectOverrides = plate?.project_overrides ?? {};
  const userOverrides = session.snapshot?.user_overrides ?? {};
  const objectOverrides =
    plate && selected ? plate.object_overrides[selected.id] ?? {} : {};

  const allObjects = useMemo(() => allObjectsForPanel(plate), [plate]);

  // Pick the printer profile for cascade resolution from the active
  // plate's bound instance (the snapshot's derived
  // `printer_identity`). Falls back to the bootstrap printer when
  // the plate hasn't been bound yet — that's the App.tsx default-
  // printer load that runs before the user touches anything.
  const activeProfile = useMemo(() => {
    if (plate?.printer_identity) {
      const entry = catalog.entries.find(
        (e) => e.identity === plate.printer_identity,
      );
      if (entry) return entry.profile;
    }
    return session.printer;
  }, [plate?.printer_identity, catalog.entries, session.printer]);

  const context = useMemo(() => {
    if (!activeProfile) return null;
    return buildContextJson({
      printer: activeProfile,
      projectOverrides,
      userOverrides,
      objectOverrides,
      activeSlot,
    });
  }, [
    activeProfile,
    projectOverrides,
    userOverrides,
    objectOverrides,
    activeSlot,
  ]);

  const objectCbs = useMemo(
    () => makeObjectOverrideCallbacks(plate?.plate_id ?? null, selected?.id ?? null),
    [plate?.plate_id, selected?.id],
  );
  const projectCbs = useMemo(
    () => makeProjectOverrideCallbacks(plate?.plate_id ?? null),
    [plate?.plate_id],
  );

  // The bound PrinterInstance — drives the BuildPlateSelector +
  // NozzlePicker chips. Reads off the instance (not the binding)
  // per the post-build-plate-refactor source-of-truth: pickers
  // write through `printerInstanceSetBed` /
  // `printerInstanceSetExtruderNozzleDiameter`, the slicer composer
  // reads off `instance.bed.identity` /
  // `instance.extruders[i].installed_nozzle`. Same fetch +
  // `instance_changed` refresh pattern SlotBindingPanel uses.
  const instanceId = plate?.printer_instance_id ?? null;
  const [instance, setInstance] = useState<PrinterInstance | null>(null);
  useEffect(() => {
    if (!instanceId) {
      setInstance(null);
      return;
    }
    let cancelled = false;
    void getPrinterInstance(instanceId).then((inst) => {
      if (!cancelled) setInstance(inst);
    });
    let unlisten: UnlistenFn | null = null;
    void listen<string>("printer:instance_changed", (event) => {
      if (event.payload !== instanceId) return;
      void getPrinterInstance(instanceId).then((inst) => {
        if (!cancelled) setInstance(inst);
      });
    }).then((u) => {
      if (cancelled) u();
      else unlisten = u;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [instanceId]);
  const instanceBed = instance?.bed.identity ?? null;

  const extruderCount = instance?.extruders.length ?? 0;
  const inlineNozzles = nozzlesInline(extruderCount);
  const nozzleRows = chunkExtruders(extruderCount);
  const renderNozzlePicker = (extruderIdx: number, compact: boolean) => {
    if (!instance || !instanceId || !activeProfile) return null;
    const installed = instance.extruders[extruderIdx]?.installed_nozzle;
    if (!installed) return null;
    const defaultDiameter =
      activeProfile.toolheads[extruderIdx]?.default_nozzle_diameter ?? null;
    return (
      <NozzlePicker
        key={extruderIdx}
        extruderIdx={extruderIdx}
        totalExtruders={extruderCount}
        compact={compact}
        value={installed.diameter_mm}
        diameters={activeProfile.available_nozzle_diameters}
        printerDefault={defaultDiameter}
        onChange={(next) => {
          void setExtruderNozzleDiameter(instanceId, extruderIdx, next).catch(
            (err) => {
              console.error(
                "[settings] setExtruderNozzleDiameter failed",
                err,
              );
            },
          );
        }}
      />
    );
  };

  return (
    <div className="sp-host">
      <div className="sp-config">
        <div className="sp-config-row">
          <PrinterPicker
            plateId={plate?.plate_id ?? null}
            instances={instances}
            activeInstanceId={plate?.printer_instance_id ?? null}
            onAddPrinter={onAddPrinter}
          />
          {plate?.printer_identity && activeProfile && instanceId && instanceBed && (
            <BuildPlateSelector
              plates={activeProfile.supported_build_plates}
              value={instanceBed}
              onChange={(next) => {
                void setInstanceBed(instanceId, next).catch((err) => {
                  console.error("[settings] setInstanceBed failed", err);
                });
              }}
              printerDefault={activeProfile.supported_build_plates[0] ?? null}
            />
          )}
          {inlineNozzles &&
            Array.from({ length: extruderCount }, (_, i) =>
              renderNozzlePicker(i, false),
            )}
        </div>
        {nozzleRows.length > 0 && (
          <div
            className="sp-config-divider"
            role="separator"
            aria-label="Nozzles"
            title="Per-toolhead nozzles — click any chip below to change that extruder's installed nozzle."
          >
            <span className="sp-config-divider-label">Nozzles</span>
          </div>
        )}
        {nozzleRows.map((row, rowIdx) => (
          <div
            key={rowIdx}
            className="sp-config-row sp-config-nozzles"
          >
            {row.map((i) => renderNozzlePicker(i, true))}
          </div>
        ))}
        <SlotBindingPanel
          plateId={plate?.plate_id ?? null}
          plate={plate}
        />
      </div>
      <SettingsPanel
        printer={activeProfile}
        cascadeHandle={session.cascadeHandle}
        context={context}
        selectedObject={selected}
        objectOverrides={objectOverrides}
        onSetObjectOverride={objectCbs.onSetObjectOverride}
        onClearObjectOverride={objectCbs.onClearObjectOverride}
        projectOverrides={projectOverrides}
        onSetProjectOverride={projectCbs.onSetProjectOverride}
        onClearProjectOverride={projectCbs.onClearProjectOverride}
        allObjects={allObjects}
      />
    </div>
  );
}

/** localStorage key for the panel visibility toggle. */
const VISIBLE_KEY = "n3o.settingsPanelVisible";

/** Whether the settings panel is shown — persisted to localStorage
 * so the preference survives a reload. Default `true`. */
export function useSettingsPanelVisible(): [boolean, (v: boolean) => void] {
  const [visible, setVisible] = useState<boolean>(() => {
    if (typeof window === "undefined") return true;
    try {
      const raw = window.localStorage.getItem(VISIBLE_KEY);
      if (raw === null) return true;
      return raw === "true";
    } catch {
      return true;
    }
  });
  const set = (v: boolean) => {
    setVisible(v);
    try {
      window.localStorage.setItem(VISIBLE_KEY, String(v));
    } catch {
      // localStorage unavailable (privacy mode); preference doesn't
      // persist but the toggle still works in-session.
    }
  };
  return [visible, set];
}
