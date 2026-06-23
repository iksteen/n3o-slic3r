import { useEffect, useState } from "react";
import { WgpuViewport } from "./viewport/WgpuViewport";
import { ViewportChrome } from "./viewport/ViewportChrome";
import { ViewportToasts } from "./viewport/ViewportToasts";
import { CloneDialog } from "./objects/CloneDialog";
import { cloneObjects } from "./objects/objectCommands";
import { setupTowerMeshCache } from "./viewport/towerMeshCache";
import { ErrorConsole } from "./logging/ErrorConsole";
import { shouldIgnoreHotkey } from "./ui/hotkeyInhibit";
import { setupLogSinks } from "./logging/logStore";
import { SlicePanel } from "./slice/SlicePanel";
import { SliceProgressWindow } from "./slice/SliceProgressWindow";
import { useSliceJob } from "./slice/useSliceJob";
import { useLastSliceOutput } from "./slice/useLastSliceOutput";
import { PlateTabs } from "./plates/PlateTabs";
import { useProjectSession } from "./project/useProjectSession";
import {
  AutosaveRecoveryDialog,
  useAutosaveRecoveryGate,
} from "./project/AutosaveRecoveryDialog";
import { autosaveEnable } from "./project/autosaveCommands";
import { projectNew, projectLoad, projectSave, projectSaveAs } from "./project/projectFile";
import { message as messageDialog } from "@tauri-apps/plugin-dialog";
import { openFile, saveFile } from "./ui/fileDialog";
import { onEvents } from "./state/eventRouter";
import { PROJECT_REPLACED_EVENTS } from "./project/editEvents";
import { SettingsPanelHost } from "./settings/SettingsPanelHost";
import { ObjectsPanel } from "./objects/ObjectsPanel";
import { PreviewWorkspace } from "./preview/PreviewWorkspace";
import { useSlicePreviewBridge } from "./preview/useSlicePreviewBridge";
import { SendControls } from "./driver/SendControls";
import { SendProgressWindow } from "./driver/SendProgressWindow";
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
import { BrandMenu, ProjectMenu } from "./plugins/TopBarMenus";
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

