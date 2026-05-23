import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ViewportCanvas } from "./viewport/ViewportCanvas";
import { SlicePanel } from "./slice/SlicePanel";
import { PlateTabs } from "./plates/PlateTabs";
import { useProjectSession } from "./project/useProjectSession";
import {
  SettingsPanelHost,
  useSettingsPanelVisible,
} from "./settings/SettingsPanelHost";
import "./App.css";

type SlicerInfo = { version: string; option_count: number };

function App() {
  const [info, setInfo] = useState<SlicerInfo | null>(null);
  const [showDebug, setShowDebug] = useState(false);
  const [panelVisible, setPanelVisible] = useSettingsPanelVisible();
  const session = useProjectSession();

  useEffect(() => {
    invoke<SlicerInfo>("slicer_info")
      .then(setInfo)
      .catch(() => undefined);
  }, []);

  return (
    <div className="app">
      <header className="topbar">
        <span className="brand">
          <span className="brand-mark" aria-hidden />
          n3o-slic3r
        </span>
        <span className="tb-spacer" />
        <SlicePanel />
        {info && (
          <span style={{ color: "var(--text-dim)", fontFamily: "var(--font-mono)", fontSize: "10.5px" }}>
            {info.version} · {info.option_count} options
          </span>
        )}
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
      ) : (
        <div className={`workspace ${panelVisible ? "" : "no-panel"}`}>
          <main style={{ position: "relative", minWidth: 0 }}>
            <ViewportCanvas />
          </main>
          {panelVisible && <SettingsPanelHost session={session} />}
        </div>
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
