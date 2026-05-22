import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import "./App.css";

type SlicerInfo = { version: string; option_count: number };

type OptionSummary = {
  key: string;
  ty: string;
  label: string | null;
  category: string | null;
  default_value: string | null;
};

type SliceResult = { ok: boolean; out_path: string; error: string | null };

function App() {
  const [info, setInfo] = useState<SlicerInfo | null>(null);
  const [filter, setFilter] = useState("");
  const [options, setOptions] = useState<OptionSummary[]>([]);
  const [modelPath, setModelPath] = useState("");
  const [outPath, setOutPath] = useState("/tmp/n3o-out.gcode");
  const [sliceMsg, setSliceMsg] = useState("");

  useEffect(() => {
    invoke<SlicerInfo>("slicer_info").then(setInfo).catch((e) => setSliceMsg(`init: ${e}`));
  }, []);

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
    <main style={{ padding: 16, fontFamily: "system-ui, sans-serif" }}>
      {/* Tailwind smoke — proves the pipeline is wired. Phase 4 restyles the UI properly. */}
      <h1 className="text-2xl font-semibold tracking-tight">n3o-slic3r</h1>
      {info ? (
        <p style={{ opacity: 0.7, fontSize: 13 }}>
          {info.version} · {info.option_count} options registered
        </p>
      ) : (
        <p>loading…</p>
      )}

      <section style={{ marginTop: 24 }}>
        <h2 style={{ fontSize: 16 }}>Option introspection</h2>
        <form
          onSubmit={(e) => {
            e.preventDefault();
            loadOptions();
          }}
          style={{ display: "flex", gap: 8 }}
        >
          <input
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
            placeholder="filter by key/label (e.g. perimeter)"
            style={{ flex: 1, padding: 4 }}
          />
          <button type="submit">Search</button>
        </form>
        {options.length > 0 && (
          <table style={{ width: "100%", marginTop: 12, fontSize: 12, borderCollapse: "collapse" }}>
            <thead>
              <tr style={{ textAlign: "left", borderBottom: "1px solid #888" }}>
                <th>Key</th>
                <th>Type</th>
                <th>Label</th>
                <th>Category</th>
                <th>Default</th>
              </tr>
            </thead>
            <tbody>
              {options.map((o) => (
                <tr key={o.key} style={{ borderBottom: "1px solid #333" }}>
                  <td><code>{o.key}</code></td>
                  <td>{o.ty}</td>
                  <td>{o.label ?? ""}</td>
                  <td>{o.category ?? ""}</td>
                  <td><code>{o.default_value ?? ""}</code></td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </section>

      <section style={{ marginTop: 32 }}>
        <h2 style={{ fontSize: 16 }}>Slice a model</h2>
        <p style={{ fontSize: 13, opacity: 0.7 }}>
          Uses FullPrintConfig defaults; this smoke-tests that the IPC chain to libslic3r works.
        </p>
        <form
          onSubmit={(e) => {
            e.preventDefault();
            doSlice();
          }}
          style={{ display: "grid", gap: 6, maxWidth: 600 }}
        >
          <input
            value={modelPath}
            onChange={(e) => setModelPath(e.target.value)}
            placeholder="absolute path to .stl / .3mf / .obj / .step"
            style={{ padding: 4 }}
          />
          <input
            value={outPath}
            onChange={(e) => setOutPath(e.target.value)}
            placeholder="output .gcode path"
            style={{ padding: 4 }}
          />
          <button type="submit" disabled={!modelPath}>Slice</button>
        </form>
        {sliceMsg && <p style={{ marginTop: 8, fontFamily: "monospace" }}>{sliceMsg}</p>}
      </section>
    </main>
  );
}

export default App;