function App() {
  const [mode, setMode] = useState<"scene" | "preview" | "devices">("scene");
  // Selected printer in the Devices view, owned here (App is always mounted) so
  // it survives DevicesView's unmount on tab switches and so a Send can
  // pre-select the destination printer before the view mounts. `null` falls
  // back to the first printer.
  const [selectedDeviceId, setSelectedDeviceId] = useState<string | null>(null);
  // The wgpu viewport's gizmo mode.
  const [wgpuGizmo, setWgpuGizmo] = useState<"none" | "move" | "rotate" | "scale">("none");
  // wgpu viewport's armed placing tool (lay-flat / align). Mutually exclusive
  // with the gizmo: arming a tool clears the gizmo and vice versa.
  const [wgpuTool, setWgpuTool] = useState<
    "none" | "layflat" | "alignX" | "alignY" | "facematch" | "clone"
  >("none");
  // wgpu clone dialog (open with a set of ids; null = closed).
  const [wgpuClone, setWgpuClone] = useState<{ ids: number[]; expandGroups: boolean } | null>(null);
  // Match-face two-click sub-state: true once the reference face is picked.
  const [wgpuFaceMatchStep, setWgpuFaceMatchStep] = useState(false);
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
  // and the floating SliceProgressWindow over the canvas read one job, and
  // so the event subscription survives the topbar unmounting in Devices.
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

  // Reset the match-face step whenever the armed tool changes (arm/disarm/switch).
  useEffect(() => {
    setWgpuFaceMatchStep(false);
  }, [wgpuTool]);

  // App-lifetime listener feeding the priming-tower mesh cache. Lives here
  // (App is always mounted) rather than in ViewportCanvas, which unmounts
  // on the auto-switch to preview that fires the instant a slice finishes —
  // a viewport-local listener would miss the slice's tower mesh.
  useEffect(() => {
    // listen() returns a Promise<UnlistenFn>; the cleanup may run before it
    // resolves (StrictMode mounts→unmounts→remounts the effect synchronously),
    // so capture the promise and unlisten once it settles — a `cancelled` flag
    // tears down even if the cleanup fired first. Storing into a local `un`
    // and calling it in cleanup would no-op (still null) and leak the listener.
    let cancelled = false;
    const pending = setupTowerMeshCache().then((un) => {
      if (cancelled) un();
      return un;
    });
    return () => {
      cancelled = true;
      void pending.then((un) => un());
    };
  }, []);

  // App-lifetime routing of slice failures + libslic3r validation warnings
  // into the error-console log store (same cancel-safe pattern as above).
  useEffect(() => {
    let cancelled = false;
    const pending = setupLogSinks().then((un) => {
      if (cancelled) un();
      return un;
    });
    return () => {
      cancelled = true;
      void pending.then((un) => un());
    };
  }, []);

  // Opening / importing a project replaces the scene wholesale, so its
  // per-plate slice artifacts (output paths, preview, tower mesh) are dropped
  // by their caches. If we were *in* preview when that happened, leave preview
  // explicitly rather than leaning on `showPreview`'s `&& canPreview` mask —
  // the freshly-loaded project has nothing sliced. Devices view is left alone.
  useEffect(() => {
    return onEvents(PROJECT_REPLACED_EVENTS, () => {
      setMode((current) => (current === "preview" ? "scene" : current));
    });
  }, []);

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
      if (shouldIgnoreHotkey(e)) return;
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

  // A foreign project imported via Open project → show what mapped and
  // what was dropped, so lossy mapping is never silent.
  useEffect(() => {
    return onEvents<{ data: { report: ImportReport } }>(
      ["project:imported"],
      (e) => {
        const r = e.payload.data.report;
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
        void messageDialog(lines.join("\n"), {
          title: "Imported from OrcaSlicer / Bambu Studio",
        });
      },
    );
  }, []);

  // ----- Project file menu (Open / Save / Save as) -----
  // The backend's source_path is the project's on-disk origin; null
  // until first save. The menu label shows its basename, or
  // "Untitled.3mf" for an unsaved project.
  const sourcePath = session.snapshot?.source_path ?? null;
  const projectName = sourcePath
    ? sourcePath.split(/[\\/]/).pop() || "Untitled.n3o"
    : "Untitled.n3o";
  // Open accepts native .n3o projects AND foreign Bambu/Orca .3mf (imported);
  // Save only writes the native .n3o container.
  const openFilters = [
    { name: "Project", extensions: ["n3o", "3mf"] },
    { name: "n3o project", extensions: ["n3o"] },
    { name: "Bambu / Orca 3MF", extensions: ["3mf"] },
  ];
  const saveFilters = [{ name: "n3o project", extensions: ["n3o"] }];

  // Surface a project file-op failure to the user (a native dialog), not
  // just the console — opening e.g. an OrcaSlicer .3mf via "Open project"
  // otherwise looks like nothing happened.
  const reportProjectError = (action: string, err: unknown): void => {
    console.error(`[project] ${action} failed`, err);
    void messageDialog(String(err), { title: `${action} failed`, kind: "error" });
  };

  const handleNewProject = async (): Promise<void> => {
    try {
      await projectNew(); // → project:loaded → session + scene resync
    } catch (err) {
      reportProjectError("New project", err);
    }
  };

  const handleOpenProject = async (): Promise<void> => {
    try {
      const picked = await openFile({ filters: openFilters });
      if (picked == null) return; // cancelled
      await projectLoad(picked); // → project:loaded → session refetch
    } catch (err) {
      reportProjectError("Open project", err);
    }
  };

  const handleSaveProjectAs = async (): Promise<void> => {
    try {
      const picked = await saveFile({
        title: "Save project as",
        defaultPath: projectName,
        filters: saveFilters,
      });
      if (picked == null) return; // cancelled
      await projectSaveAs(picked); // adopts the new source_path
    } catch (err) {
      reportProjectError("Save project as", err);
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
      reportProjectError("Save project", err);
    }
  };

  // No printers + bootstrap completed → onboarding takes over.
  const noPrinters =
    !printers.loading && printers.instances.length === 0;

  // Object list — shown in both layouts (read-only in preview, kept mounted so
  // switching modes doesn't shift the layout).
  const objectsPanel = (
    <ObjectsPanel
      plate={activePlate ?? null}
      instance={activeInstance}
      printerName={activeInstance?.display_name ?? printerIdentity ?? "No printer"}
      plateSize={
        activePlate?.bed
          ? [
              activePlate.bed.extents.max[0] - activePlate.bed.extents.min[0],
              activePlate.bed.extents.max[1] - activePlate.bed.extents.min[1],
            ]
          : null
      }
      readOnly={showPreview}
    />
  );

  // Overlays that live inside the canvas frame in both layouts: the floating
  // slice-progress window and the error console. They anchor to the canvas
  // stage, never to the surrounding panels or the slider.
  const canvasOverlays = (
    <>
      <div className="progress-window-stack">
        <SliceProgressWindow
          state={slice.state}
          objectCount={sliceObjectCount ?? slicingPlate?.objects.length ?? 0}
          cancel={slice.cancel}
        />
        <SendProgressWindow driverId={activeConnection?.driverId ?? null} />
      </div>
      <ErrorConsole />
    </>
  );

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
            dirty={session.dirty}
            onNewProject={() => void handleNewProject()}
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
            />
            <SendControls
              printerIdentity={printerIdentity}
              connection={activeConnection}
              plateId={activePlateId}
              projectName={projectName}
              plateName={activePlate?.name ?? null}
              lastSliceOutputPath={lastSliceOutputPath}
              onSent={() => {
                // Land the user on the destination printer's live monitor —
                // but only when it's actually connected (otherwise the monitor
                // has nothing to show). Select before switching so the view
                // mounts already pointing at the right printer.
                if (
                  activeInstance != null &&
                  activeConnection?.status === "connected"
                ) {
                  setSelectedDeviceId(activeInstance.id);
                  setMode("devices");
                }
              }}
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
          selectedId={selectedDeviceId}
          onSelectId={setSelectedDeviceId}
          onAddPrinter={() => setShowAddPrinter(true)}
          onEditPrinter={(id) => setEditingPrinterId(id)}
        />
      ) : (
        showPreview ? (
          // ── Preview layout: objects (disabled) · canvas+slider · details ──
          <div className="layout-preview">
            {objectsPanel}
            <PreviewWorkspace
              preview={bridge.activePreview}
              bedExtents={bedExtents}
              toolbar={modeToggle}
              overlays={canvasOverlays}
            />
          </div>
        ) : (
          // ── Prepare layout: objects · canvas · settings ──
          <div className="layout-prepare">
            {objectsPanel}
            <div className="canvas-stage canvas-stage-prepare">
              {/* Strategy-A wgpu viewport: an opaque canvas fed by Rust-rendered
                  frames. The 3D scene state is authoritative in Rust; this is a
                  read-only consumer. */}
              <WgpuViewport
                objects={activePlate?.objects ?? []}
                selectedIds={activePlate?.selection ?? []}
                activePlateId={activePlateId}
                gizmoMode={wgpuGizmo}
                tool={wgpuTool}
                onToolDone={() => setWgpuTool("none")}
                onClonePick={(id) => {
                  setWgpuTool("none");
                  setWgpuClone({ ids: [id], expandGroups: true });
                }}
                onFaceMatchStep={setWgpuFaceMatchStep}
              />
              <ViewportChrome
                leading={modeToggle}
                objects={activePlate?.objects ?? []}
                selectedIds={activePlate?.selection ?? []}
                gizmoMode={wgpuGizmo}
                onGizmoMode={(m) => {
                  setWgpuTool("none");
                  setWgpuGizmo((cur) => (cur === m ? "none" : m));
                }}
                tool={wgpuTool}
                onTool={(t) => {
                  setWgpuGizmo("none");
                  setWgpuTool((cur) => (cur === t ? "none" : t));
                }}
                onClone={() => {
                  // With a selection, open the dialog on it; otherwise arm
                  // pick-to-clone (the next clicked object's group).
                  const sel = activePlate?.selection ?? [];
                  if (sel.length > 0) {
                    setWgpuClone({ ids: sel, expandGroups: false });
                  } else {
                    setWgpuGizmo("none");
                    setWgpuTool((cur) => (cur === "clone" ? "none" : "clone"));
                  }
                }}
                faceMatchRefSet={wgpuFaceMatchStep}
              />
              {wgpuClone && (
                <CloneDialog
                  count={wgpuClone.ids.length}
                  onConfirm={(copies) => {
                    const dlg = wgpuClone;
                    setWgpuClone(null);
                    void cloneObjects(dlg.ids, copies, dlg.expandGroups).catch((e) =>
                      console.error("clone failed", e),
                    );
                  }}
                  onCancel={() => setWgpuClone(null)}
                />
              )}
              {canvasOverlays}
              {/* Scene warnings (OOB / overflow) for whichever viewport is mounted. */}
              <ViewportToasts />
            </div>
            <SettingsPanelHost
              session={session}
              instances={printers.instances}
              connections={driverConnections}
              onAddPrinter={() => setShowAddPrinter(true)}
              onEditPrinter={(id) => setEditingPrinterId(id)}
            />
          </div>
        )
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
