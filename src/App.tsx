import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ViewportCanvas } from "./viewport/ViewportCanvas";
import { SlicePanel } from "./slice/SlicePanel";
import { useLastSliceOutput } from "./slice/useLastSliceOutput";
import { PlateTabs } from "./plates/PlateTabs";
import { useProjectSession } from "./project/useProjectSession";
import {
  AutosaveRecoveryDialog,
  useAutosaveRecoveryGate,
} from "./project/AutosaveRecoveryDialog";
import { autosaveEnable } from "./project/autosaveCommands";
import {
  SettingsPanelHost,
  useSettingsPanelVisible,
} from "./settings/SettingsPanelHost";
import { PreviewWorkspace } from "./preview/PreviewWorkspace";
import { useSlicePreviewBridge } from "./preview/useSlicePreviewBridge";
import { PrinterPanel } from "./driver/PrinterPanel";
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

type SlicerInfo = { version: string; option_count: number };

function App() {
  const [info, setInfo] = useState<SlicerInfo | null>(null);
  const [showDebug, setShowDebug] = useState(false);
  const [panelVisible, setPanelVisible] = useSettingsPanelVisible();
  const [mode, setMode] = useState<"scene" | "preview">("scene");
  const session = useProjectSession();
  const recovery = useAutosaveRecoveryGate();
  const printers = usePrinterInstances();
  const printerCatalog = usePrinterCatalog();
  const [showAddPrinter, setShowAddPrinter] = useState(false);
  /** ID of the printer instance whose settings modal is open, or
   *  `null` when the modal isn't shown. The cog button in
   *  PrinterPicker sets this. */
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
  // model get distinct drivers. Feeds the active connection
  // summary to PrinterPanel (it picks the driver id off it + the
  // status branches the empty-state copy) and the per-printer
  // status dot to PrinterPicker.
  const driverConnections = useDriverConnections(printers.instances);
  const activeConnection =
    activeInstance != null
      ? (driverConnections[activeInstance.id] ?? null)
      : null;

  // Auto-switch to preview on slice completion, unless the user
  // has manually toggled out of preview during this session.
  const userToggledOutRef = useRef(false);
  useEffect(() => {
    bridge.enableAutoSwitch(!userToggledOutRef.current);
    bridge.onPreviewReady(() => {
      if (!userToggledOutRef.current) {
        setMode("preview");
      }
    });
  }, [bridge]);

  // Keyboard shortcut: `P` toggles between scene and preview.
  useEffect(() => {
    const onKey = (e: KeyboardEvent): void => {
      if (e.key !== "p" && e.key !== "P") return;
      const el = document.activeElement;
      const tag = el?.tagName;
      if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return;
      if ((el as HTMLElement | null)?.isContentEditable) return;
      togglePreview();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
    // togglePreview captures setMode; safe with empty deps.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const togglePreview = (): void => {
    setMode((current) => {
      if (current === "preview") {
        userToggledOutRef.current = true;
        return "scene";
      }
      userToggledOutRef.current = false;
      return "preview";
    });
  };

  const canPreview = bridge.activePreview != null;
  const showPreview = mode === "preview" && canPreview;

  useEffect(() => {
    invoke<SlicerInfo>("slicer_info")
      .then(setInfo)
      .catch(() => undefined);
  }, []);

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
        <SlicePanel
          snapshot={session.snapshot}
          activePlate={activePlate}
        />
        <PrinterPanel
          printerIdentity={printerIdentity}
          connection={activeConnection}
          plateId={activePlateId}
          lastSliceOutputPath={lastSliceOutputPath}
        />
        {info && (
          <span style={{ color: "var(--text-dim)", fontFamily: "var(--font-mono)", fontSize: "10.5px" }}>
            {info.version} · {info.option_count} options
          </span>
        )}
        <button
          type="button"
          className={`tb-btn${showPreview ? " active" : ""}`}
          onClick={togglePreview}
          disabled={!canPreview && mode === "scene"}
          title={
            canPreview
              ? "Toggle G-code preview (P)"
              : "Slice the active plate first"
          }
        >
          Preview <span className="kbd">P</span>
        </button>
        <button
          type="button"
          className="tb-btn"
          onClick={() => setPanelVisible(!panelVisible)}
          title="Toggle settings panel (persists across reloads)"
        >
          {panelVisible ? "Hide settings" : "Settings"}
        </button>
        <button
          type="button"
          className="tb-btn"
          onClick={() => setShowDebug((v) => !v)}
        >
          {showDebug ? "Hide debug" : "Debug"}
        </button>
      </header>

      <PlateTabs />

      {session.error ? (
        <div className="sp-error" role="alert" style={{ margin: "8px 14px" }}>
          Bootstrap failed: {session.error}
        </div>
      ) : noPrinters ? (
        <PrintersEmptyState
          catalog={printerCatalog.entries}
          onAdd={() => setShowAddPrinter(true)}
        />
      ) : (
        <div
          className={`workspace ${
            showPreview ? "preview-mode" : panelVisible ? "" : "no-panel"
          }`}
        >
          <main style={{ position: "relative", minWidth: 0 }}>
            {showPreview ? (
              <PreviewWorkspace
                preview={bridge.activePreview}
                bedExtents={bedExtents}
              />
            ) : (
              <ViewportCanvas />
            )}
          </main>
          {!showPreview && panelVisible && (
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
        {showDebug && <DebugPanel />}
      </footer>
    </div>
  );
}

type OptionSummary = {
  key: string;
  ty: string;
  label: string | null;
  category: string | null;
  default_value: string | null;
};

type SliceResult = { ok: boolean; out_path: string; error: string | null };

function DebugPanel() {
  const [filter, setFilter] = useState("");
  const [options, setOptions] = useState<OptionSummary[]>([]);
  const [modelPath, setModelPath] = useState("");
  const [outPath] = useState("/tmp/n3o-out.gcode");
  const [sliceMsg, setSliceMsg] = useState("");

  async function loadOptions() {
    const opts = await invoke<OptionSummary[]>("slicer_options", { filter });
    setOptions(opts.slice(0, 50));
  }

  async function doSlice() {
    setSliceMsg("slicing…");
    const r = await invoke<SliceResult>("slicer_slice", { modelPath, outPath });
    setSliceMsg(r.ok ? `wrote ${r.out_path}` : `error: ${r.error}`);
  }

  // Debug panel renders inline in the status row as a popover-like inline
  // text block. Kept minimal — it's a developer affordance, not user UX.
  return (
    <span style={{ display: "flex", gap: 8, alignItems: "center" }}>
      <input
        value={filter}
        onChange={(e) => setFilter(e.target.value)}
        placeholder="opt filter"
        style={{
          background: "var(--surface-2)",
          border: "1px solid var(--border)",
          borderRadius: 4,
          padding: "1px 6px",
          fontSize: 10.5,
          width: 120,
        }}
      />
      <button type="button" className="tb-btn" style={{ height: 20, padding: "0 8px", fontSize: 10.5 }} onClick={() => void loadOptions()}>
        opts ({options.length})
      </button>
      <input
        value={modelPath}
        onChange={(e) => setModelPath(e.target.value)}
        placeholder="model.stl"
        style={{
          background: "var(--surface-2)",
          border: "1px solid var(--border)",
          borderRadius: 4,
          padding: "1px 6px",
          fontSize: 10.5,
          width: 140,
        }}
      />
      <button type="button" className="tb-btn" style={{ height: 20, padding: "0 8px", fontSize: 10.5 }} disabled={!modelPath} onClick={() => void doSlice()}>
        slice
      </button>
      {sliceMsg && <span style={{ color: "var(--text-dim)" }}>{sliceMsg}</span>}
    </span>
  );
}

export default App;
