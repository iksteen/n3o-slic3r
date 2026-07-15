// Settings-panel host.
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
import type { ProjectSession } from "../project/useProjectSession";
import {
  SettingsPanel,
  type PlateObject,
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
  setExtruderNozzleDiameter,
  setInstanceBed,
  type PrinterInstance,
} from "../printer/printerInstance";
import { usePrinterInstance } from "../printer/usePrinterInstance";
import { SlotBindingPanel } from "../material/SlotBindingPanel";
import { BuildPlateSelector } from "./BuildPlateSelector";
import { NozzlePicker } from "./NozzlePicker";
import { QualityPicker } from "./QualityPicker";
import { chunkExtruders, nozzlesInline } from "./nozzleLayout";
import {
  listProcessFragments,
  setPlateQualityProfile,
  stampUserProcess,
  revertUserProcess,
  duplicateUserProcess,
  deleteUserProcess,
  STAMP_EXCLUDED_KEYS,
  type ProcessFragmentSummary,
} from "./processFragment";
import { QualityProfileNameDialog } from "./QualityProfileNameDialog";
import type { PlateSnapshot } from "../viewport/types";

/** Floppy-disk "save" glyph for the Quality Save button. */
function SaveIcon(): React.JSX.Element {
  return (
    <svg width="15" height="15" viewBox="0 0 16 16" fill="none" aria-hidden>
      <path
        d="M3 2.5h7l3 3V13a.5.5 0 0 1-.5.5h-10A.5.5 0 0 1 2 13V3.5A1 1 0 0 1 3 2.5Z"
        stroke="currentColor"
        strokeWidth="1.2"
        strokeLinejoin="round"
      />
      <path
        d="M5 2.5v3.5h5V2.5M5.5 13v-3.5h5V13"
        stroke="currentColor"
        strokeWidth="1.2"
        strokeLinejoin="round"
      />
    </svg>
  );
}

