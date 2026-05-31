import { useEffect, useState } from "react";
import { ViewportCanvas } from "./viewport/ViewportCanvas";
import type { GizmoMode } from "./viewport/types";
import { SlicePanel } from "./slice/SlicePanel";
import { SlicingWindow } from "./slice/SlicingWindow";
import { useSliceJob } from "./slice/useSliceJob";
import { useLastSliceOutput } from "./slice/useLastSliceOutput";
import { PlateTabs } from "./plates/PlateTabs";
import { useProjectSession } from "./project/useProjectSession";
import {
  AutosaveRecoveryDialog,
  useAutosaveRecoveryGate,
} from "./project/AutosaveRecoveryDialog";
import { autosaveEnable } from "./project/autosaveCommands";
import { projectLoad, projectSave, projectSaveAs } from "./project/projectFile";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { SettingsPanelHost } from "./settings/SettingsPanelHost";
import { PreviewWorkspace } from "./preview/PreviewWorkspace";
import { useSlicePreviewBridge } from "./preview/useSlicePreviewBridge";
import { SendControls } from "./driver/SendControls";
import { DevicesView } from "./driver/DevicesView";
import { useDriverConnections } from "./driver/useDriverConnections";
import { usePrinterInstances } from "./printer/usePrinterInstances";
import { usePrinterCatalog } from "./printer/usePrinterCatalog";
import { PrintersEmptyState } from "./printer/PrintersEmptyState";
import {
  AddPrinterModal,
  type AddPrinterResult,
} from "./printer/AddPrinterModal";
import { PrinterSettingsModal } from "./printer/PrinterSettingsModal";
import { createInstance } from "./printer/printerInstance";
import { rebindPlatePrinter } from "./printer/printerCommands";
import { usePlugins } from "./plugins/usePlugins";
import { BrandMenu, ProjectMenu } from "./plugins/TopBarPluginMenus";
import { PluginsModal } from "./plugins/PluginsModal";
import {
  globalPluginWriters,
  projectPluginWriters,
} from "./plugins/pluginWriters";
import {
  countActiveAtLevel,
  type CascadeSources,
} from "./plugins/pluginCascade";
import "./App.css";

