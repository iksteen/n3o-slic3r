import { useEffect, useRef, useState } from "react";
import { ViewportCanvas } from "./viewport/ViewportCanvas";
import type { GizmoMode } from "./viewport/types";
import { SlicePanel } from "./slice/SlicePanel";
import { useLastSliceOutput } from "./slice/useLastSliceOutput";
import { PlateTabs } from "./plates/PlateTabs";
import { useProjectSession } from "./project/useProjectSession";
import {
  AutosaveRecoveryDialog,
  useAutosaveRecoveryGate,
} from "./project/AutosaveRecoveryDialog";
import { autosaveEnable } from "./project/autosaveCommands";
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
import "./App.css";

function App() {
  const [mode, setMode] = useState<"scene" | "preview" | "devices">("scene");
  // Gizmo transform mode lives here (not in ViewportCanvas) so it
  // survives the unmount/remount ViewportCanvas undergoes on
  // prepare↔preview↔devices switches.
  const [gizmoMode, setGizmoMode] = useState<GizmoMode>("Translate");
  const session = useProjectSession();
  const recovery = useAutosaveRecoveryGate();
  const printers = usePrinterInstances();
  const printerCatalog = usePrinterCatalog();
  const [showAddPrinter, setShowAddPrinter] = useState(false);
  /** ID of the printer instance whose settings modal is open, or
   *  `null` when the modal isn't shown. Set by the per-printer
   *  settings cog (Devices view / settings panel). */
  const [editingPrinterId, setEditingPrinterId] = useState<string | null>(null);

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

  const bridge = useSlicePreviewBridge(activePlateId ?? null);
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

  // Auto-switch to preview on slice completion — unless the user has
  // manually toggled out of preview this session, or is currently in
  // the Devices monitor (a background/queued slice finishing must not
  // yank them out of the fleet view). The functional updater reads the
  // live mode rather than a closed-over copy.
  const userToggledOutRef = useRef(false);
  useEffect(() => {
    bridge.enableAutoSwitch(!userToggledOutRef.current);
    bridge.onPreviewReady(() => {
      if (userToggledOutRef.current) return;
      setMode((current) => (current === "devices" ? current : "preview"));
    });
  }, [bridge]);

  const goPrepare = (): void => {
    userToggledOutRef.current = true;
    setMode("scene");
  };
  const goPreview = (): void => {
    if (!canPreview) return;
    userToggledOutRef.current = false;
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
          userToggledOutRef.current = true;
          return "scene";
        }
        if (!canPreview) return current;
        userToggledOutRef.current = false;
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

  // No printers + bootstrap completed → onboarding takes over.
  const noPrinters =
    !printers.loading && printers.instances.length === 0;

  return (
    <div className="app">
      {!recovery.resolved && (
        <AutosaveRecoveryDialog onResolved={recovery.markResolved} />
      )}
      <header className="topbar">
        <span className="brand">
          <span className="brand-mark" aria-hidden />
          n3o-slic3r
        </span>
        <span className="tb-spacer" />
        {!showDevices && (
          <>
            <SlicePanel
              snapshot={session.snapshot}
              activePlate={activePlate}
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