/** Counterclockwise "undo" arrow for the Quality Revert button. */
function RevertIcon(): React.JSX.Element {
  return (
    <svg width="15" height="15" viewBox="0 0 16 16" fill="none" aria-hidden>
      <path
        d="M4 7h5a3.5 3.5 0 1 1 0 7H5.5"
        stroke="currentColor"
        strokeWidth="1.3"
        strokeLinecap="round"
      />
      <path
        d="M6 4.5 3.5 7 6 9.5"
        stroke="currentColor"
        strokeWidth="1.3"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

/** Two overlapping squares — "duplicate / save as" for the Quality Duplicate
 *  button. */
function DuplicateIcon(): React.JSX.Element {
  return (
    <svg width="15" height="15" viewBox="0 0 16 16" fill="none" aria-hidden>
      <rect
        x="5.5"
        y="5.5"
        width="8"
        height="8"
        rx="1.3"
        stroke="currentColor"
        strokeWidth="1.2"
      />
      <path
        d="M10.5 5.5V3.8A1.3 1.3 0 0 0 9.2 2.5H3.8A1.3 1.3 0 0 0 2.5 3.8v5.4a1.3 1.3 0 0 0 1.3 1.3h1.7"
        stroke="currentColor"
        strokeWidth="1.2"
      />
    </svg>
  );
}

/** Trash can — "delete" for the Quality button when a custom profile is
 *  selected. */
function DeleteIcon(): React.JSX.Element {
  return (
    <svg width="15" height="15" viewBox="0 0 16 16" fill="none" aria-hidden>
      <path
        d="M3 4.5h10M6.5 4.5V3.2A.7.7 0 0 1 7.2 2.5h1.6a.7.7 0 0 1 .7.7v1.3M4.2 4.5l.6 8a1 1 0 0 0 1 .9h4.4a1 1 0 0 0 1-.9l.6-8"
        stroke="currentColor"
        strokeWidth="1.2"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

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

/** Display name for a group: its authored name, else the same
 * appearance-ordinal fallback the Objects panel shows ("Group 2"), so
 * the two surfaces always agree on what a group is called. */
function groupDisplayName(plate: PlateSnapshot, group: string): string {
  const named = plate.groups[group]?.name;
  if (named) return named;
  const seen = new Set<string>();
  for (const o of plate.objects) {
    if (o.group != null && !seen.has(o.group)) {
      seen.add(o.group);
      if (o.group === group) return `Group ${seen.size}`;
    }
  }
  return "Group";
}

/** The selection projected to the SettingsPanel's `SelectedObject`
 * shape. A whole-group selection (the canvas's click-selects-the-group)
 * presents as the group — object-scope edits apply to it as one print
 * object, so the tab must say so. A partial selection (a member picked
 * from the objects panel) presents as that object. `null` when nothing's
 * selected or the plate is empty. Exported for tests. */
export function selectedObject(
  plate: PlateSnapshot | null,
): { id: number; name: string; kind: "object" | "group" } | null {
  if (!plate || plate.selection.length === 0) return null;
  const id = plate.selection[0];
  const obj = plate.objects.find((o) => o.id === id);
  if (!obj) return null;
  if (obj.group != null) {
    const members = plate.objects.filter((o) => o.group === obj.group);
    const selected = new Set(plate.selection);
    if (
      members.length >= 2 &&
      plate.selection.length === members.length &&
      members.every((m) => selected.has(m.id))
    ) {
      return { id, name: groupDisplayName(plate, obj.group), kind: "group" };
    }
  }
  return { id: obj.id, name: obj.name, kind: "object" };
}

/** One object's effective override map: its own overrides plus its
 * group's (a group slices as one object, so group-stored object-scope
 * settings apply to every member; the group value wins over a stale
 * member-stored copy). Exported for tests. */
export function effectiveObjectOverrides(
  plate: PlateSnapshot,
  objectId: number,
): Record<string, string> {
  const obj = plate.objects.find((o) => o.id === objectId);
  const group = obj?.group ? plate.groups[obj.group] : undefined;
  return {
    ...plate.object_overrides[objectId],
    ...group?.overrides,
  };
}

/** Project the plate's objects to the SettingsPanel's
 * `PlateObject[]` shape — drives the "N objects override" badge
 * on Project-tab rows (FR-CAS-7b). Exported for tests. */
export function allObjectsForPanel(
  plate: PlateSnapshot | null,
): PlateObject[] {
  if (!plate) return [];
  return plate.objects.map((o) => ({
    id: o.id,
    name: o.name,
    color: null, // Filament-color dots arrive with filament sync.
    overrides: effectiveObjectOverrides(plate, o.id),
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

  // Bumped after a stamp/revert/duplicate/delete so the Quality option list
  // (its `edited`/`custom` flags) and the cascade resolve both refresh — the
  // effective values move between the project tier and the baked profile.
  const [processGen, setProcessGen] = useState(0);
  // The Duplicate name dialog: `null` closed, else carries the ⌘/Ctrl-click
  // `clear` modifier captured at click time + the source profile's name.
  const [nameDialog, setNameDialog] = useState<{
    clear: boolean;
    sourceName: string;
  } | null>(null);

  const projectOverrides = plate?.project_overrides ?? {};
  const userOverrides = session.snapshot?.user_overrides ?? {};
  // Member + group overrides merged — the backend routes object-scope
  // writes on a grouped member to its group, so the panel reads both
  // through one map (writes stay on the plain object commands).
  const objectOverrides =
    plate && selected ? effectiveObjectOverrides(plate, selected.id) : {};

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

  // Writes bind to the first selected member even under a group
  // presentation: the backend routes object-scope keys to the group.
  // ponytail: a region-scope edit while "Group: …" is shown lands on that
  // one member only; fan region keys out group-wide if that ever bites.
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
  // `instance.extruders[i].installed_nozzle`. The shared per-instance
  // query handles the fetch + `instance_changed` refresh (and shares
  // it with SlotBindingPanel, which reads the same id).
  const instanceId = plate?.printer_instance_id ?? null;
  const instance = usePrinterInstance(instanceId);
  const instanceBed = instance?.bed.identity ?? null;

  // Quality picker — process fragments available for the active
  // (printer, installed-nozzle-set). The Quality chip surfaces
  // every process whose `available_for` includes any installed
  // nozzle (union rule, so composite profiles like `0.4+0.6` show
  // alongside single-nozzle ones whenever any of their nozzles is
  // present). Selection writes back via
  // `setPlateQualityProfile` (`project_set_plate_quality_profile`);
  // the backend emits `PlateChanged` so the session refetches
  // the snapshot and the cascade ladder re-resolves.
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

  // The active plate's cascade resolution (fragments composed against its
  // effective process, each value tagged with the layer it won from).
  // Re-resolves on the backend inputs: plate switch, its process, its printer
  // binding, and the bound instance's bed + installed-nozzle loadout (the
  // backend composes the cascade off those, and a bed/nozzle pick emits only
  // `printer:instance_changed` — outside the snapshot refetch set — so they
  // must be in the key or the ladder shows the previous fragment's values).
  const { resolved } = usePlateCascadeResolve(
    plate?.plate_id ?? null,
    `${plate?.quality_profile ?? ""}|${plate?.printer_instance_id ?? ""}|${instanceBed ?? ""}|${installedNozzleKey}|${processGen}`,
  );
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
  }, [printerFragmentSlug, printerModel, installedNozzleKey, processGen]);

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
          <>
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
              {(() => {
                const selectedSlug =
                  plate.quality_profile ?? instance.quality_profile;
                const selectedOption = processOptions.find(
                  (o) => o.slug === selectedSlug,
                );
                const selectedEdited = !!selectedOption?.edited;
                const selectedCustom = !!selectedOption?.custom;
                const selectedName =
                  selectedOption?.display_name ?? selectedSlug;
                // Something to stamp = the plate carries a stampable quality
                // edit — a Process-bucket project override that isn't a
                // viewport-managed placement key (a dragged tower must never
                // enable Save).
                const hasEdits = Object.keys(projectOverrides).some(
                  (k) => !STAMP_EXCLUDED_KEYS.includes(k),
                );
                return (
                  <>
                    <button
                      type="button"
                      className="sp-quality-action"
                      disabled={!hasEdits}
                      aria-label="Save quality settings to this profile"
                      title={
                        hasEdits
                          ? "Save these quality settings onto this profile.\n⌘/Ctrl-click: save, then clear them from the plate (save then clear)."
                          : "No unsaved quality changes to save."
                      }
                      onClick={(e) => {
                        const clear = e.ctrlKey || e.metaKey;
                        void stampUserProcess(plate.plate_id, clear)
                          .then(() => setProcessGen((g) => g + 1))
                          .catch((err) =>
                            console.error(
                              "[settings] stampUserProcess failed",
                              err,
                            ),
                          );
                      }}
                    >
                      <SaveIcon />
                    </button>
                    <button
                      type="button"
                      className="sp-quality-action"
                      aria-label="Save as a new custom quality profile"
                      title={
                        "Save these quality settings as a new named profile.\n⌘/Ctrl-click: save, then clear them from the plate (save then clear)."
                      }
                      onClick={(e) =>
                        setNameDialog({
                          clear: e.ctrlKey || e.metaKey,
                          sourceName: selectedName,
                        })
                      }
                    >
                      <DuplicateIcon />
                    </button>
                    {selectedCustom ? (
                      <button
                        type="button"
                        className="sp-quality-action danger"
                        aria-label="Delete this custom profile"
                        title={
                          "Delete this custom profile and switch back to the default.\n⌘/Ctrl-click: delete, but keep its settings as project overrides."
                        }
                        onClick={(e) => {
                          const apply = e.ctrlKey || e.metaKey;
                          if (
                            !apply &&
                            !window.confirm(
                              `Delete the custom profile “${selectedName}”?`,
                            )
                          ) {
                            return;
                          }
                          void deleteUserProcess(plate.plate_id, apply)
                            .then(() => setProcessGen((g) => g + 1))
                            .catch((err) =>
                              console.error(
                                "[settings] deleteUserProcess failed",
                                err,
                              ),
                            );
                        }}
                      >
                        <DeleteIcon />
                      </button>
                    ) : (
                      <button
                        type="button"
                        className="sp-quality-action danger"
                        disabled={!selectedEdited}
                        aria-label="Revert this profile to bundled defaults"
                        title={
                          selectedEdited
                            ? "Revert this profile to its bundled defaults.\n⌘/Ctrl-click: revert, but keep its settings as project overrides."
                            : "This profile has no saved overrides to revert."
                        }
                        onClick={(e) => {
                          const apply = e.ctrlKey || e.metaKey;
                          if (
                            !apply &&
                            !window.confirm(
                              "Revert this quality profile to its bundled defaults?",
                            )
                          ) {
                            return;
                          }
                          void revertUserProcess(plate.plate_id, apply)
                            .then(() => setProcessGen((g) => g + 1))
                            .catch((err) =>
                              console.error(
                                "[settings] revertUserProcess failed",
                                err,
                              ),
                            );
                        }}
                      >
                        <RevertIcon />
                      </button>
                    )}
                  </>
                );
              })()}
            </div>
          </div>
          {nameDialog && (
            <QualityProfileNameDialog
              sourceName={nameDialog.sourceName}
              onClose={() => setNameDialog(null)}
              onCreate={(name) => {
                const { clear } = nameDialog;
                setNameDialog(null);
                void duplicateUserProcess(plate.plate_id, name, clear)
                  .then(() => setProcessGen((g) => g + 1))
                  .catch((err) =>
                    console.error(
                      "[settings] duplicateUserProcess failed",
                      err,
                    ),
                  );
              }}
            />
          )}
          </>
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

