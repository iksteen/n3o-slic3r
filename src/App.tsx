import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ViewportCanvas } from "./viewport/ViewportCanvas";
import { SlicePanel } from "./slice/SlicePanel";
import { PlateTabs } from "./plates/PlateTabs";
import "./App.css";

type SlicerInfo = { version: string; option_count: number };

// Phase 4 will restyle this properly. For PR-2-9 the layout is the
// minimum needed to host the viewport plus a folded-away debug panel
// that the Phase 0 smoke relied on.
function App() {
  const [info, setInfo] = useState<SlicerInfo | null>(null);
  const [showDebug, setShowDebug] = useState(false);

  useEffect(() => {
    invoke<SlicerInfo>("slicer_info")
      .then(setInfo)
      .catch(() => undefined);
  }, []);

  return (
    <div className="flex flex-col h-screen w-screen bg-neutral-900 text-neutral-100">
      <header className="flex items-center justify-between px-4 py-2 border-b border-neutral-800">
        <h1 className="text-lg font-semibold tracking-tight">n3o-slic3r</h1>
        <div className="text-xs text-neutral-400 flex items-center gap-3">
          <SlicePanel />
          {info && (
            <span>
              {info.version} · {info.option_count} options
            </span>
          )}
          <button
            type="button"
            className="px-2 py-1 bg-neutral-800 hover:bg-neutral-700 rounded text-xs"
            onClick={() => setShowDebug((v) => !v)}
          >
            {showDebug ? "Hide debug" : "Debug"}
          </button>
        </div>
      </header>
      <PlateTabs />
      <main className="flex-1 relative">
        <ViewportCanvas />
      </main>
      {showDebug && <DebugPanel />}
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
  const [outPath, setOutPath] = useState("/tmp/n3o-out.gcode");
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

  return (
    <section className="border-t border-neutral-800 bg-neutral-900/95 p-4 max-h-72 overflow-auto text-sm">
      <div className="flex gap-6">
        <form
          onSubmit={(e) => {
            e.preventDefault();
            void loadOptions();
          }}
          className="flex-1 min-w-0"
        >
          <h2 className="text-xs uppercase tracking-wider text-neutral-400 mb-1">
            Option introspection
          </h2>
          <div className="flex gap-2 mb-2">
            <input
              value={filter}
              onChange={(e) => setFilter(e.target.value)}
              placeholder="filter by key/label"
              className="flex-1 bg-neutral-800 px-2 py-1 rounded text-xs"
            />
            <button
              type="submit"
              className="bg-neutral-700 hover:bg-neutral-600 px-3 py-1 rounded text-xs"
            >
              Search
            </button>
          </div>
          {options.length > 0 && (
            <table className="w-full text-xs">
              <thead className="text-neutral-400">
                <tr className="border-b border-neutral-800 text-left">
                  <th>Key</th>
                  <th>Type</th>
                  <th>Label</th>
                  <th>Category</th>
                  <th>Default</th>
                </tr>
              </thead>
              <tbody>
                {options.map((o) => (
                  <tr key={o.key} className="border-b border-neutral-900">
                    <td>
                      <code>{o.key}</code>
                    </td>
                    <td>{o.ty}</td>
                    <td>{o.label ?? ""}</td>
                    <td>{o.category ?? ""}</td>
                    <td>
                      <code>{o.default_value ?? ""}</code>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </form>
        <form
          onSubmit={(e) => {
            e.preventDefault();
            void doSlice();
          }}
          className="w-80 flex flex-col gap-2"
        >
          <h2 className="text-xs uppercase tracking-wider text-neutral-400">
            Slice a model
          </h2>
          <input
            value={modelPath}
            onChange={(e) => setModelPath(e.target.value)}
            placeholder="absolute path to .stl / .3mf / .obj / .step"
            className="bg-neutral-800 px-2 py-1 rounded text-xs"
          />
          <input
            value={outPath}
            onChange={(e) => setOutPath(e.target.value)}
            placeholder="output .gcode path"
            className="bg-neutral-800 px-2 py-1 rounded text-xs"
          />
          <button
            type="submit"
            disabled={!modelPath}
            className="bg-neutral-700 hover:bg-neutral-600 disabled:opacity-40 px-3 py-1 rounded text-xs"
          >
            Slice
          </button>
          {sliceMsg && (
            <p className="font-mono text-xs text-neutral-300">{sliceMsg}</p>
          )}
        </form>
      </div>
    </section>
  );
}

export default App;
