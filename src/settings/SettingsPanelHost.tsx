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
import {
  SettingsPanel,
  type PlateObjectStub,
  type PluginPlateSurface,
} from "./SettingsPanel";
import { usePlugins } from "../plugins/usePlugins";
import { platePluginWriters } from "../plugins/pluginWriters";
import { pluginSupportsPrinter } from "../plugins/pluginCascade";
import { usePlateCascadeResolve } from "./resolve";
import { makeObjectOverrideCallbacks } from "./overrideCommands";
import { makeProjectOverrideCallbacks } from "./projectOverrideCommands";
import { PrinterPicker } from "../printer/PrinterPicker";
import type { ConnectionSummary } from "../driver/useDriverConnections";
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
import { QualityPicker } from "./QualityPicker";
import { chunkExtruders, nozzlesInline } from "./nozzleLayout";
import {
  listProcessFragments,
  setPlateQualityProfile,
  type ProcessFragmentSummary,
} from "./processFragment";
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
  /** Per-(instance.id) auto-connection summary from
   *  useDriverConnections. Drives the picker chip's status-dot
   *  indicator and each row's right-side status. */
  connections: Record<string, ConnectionSummary>;
  /** Open the add-printer modal (the picker's "+ New printer…"
   *  entry fires this; App.tsx owns the modal). */
  onAddPrinter: () => void;
  /** Open the per-printer settings modal for the given instance.
   *  Wired from the cog button next to each row in PrinterPicker. */
  onEditPrinter: (instanceId: string) => void;
}