function App() {
  const [mode, setMode] = useState<"scene" | "preview" | "devices">("scene");
  // Gizmo transform mode lives here (not in ViewportCanvas) so it
  // survives the unmount/remount ViewportCanvas undergoes on
  // prepare↔preview↔devices switches.
  const [gizmoMode, setGizmoMode] = useState<GizmoMode>("Translate");
  // Object count frozen at the moment a slice is submitted — what the
  // backend actually snapshots and slices (build_slice_input). Held
  // here so the progress window's count stays put across tab switches
  // and event timing, rather than tracking the live front tab. `null`
  // until the first slice this session (and on a resume-from-reload,
  // which bypasses the submit path — handled by the lookup fallback).
  const [sliceObjectCount, setSliceObjectCount] = useState<number | null>(null);
  const session = useProjectSession();
  const recovery = useAutosaveRecoveryGate();
  const printers = usePrinterInstances();
  const printerCatalog = usePrinterCatalog();
  const [showAddPrinter, setShowAddPrinter] = useState(false);
  /** ID of the printer instance whose settings modal is open, or
   *  `null` when the modal isn't shown. Set by the per-printer
   *  settings cog (Devices view / settings panel). */
  const [editingPrinterId, setEditingPrinterId] = useState<string | null>(null);
  // Plugin catalog (health + global state); re-fetched on plugin:changed.
  const pluginList = usePlugins();
  const [showGlobalPlugins, setShowGlobalPlugins] = useState(false);
  const [showProjectPlugins, setShowProjectPlugins] = useState(false);

  const activePlate =
    session.snapshot?.plates.find(
      (p) => p.plate_id === session.snapshot?.active_plate_id,
    ) ?? null;
  const activePlateId = activePlate?.plate_id ?? null;
  const bedExtents = activePlate?.bed
    ? {
        min: activePlate.bed.extents.min,
        max: activePlate.bed.extents.max,
      }
    : null;

  // Plugin override sources for the menus' counts + modals: project
  // level = the project-wide user overrides, plate level = the active
  // plate's overrides.
  const pluginSources: CascadeSources = {
    projectOverrides: session.snapshot?.user_overrides ?? {},
    plateOverrides: activePlate?.project_overrides,
  };
  const globalPluginCount = countActiveAtLevel(
    pluginList.plugins,
    "global",
    pluginSources,
  );
  const projectPluginCount = countActiveAtLevel(
    pluginList.plugins,
    "project",
    pluginSources,
  );

  const bridge = useSlicePreviewBridge(activePlateId ?? null);
  // Slice-job state lives here (not in SlicePanel) so the topbar button
  // and the floating SlicingWindow over the canvas read one job, and so
  // the event subscription survives the topbar unmounting in Devices.
  const slice = useSliceJob();
  // Resume-only fallback for the progress window's object count: on a
  // reload mid-slice the submit path didn't run, so `sliceObjectCount`
  // is null — look the slicing plate up by the resumed job's plate_id.
  // (`plate_id` is null only in the brief pre-PlateStarted window.) In
  // the normal flow `sliceObjectCount` wins and this isn't consulted.
  const slicingPlate =
    slice.state.plate_id != null
      ? (session.snapshot?.plates.find(
          (p) => p.plate_id === slice.state.plate_id,
        ) ?? null)
      : null;
  const lastSliceOutput = useLastSliceOutput();
  const lastSliceOutputPath =
    activePlateId != null ? lastSliceOutput.pathForPlate(activePlateId) : null;
  const printerIdentity = activePlate?.printer_identity ?? null;
  const activeInstance =
    activePlate?.printer_instance_id != null
      ? (printers.instances.find(
          (i) => i.id === activePlate.printer_instance_id,
        ) ?? null)
      : null;
  // Auto-connect / reconnect / disconnect every printer instance
  // whenever its persisted connection settings change. Drivers
  // outlive the React tree (module-scoped). The summary is keyed
  // by instance.id (UUID) so two instances of the same printer
  // model get distinct drivers. Feeds the active connection summary
  // to SendControls (which picks the driver id off it) and the
  // Devices view (per-printer monitor + status dot).
  const driverConnections = useDriverConnections(printers.instances);
  const activeConnection =
    activeInstance != null
      ? (driverConnections[activeInstance.id] ?? null)
      : null;

  const canPreview = bridge.activePreview != null;
  const showPreview = mode === "preview" && canPreview;
  const showDevices = mode === "devices";

  // Auto-switch to preview on slice completion — but only for the plate
  // the user is actually looking at. A slice finishing for a tab they
  // switched away from must not flip the view (the active plate has no
  // preview, so "preview" mode would render nothing), and a
  // background/queued slice finishing must not yank them out of the
  // Devices monitor. `activePlateId` is in the deps so the callback
  // always closes over the current active plate; the functional updater
  // reads the live mode rather than a closed-over copy.
  useEffect(() => {
    bridge.onPreviewReady((plateId) => {
      if (plateId !== activePlateId) return;
      setMode((current) => (current === "devices" ? current : "preview"));
    });
  }, [bridge, activePlateId]);

  const goPrepare = (): void => {
    setMode("scene");
  };
  const goPreview = (): void => {
    if (!canPreview) return;
    setMode("preview");
  };

  // Keyboard shortcut: `P` toggles the G-code preview. From preview it
  // returns to prepare; from any non-preview mode (scene OR devices) it
  // enters preview, but only when a slice exists — entering "preview"
  // with canPreview false would leave mode and the render desynced.
  // Depends on canPreview so the listener never reads a stale value;
  // the functional updater keeps `mode` itself fresh.
  useEffect(() => {
    const onKey = (e: KeyboardEvent): void => {
      if (e.key !== "p" && e.key !== "P") return;
      const el = document.activeElement;
      const tag = el?.tagName;
      if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return;
      if ((el as HTMLElement | null)?.isContentEditable) return;
      setMode((current) => {
        if (current === "preview") {
          return "scene";
        }
        if (!canPreview) return current;
        return "preview";
      });
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [canPreview]);

  // Prepare/Preview segmented toggle. Lives in the canvas toolbar:
  // rendered as the leading item of the viewport toolbar in prepare
  // mode, and as a standalone overlay over the preview workspace —
  // the toolbar itself (ViewportCanvas) is unmounted in preview, so
  // the toggle has to ride along separately to stay reachable.
  const modeToggle = (
    <div className="bg-neutral-800/90 text-neutral-100 text-xs rounded shadow flex overflow-hidden pointer-events-auto">
      <button
        type="button"
        className={`px-3 py-1 ${
          !showPreview ? "bg-neutral-700" : "hover:bg-neutral-700/60"
        }`}
        onClick={goPrepare}
        title="Prepare (P)"
      >
        Prepare
      </button>
      <button
        type="button"
        className={`px-3 py-1 ${
          showPreview
            ? "bg-neutral-700"
            : canPreview
              ? "hover:bg-neutral-700/60"
              : "opacity-40 cursor-not-allowed"
        }`}
        onClick={goPreview}
        disabled={!canPreview}
        title={canPreview ? "G-code preview (P)" : "Slice the active plate first"}
      >
        Preview
      </button>
    </div>
  );

  // Enable the autosave worker on launch. Safe to fire before the
  // recovery dialog resolves — the worker writes to the new
  // session's per-uuid file, which never collides with a still-
  // unresolved recovery candidate (different uuid).
  useEffect(() => {
    void autosaveEnable().catch((err) =>
      console.error("[autosave] enable failed", err),
    );
  }, []);

  const handleAddPrinter = async (result: AddPrinterResult): Promise<void> => {
    try {
      const inst = await createInstance(
        result.printerIdentity,
        result.displayName,
        result.amsUnits,
      );
      setShowAddPrinter(false);
      // Auto-bind: every plate without a current printer binding
      // gets the new one. Covers two cases together —
      //   (a) Empty-state flow (deleted-last-printer or first
      //       launch): every plate is unbound; all bind to the
      //       new printer.
      //   (b) Picker flow with extra unbound plates lying around:
      //       same.
      // The active plate is rebound regardless (matches the
      // design's "always auto-bind the active plate after a
      // create" choice). Other plates already bound to a different
      // printer stay put.
      const snapshot = session.snapshot;
      const targets = new Set<number>();
      if (activePlateId != null) targets.add(activePlateId);
      if (snapshot) {
        for (const plate of snapshot.plates) {
          if (plate.printer_instance_id == null) targets.add(plate.plate_id);
        }
      }
      await Promise.all(
        [...targets].map((plateId) => rebindPlatePrinter(plateId, inst.id)),
      );
    } catch (err) {
      console.error("[printer] create failed", err);
    }
  };

  // ----- Project file menu (Open / Save / Save as) -----
  // The backend's source_path is the project's on-disk origin; null
  // until first save. The menu label shows its basename, or
  // "Untitled.3mf" for an unsaved project.
  const sourcePath = session.snapshot?.source_path ?? null;
  const projectName = sourcePath
    ? sourcePath.split(/[\\/]/).pop() || "Untitled.3mf"
    : "Untitled.3mf";
  const projectFilters = [{ name: "n3o project", extensions: ["3mf"] }];

  const handleOpenProject = async (): Promise<void> => {
    try {
      const picked = await openDialog({ multiple: false, filters: projectFilters });
      if (typeof picked !== "string") return; // cancelled
      await projectLoad(picked); // → project:loaded → session refetch
    } catch (err) {
      console.error("[project] open failed", err);
    }
  };

  const handleSaveProjectAs = async (): Promise<void> => {
    try {
      const picked = await saveDialog({
        title: "Save project as",
        defaultPath: projectName,
        filters: projectFilters,
      });
      if (typeof picked !== "string") return; // cancelled
      await projectSaveAs(picked); // adopts the new source_path
    } catch (err) {
      console.error("[project] save-as failed", err);
    }
  };

  const handleSaveProject = async (): Promise<void> => {
    // No source path yet (Untitled) → behave like Save As.
    if (!sourcePath) {
      await handleSaveProjectAs();
      return;
    }
    try {
      await projectSave(sourcePath);
    } catch (err) {
      console.error("[project] save failed", err);
    }
  };

  // No printers + bootstrap completed → onboarding takes over.
  const noPrinters =
    !printers.loading && printers.instances.length === 0;

  return (
    <div className="app">
      {!recovery.resolved && (
        <AutosaveRecoveryDialog onResolved={recovery.markResolved} />
      )}
      <header className="topbar">
        <BrandMenu
          onOpenGlobalPlugins={() => setShowGlobalPlugins(true)}
          globalPluginCount={globalPluginCount}
        />
        {session.snapshot && (
          <ProjectMenu
            projectName={projectName}
            onOpenProject={() => void handleOpenProject()}
            onSaveProject={() => void handleSaveProject()}
            onSaveProjectAs={() => void handleSaveProjectAs()}
            onOpenProjectPlugins={() => setShowProjectPlugins(true)}
            projectPluginCount={projectPluginCount}
          />
        )}
        <span className="tb-spacer" />
        {!showDevices && (
          <>
            <SlicePanel
              snapshot={session.snapshot}
              activePlate={activePlate}
              state={slice.state}
              start={() => {
                // Freeze the count for the plate being sliced (the
                // active plate — slice_active_plate targets it) at
                // submit time, so the progress window shows what's
                // actually being sliced regardless of later tab
                // switches.
                setSliceObjectCount(activePlate?.objects.length ?? 0);
                return slice.start();
              }}
              cancel={slice.cancel}
            />
            <SendControls
              printerIdentity={printerIdentity}
              connection={activeConnection}
              plateId={activePlateId}
              lastSliceOutputPath={lastSliceOutputPath}
            />
          </>
        )}
      </header>

      <PlateTabs
        devicesActive={showDevices}
        deviceCount={printers.instances.length}
        onSelectDevices={() => setMode("devices")}
        onSelectPlate={() => setMode("scene")}
      />

      {session.error ? (
        <div className="sp-error" role="alert" style={{ margin: "8px 14px" }}>
          Bootstrap failed: {session.error}
        </div>
      ) : noPrinters ? (
        <PrintersEmptyState
          catalog={printerCatalog.entries}
          onAdd={() => setShowAddPrinter(true)}
        />
      ) : showDevices ? (
        <DevicesView
          instances={printers.instances}
          connections={driverConnections}
          onAddPrinter={() => setShowAddPrinter(true)}
          onEditPrinter={(id) => setEditingPrinterId(id)}
        />
      ) : (
        <div className={`workspace ${showPreview ? "preview-mode" : ""}`}>
          <main style={{ position: "relative", minWidth: 0 }}>
            {showPreview ? (
              <>
                <PreviewWorkspace
                  preview={bridge.activePreview}
                  bedExtents={bedExtents}
                />
                <div className="absolute top-2 left-2 flex pointer-events-none">
                  {modeToggle}
                </div>
              </>
            ) : (
              <ViewportCanvas
                leading={modeToggle}
                gizmoMode={gizmoMode}
                onGizmoMode={setGizmoMode}
              />
            )}
            {/* Floating slice-progress window — positioned against this
                `relative` <main>, lower-left, over whichever canvas is
                active. Renders only while a slice is in flight. */}
            <SlicingWindow
              state={slice.state}
              objectCount={
                sliceObjectCount ?? slicingPlate?.objects.length ?? 0
              }
            />
          </main>
          {!showPreview && (
            <SettingsPanelHost
              session={session}
              instances={printers.instances}
              connections={driverConnections}
              onAddPrinter={() => setShowAddPrinter(true)}
              onEditPrinter={(id) => setEditingPrinterId(id)}
            />
          )}
        </div>
      )}

      {showAddPrinter && (
        <AddPrinterModal
          catalog={printerCatalog.entries}
          existingNames={printers.instances.map((i) => i.display_name)}
          onAdd={(result) => {
            void handleAddPrinter(result);
          }}
          onClose={() => setShowAddPrinter(false)}
        />
      )}

      {editingPrinterId &&
        (() => {
          const editing = printers.instances.find(
            (i) => i.id === editingPrinterId,
          );
          if (!editing) {
            // Instance vanished out from under us (deleted in
            // another window? race?). Drop the modal state on
            // the next render — guard the mount, not the state.
            return null;
          }
          return (
            <PrinterSettingsModal
              instance={editing}
              instances={printers.instances}
              plates={session.snapshot?.plates ?? []}
              onClose={() => setEditingPrinterId(null)}
            />
          );
        })()}

      {showGlobalPlugins && (
        <PluginsModal
          level="global"
          plugins={pluginList.plugins}
          sources={pluginSources}
          writers={globalPluginWriters()}
          onClose={() => setShowGlobalPlugins(false)}
        />
      )}

      {showProjectPlugins && (
        <PluginsModal
          level="project"
          plugins={pluginList.plugins}
          sources={pluginSources}
          writers={projectPluginWriters()}
          onClose={() => setShowProjectPlugins(false)}
        />
      )}

      <footer className="statusbar">
        <span className="dot" aria-hidden />
        <span>
          {session.loading
            ? "Loading…"
            : session.snapshot
            ? `${session.snapshot.plates.length} plate${session.snapshot.plates.length === 1 ? "" : "s"}`
            : "—"}
        </span>
        <span className="spacer" />
      </footer>
    </div>
  );
}

export default App;