export function SettingsPanelHost({
  session,
  instances,
  connections,
  onAddPrinter,
  onEditPrinter,
}: SettingsPanelHostProps) {
  const plate = useMemo(() => activePlate(session), [session]);
  const selected = useMemo(() => selectedObject(plate), [plate]);
  const catalog = usePrinterCatalog();

  const projectOverrides = plate?.project_overrides ?? {};
  const userOverrides = session.snapshot?.user_overrides ?? {};
  const objectOverrides =
    plate && selected ? plate.object_overrides[selected.id] ?? {} : {};

  const allObjects = useMemo(() => allObjectsForPanel(plate), [plate]);

  // Plate-level plugin surface for the panel's Plugins tab. Note the
  // cascade-tier name remap: the plugin "project" level is the project-
  // wide user overrides, the plugin "plate" level is this plate's
  // project_overrides.
  const pluginList = usePlugins();

  // Pick the printer profile for cascade resolution from the active
  // plate's bound instance (the snapshot's derived `printer_identity`)
  // against the catalog. `null` when the plate is unbound (empty
  // library) or the catalog hasn't loaded — the panel renders its "No
  // printer selected" state for that.
  const activeProfile = useMemo(() => {
    if (plate?.printer_identity) {
      const entry = catalog.entries.find(
        (e) => e.identity === plate.printer_identity,
      );
      if (entry) return entry.profile;
    }
    return null;
  }, [plate?.printer_identity, catalog.entries]);

  // Plate-level plugin surface for the panel's Plugins tab. Reads the plugin
  // "project" level off the project-wide user overrides and "plate" off this
  // plate's project_overrides; the plate inherits its bound instance's
  // per-printer defaults (instanceOverrides). The list is filtered to plugins
  // compatible with the bound printer — a U1 plate doesn't show platecycler.
  const pluginSurface = useMemo<PluginPlateSurface | null>(() => {
    if (!plate) return null;
    const boundInstance =
      plate.printer_instance_id != null
        ? instances.find((i) => i.id === plate.printer_instance_id)
        : undefined;
    const model = activeProfile?.model ?? null;
    return {
      plugins: pluginList.plugins.filter((p) =>
        pluginSupportsPrinter(p, model),
      ),
      sources: {
        instanceOverrides: boundInstance?.config_overrides,
        projectOverrides: userOverrides,
        plateOverrides: projectOverrides,
      },
      writers: platePluginWriters(plate.plate_id),
      plateName: plate.name,
    };
  }, [
    plate,
    instances,
    activeProfile,
    pluginList.plugins,
    userOverrides,
    projectOverrides,
  ]);

  // The active plate's cascade resolution (fragments composed against
  // its effective process, each value tagged with the layer it won from).
  // Re-resolves when the plate switches, its process changes, or it's
  // rebound — those are the backend inputs.
  const { resolved } = usePlateCascadeResolve(
    plate?.plate_id ?? null,
    `${plate?.quality_profile ?? ""}|${plate?.printer_instance_id ?? ""}`,
  );

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

  // Quality picker — process fragments available for the active
  // (printer, installed-nozzle-set). The Quality chip surfaces
  // every process whose `available_for` includes any installed
  // nozzle (union rule, so composite profiles like `0.4+0.6` show
  // alongside single-nozzle ones whenever any of their nozzles is
  // present). Selection writes back via
  // `printer_instance_set_quality_profile`; the backend
  // emits the same `printer:instance_changed` event the bed +
  // nozzle setters do.
  //
  // The installed-nozzle list is computed across every extruder
  // (deduped + sorted) and joined into a stable comma-string. We
  // dep the effect on the *string* form (and the scalar slug +
  // model the listProcessFragments call needs) rather than on the
  // `instance` object itself: instance gets a fresh identity on
  // every printer:instance_changed event (slot color, slot binding,
  // bed swap), and listing it in deps re-fires the IPC call even
  // when none of the inputs the picker actually consumes changed.
  // Primitive-string deps compare by value via Object.is, so an
  // unchanged installed set is correctly a no-op.
  const installedNozzleKey = useMemo(() => {
    if (!instance) return "";
    const set = new Set<string>();
    for (const ext of instance.extruders) {
      set.add(ext.installed_nozzle.diameter);
    }
    return [...set].sort().join(",");
  }, [instance]);
  const printerFragmentSlug = instance?.printer_fragment_slug ?? null;
  const printerModel = activeProfile?.model ?? null;
  const [processOptions, setProcessOptions] = useState<
    readonly ProcessFragmentSummary[]
  >([]);
  useEffect(() => {
    if (!printerFragmentSlug || !printerModel || !installedNozzleKey) {
      setProcessOptions([]);
      return;
    }
    let cancelled = false;
    void listProcessFragments(
      printerFragmentSlug,
      printerModel,
      installedNozzleKey.split(","),
    )
      .then((opts) => {
        if (!cancelled) setProcessOptions(opts);
      })
      .catch((err) => {
        console.error("[settings] listProcessFragments failed", err);
        if (!cancelled) setProcessOptions([]);
      });
    return () => {
      cancelled = true;
    };
  }, [printerFragmentSlug, printerModel, installedNozzleKey]);

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
        value={installed.diameter}
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
            connections={connections}
            onAddPrinter={onAddPrinter}
            onEditPrinter={onEditPrinter}
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
        {instance && instanceId && plate && (
          <div className="sp-quality">
            <span className="config-row-label sp-quality-label">Quality</span>
            <div className="sp-quality-wrap">
              <QualityPicker
                // The plate's own process when set, else the bound
                // instance's default (the seed for new plates).
                value={plate.quality_profile ?? instance.quality_profile}
                options={processOptions}
                onChange={(next) => {
                  // Per-plate: record the choice on the plate, leaving
                  // the shared instance default untouched.
                  void setPlateQualityProfile(plate.plate_id, next).catch(
                    (err) => {
                      console.error(
                        "[settings] setPlateQualityProfile failed",
                        err,
                      );
                    },
                  );
                }}
              />
            </div>
          </div>
        )}
        <SlotBindingPanel
          plateId={plate?.plate_id ?? null}
          plate={plate}
          driverId={
            // Only hand the sync path a driver id when the connection
            // is actually `connected`. summaryFor still surfaces the
            // (old) driver id during a queued replace with status
            // "connecting" — dispatching a sync against that
            // about-to-be-torn-down driver is the bug we're avoiding.
            instanceId != null &&
            connections[instanceId]?.status === "connected"
              ? (connections[instanceId]?.driverId ?? null)
              : null
          }
        />
      </div>
      <SettingsPanel
        printer={activeProfile}
        resolved={resolved}
        selectedObject={selected}
        objectOverrides={objectOverrides}
        onSetObjectOverride={objectCbs.onSetObjectOverride}
        onClearObjectOverride={objectCbs.onClearObjectOverride}
        projectOverrides={projectOverrides}
        onSetProjectOverride={projectCbs.onSetProjectOverride}
        onClearProjectOverride={projectCbs.onClearProjectOverride}
        allObjects={allObjects}
        pluginSurface={pluginSurface}
      />
    </div>
  );
}

