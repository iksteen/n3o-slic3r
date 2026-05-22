// app.jsx — main shell. Wires TopBar / PlateTabs / ObjectsPanel / BuildPlate / SettingsPanel
// + Tweaks panel for theme/accountability/search variants.
//
// State is plate-centric: a project contains multiple Plates, each owning its
// own printer/bed/nozzle/objects/project-overrides. Switching plate tabs
// switches the entire workspace.

const { useState, useRef, useEffect, useMemo, useCallback } = React;
const { CASCADE_LAYERS, LAYER_BY_ID, CATEGORIES, ALL_SETTINGS } = window.SLICER_DATA;

const TWEAK_DEFAULTS = /*EDITMODE-BEGIN*/{
  "theme": "light",
  "accent": "cyan",
  "accountability": "rule",
  "search": "instant",
  "density": "regular"
}/*EDITMODE-END*/;

// Pre-seed a small set of objects so the prototype shows life on first load.
const INITIAL_FILAMENTS = [
  { id: "fil_a", label: "PETG Cool Grey", material: "PETG", color: "#7A8794" },
  { id: "fil_b", label: "PLA Signal Red", material: "PLA",  color: "#C24B45" },
  { id: "fil_c", label: "TPU Cyan",       material: "TPU",  color: "#2BB6C2" },
  { id: "fil_d", label: "PLA Matte Black",material: "PLA",  color: "#2A2D33" },
];

const PRINTER_PRESETS = [
  { id: "voron_2_4_350", name: "Voron 2.4 — 350",  bedPlate: "Textured PEI",     nozzle: "0.4 mm brass CHT",   plateSize: [256, 256] },
  { id: "prusa_mk4",     name: "Prusa MK4",        bedPlate: "Satin Powder",     nozzle: "0.4 mm hardened",    plateSize: [250, 210] },
  { id: "bambu_x1c",     name: "Bambu X1C",        bedPlate: "Engineering PEI",  nozzle: "0.4 mm hardened",    plateSize: [256, 256] },
  { id: "ender_3_v3",    name: "Ender 3 V3 KE",    bedPlate: "PC Spring Steel",  nozzle: "0.4 mm brass",       plateSize: [220, 220] },
];

const INITIAL_PLATES = [
  {
    id: "plate_1",
    name: "Production batch",
    printer: "Voron 2.4 — 350",
    bedPlate: "Textured PEI",
    nozzle: "0.4 mm brass CHT",
    plateSize: [256, 256],
    objects: [
      {
        id: "obj_seed_1", name: "front_mount_v3.stl",   kind: "stl_mount",  x: -45, y: -30, rotZ: 0, filamentId: "fil_a",
        overrides: { infill_density: 45, wall_count: 5, print_temp: 235 },
      },
      {
        id: "obj_seed_2", name: "calibration_cube.stl", kind: "calicube",   x: 40,  y: -30, rotZ: 0, filamentId: "fil_b",
        overrides: { infill_density: 100, top_layers: 3, bottom_layers: 3, support_enable: false, print_speed: 30 },
      },
      {
        id: "obj_seed_3", name: "fan_bracket_r2.stl",   kind: "stl_bracket",x: -20, y: 40,  rotZ: 0.5, filamentId: "fil_c",
        overrides: { support_enable: true, support_density: 25, adhesion_type: "brim", brim_width: 12 },
      },
    ],
    userOverrides: { top_layers: { user: 6 } },
    selectedId: null,
  },
  {
    id: "plate_2",
    name: "TPU gaskets",
    printer: "Prusa MK4",
    bedPlate: "Satin Powder",
    nozzle: "0.6 mm hardened",
    plateSize: [250, 210],
    objects: [
      {
        id: "obj_p2_1", name: "gasket_outer.stl", kind: "torus", x: -40, y: 0, rotZ: 0, filamentId: "fil_c",
        overrides: { print_temp: 235, retract_dist: 0.4, print_speed: 25 },
      },
      {
        id: "obj_p2_2", name: "gasket_inner.stl", kind: "torus", x: 40,  y: 0, rotZ: 0, filamentId: "fil_c",
        overrides: { print_temp: 235, retract_dist: 0.4 },
      },
    ],
    userOverrides: {},
    selectedId: null,
  },
];

function App() {
  const [t, setTweak] = useTweaks(TWEAK_DEFAULTS);
  const [contextLayer, setContextLayer] = useState("project");
  const [plates, setPlates] = useState(INITIAL_PLATES);
  const [activePlateId, setActivePlateId] = useState(INITIAL_PLATES[0].id);
  const [filaments] = useState(INITIAL_FILAMENTS);
  const [slicing, setSlicing] = useState(false);
  const cameraResetRef = useRef(null);

  // Derived: active plate + its slot accessors
  const activePlate = useMemo(
    () => plates.find(p => p.id === activePlateId) || plates[0],
    [plates, activePlateId]
  );

  // Patch the active plate
  const patchPlate = useCallback((id, patch) => {
    setPlates(prev => prev.map(p => p.id === id ? { ...p, ...(typeof patch === "function" ? patch(p) : patch) } : p));
  }, []);

  // Wrappers that operate on the active plate so existing components don't need to know about plates
  const objects = activePlate.objects;
  const selectedId = activePlate.selectedId;
  const userOverrides = activePlate.userOverrides;
  const plateSize = activePlate.plateSize;
  const printer = activePlate.printer;
  const bedPlate = activePlate.bedPlate;
  const nozzle = activePlate.nozzle;

  const setObjects = useCallback((updater) => {
    patchPlate(activePlateId, p => ({ objects: typeof updater === "function" ? updater(p.objects) : updater }));
  }, [activePlateId, patchPlate]);
  const setSelectedId = useCallback((id) => {
    patchPlate(activePlateId, { selectedId: id });
    if (id) setContextLayer("object");
    else if (contextLayer === "object") setContextLayer("project");
  }, [activePlateId, patchPlate, contextLayer]);
  const setUserOverrides = useCallback((updater) => {
    patchPlate(activePlateId, p => ({ userOverrides: typeof updater === "function" ? updater(p.userOverrides) : updater }));
  }, [activePlateId, patchPlate]);

  // Plate management
  const addPlate = useCallback(() => {
    const preset = PRINTER_PRESETS[plates.length % PRINTER_PRESETS.length];
    const id = `plate_${Date.now()}`;
    setPlates(prev => [...prev, {
      id, name: `Plate ${prev.length + 1}`,
      printer: preset.name, bedPlate: preset.bedPlate, nozzle: preset.nozzle, plateSize: preset.plateSize,
      objects: [], userOverrides: {}, selectedId: null,
    }]);
    setActivePlateId(id);
  }, [plates.length]);
  const closePlate = useCallback((id) => {
    setPlates(prev => {
      if (prev.length <= 1) return prev;
      const next = prev.filter(p => p.id !== id);
      if (activePlateId === id) setActivePlateId(next[0].id);
      return next;
    });
  }, [activePlateId]);
  const renamePlate = useCallback((id, name) => {
    patchPlate(id, { name });
  }, [patchPlate]);
  const setPlatePrinter = useCallback((id, presetId) => {
    const preset = PRINTER_PRESETS.find(p => p.id === presetId);
    if (!preset) return;
    patchPlate(id, { printer: preset.name, bedPlate: preset.bedPlate, nozzle: preset.nozzle, plateSize: preset.plateSize });
  }, [patchPlate]);

  // Filaments actually used by current objects (with use-counts)
  const filamentsInUse = useMemo(() => {
    const counts = {};
    objects.forEach(o => { counts[o.filamentId] = (counts[o.filamentId] || 0) + 1; });
    return Object.entries(counts).map(([id, useCount]) => {
      const fil = filaments.find(f => f.id === id);
      return fil ? { ...fil, useCount } : null;
    }).filter(Boolean);
  }, [objects, filaments]);

  // Stubs for the swap actions (would open a profile picker in the real app)
  const noop = (label) => () => console.log(`[swap] ${label}`);

  // Object-level overrides live on each object. These helpers patch the
  // selected object's `overrides` map.
  const setObjectOverride = useCallback((settingId, value) => {
    if (!selectedId) return;
    setObjects(prev => prev.map(o => o.id === selectedId
      ? { ...o, overrides: { ...(o.overrides || {}), [settingId]: value } }
      : o
    ));
  }, [selectedId]);

  const resetObjectOverride = useCallback((settingId) => {
    if (!selectedId) return;
    setObjects(prev => prev.map(o => {
      if (o.id !== selectedId) return o;
      const next = { ...(o.overrides || {}) };
      delete next[settingId];
      return { ...o, overrides: next };
    }));
  }, [selectedId]);

  // Apply theme + accent tokens
  useEffect(() => {
    const root = document.documentElement;
    root.setAttribute("data-theme", t.theme);
    if (t.accent === "cyan") root.removeAttribute("data-accent");
    else root.setAttribute("data-accent", t.accent);
  }, [t.theme, t.accent]);

  // Context-aware target label
  const contextTarget = useMemo(() => {
    if (contextLayer === "object") {
      const sel = objects.find(o => o.id === selectedId);
      return sel ? sel.name : "Select an object";
    }
    if (contextLayer === "filament")    return filaments[0].label;
    if (contextLayer === "project")     return "untitled.3mf";
    if (contextLayer === "user")        return "Anders · Travel";
    if (contextLayer === "printer")     return "Voron 2.4 — 350mm";
    if (contextLayer === "build_plate") return "Textured PEI";
    if (contextLayer === "default")     return "Shipped defaults";
    return "";
  }, [contextLayer, selectedId, objects, filaments]);

  // Auto-switch to object context when an object is selected (gentle behavior)
  useEffect(() => {
    if (selectedId && contextLayer !== "object") {
      // don't auto-jump — but if user clicks object chip with selected, the panel reflects
    }
  }, [selectedId]);

  const countObjectOverrides = useCallback((objId) => {
    const obj = objects.find(o => o.id === objId);
    if (!obj || !obj.overrides) return 0;
    return Object.keys(obj.overrides).length;
  }, [objects]);

  const onSlice = () => {
    setSlicing(true);
    setTimeout(() => setSlicing(false), 1800);
  };

  const onResetCamera = () => {
    cameraResetRef.current?.();
  };

  // Status bar info
  const overrideCount = Object.values(userOverrides).reduce(
    (n, layers) => n + Object.keys(layers).length, 0
  );

  return (
    <div className="app">
      <TopBar
        projectName="untitled.3mf"
        onSlice={onSlice}
        onResetCamera={onResetCamera}
      />

      <PlateTabs
        plates={plates}
        activePlateId={activePlateId}
        setActivePlateId={setActivePlateId}
        addPlate={addPlate}
        closePlate={closePlate}
        renamePlate={renamePlate}
      />

      <div className="workspace">
        <ObjectsPanel
          objects={objects}
          setObjects={setObjects}
          selectedId={selectedId}
          setSelectedId={setSelectedId}
          filaments={filaments}
          printerName={activePlate.name}
          countObjectOverrides={countObjectOverrides}
          plateSize={plateSize}
        />

        <div className="viewport">
          <BuildPlate
            key={activePlateId}
            objects={objects}
            setObjects={setObjects}
            selectedId={selectedId}
            setSelectedId={setSelectedId}
            plateSize={plateSize}
            filaments={filaments}
            onCameraReset={cameraResetRef}
          />

          <div className="viewport-toolbar">
            <button className="vp-tool active" title="Move">
              <svg viewBox="0 0 14 14" fill="none">
                <path d="M7 1v12M1 7h12M7 1l-2 2M7 1l2 2M7 13l-2-2M7 13l2-2M1 7l2-2M1 7l2 2M13 7l-2-2M13 7l-2 2" stroke="currentColor" strokeWidth="1.2" strokeLinecap="round"/>
              </svg>
            </button>
            <button className="vp-tool" title="Rotate">
              <svg viewBox="0 0 14 14" fill="none">
                <path d="M2 7a5 5 0 1 0 1.5-3.5" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round"/>
                <path d="M1.5 1.5v3h3" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round"/>
              </svg>
            </button>
            <button className="vp-tool" title="Scale">
              <svg viewBox="0 0 14 14" fill="none">
                <rect x="2" y="2" width="6" height="6" stroke="currentColor" strokeWidth="1.2"/>
                <rect x="6" y="6" width="6" height="6" stroke="currentColor" strokeWidth="1.2"/>
              </svg>
            </button>
            <div className="vp-sep"/>
            <button className="vp-tool" title="Auto-arrange">
              <svg viewBox="0 0 14 14" fill="none">
                <rect x="1.5" y="1.5" width="4" height="4" stroke="currentColor" strokeWidth="1.2"/>
                <rect x="8.5" y="1.5" width="4" height="4" stroke="currentColor" strokeWidth="1.2"/>
                <rect x="1.5" y="8.5" width="4" height="4" stroke="currentColor" strokeWidth="1.2"/>
                <rect x="8.5" y="8.5" width="4" height="4" stroke="currentColor" strokeWidth="1.2"/>
              </svg>
            </button>
            <button className="vp-tool" title="Layer preview">
              <svg viewBox="0 0 14 14" fill="none">
                <path d="M1.5 4l5.5 3 5.5-3-5.5-3-5.5 3z" stroke="currentColor" strokeWidth="1.2" strokeLinejoin="round"/>
                <path d="M1.5 7l5.5 3 5.5-3M1.5 10l5.5 3 5.5-3" stroke="currentColor" strokeWidth="1.2" strokeLinejoin="round"/>
              </svg>
            </button>
          </div>

          <div className="gizmo-hint">
            Drag · LMB rotate · MMB pan · scroll zoom
          </div>

          <div className="viewport-corner">
            <div className="axes">
              <span className="axis axis-x">X</span>
              <span className="axis axis-y">Y</span>
              <span className="axis axis-z">Z</span>
            </div>
            <div>{plateSize[0]} × {plateSize[1]} × 250 mm</div>
          </div>

          {slicing && (
            <div style={{
              position: "absolute", inset: 0, background: "rgba(15,17,21,0.45)",
              display: "flex", alignItems: "center", justifyContent: "center",
              backdropFilter: "blur(2px)",
              zIndex: 20,
            }}>
              <div style={{
                background: "var(--surface)", padding: "18px 22px", borderRadius: 10,
                boxShadow: "var(--shadow-lg)",
                fontFamily: "var(--font-mono)", fontSize: 13,
                display: "flex", flexDirection: "column", gap: 8, minWidth: 280,
              }}>
                <div>Slicing {objects.length} object{objects.length !== 1 ? "s" : ""}…</div>
                <div style={{ height: 4, background: "var(--surface-3)", borderRadius: 2, overflow: "hidden" }}>
                  <div style={{
                    height: "100%", width: "60%", background: "var(--accent)",
                    animation: "progress 1.6s ease-in-out infinite",
                  }}/>
                </div>
                <div className="dim" style={{ fontSize: 11 }}>Walls · Top/Bottom · Infill · Support · G-code</div>
              </div>
            </div>
          )}
        </div>

        <SettingsPanel
          contextLayer={contextLayer}
          setContextLayer={setContextLayer}
          selectedObject={objects.find(o => o.id === selectedId) || null}
          setObjectOverride={setObjectOverride}
          resetObjectOverride={resetObjectOverride}
          accountabilityMode={t.accountability}
          searchMode={t.search}
          userOverrides={userOverrides}
          setUserOverrides={setUserOverrides}
          objects={objects}
          filaments={filaments}
          printer={printer}
          bedPlate={bedPlate}
          nozzle={nozzle}
          filamentsInUse={filamentsInUse}
          printerPresets={PRINTER_PRESETS}
          onSwapPrinter={(presetId) => setPlatePrinter(activePlateId, presetId)}
          onSwapBedPlate={noop("bedPlate")}
          onSwapNozzle={noop("nozzle")}
          onSwapFilament={(id) => console.log("[swap] filament", id)}
        />
      </div>

      <div className="statusbar">
        <span className="dot"/>
        <span>Ready</span>
        <span>·</span>
        <span>{objects.length} object{objects.length !== 1 ? "s" : ""} on plate</span>
        <span>·</span>
        <span>{overrideCount} active override{overrideCount !== 1 ? "s" : ""}</span>
        <span className="spacer"/>
        <span>Editing {LAYER_BY_ID[contextLayer].label.toLowerCase()} layer</span>
        <span>·</span>
        <span>v0.4.1-prototype</span>
      </div>

      <TweaksPanel>
        <TweakSection label="Accountability" />
        <TweakRadio  label="Source viz" value={t.accountability}
                     options={["rule", "breadcrumb", "ladder-only"]}
                     onChange={(v) => setTweak('accountability', v)} />
        <div style={{ fontSize: 10.5, color: "rgba(41,38,27,.55)", lineHeight: 1.45 }}>
          <b>rule</b> — left rule colored by origin · <b>breadcrumb</b> — adds inline cascade trail · <b>ladder-only</b> — rule + hover ladder only
        </div>

        <TweakSection label="Search" />
        <TweakRadio  label="Filter mode" value={t.search}
                     options={["instant", "scoped", "fuzzy"]}
                     onChange={(v) => setTweak('search', v)} />
        <div style={{ fontSize: 10.5, color: "rgba(41,38,27,.55)", lineHeight: 1.45 }}>
          <b>instant</b> — substring across all · <b>scoped</b> — only active category · <b>fuzzy</b> — sloppy match
        </div>

        <TweakSection label="Theme" />
        <TweakRadio  label="Mode" value={t.theme}
                     options={["light", "dark"]}
                     onChange={(v) => setTweak('theme', v)} />
        <TweakColor  label="Accent" value={t.accent === "cyan" ? "#2BB6C2" : t.accent === "ember" ? "#D97757" : t.accent === "violet" ? "#7A5AE0" : "#1F8A5B"}
                     options={["#2BB6C2", "#D97757", "#7A5AE0", "#1F8A5B"]}
                     onChange={(v) => {
                       const map = { "#2BB6C2": "cyan", "#D97757": "ember", "#7A5AE0": "violet", "#1F8A5B": "mint" };
                       setTweak('accent', map[v] || "cyan");
                     }} />
      </TweaksPanel>
    </div>
  );
}

const root = ReactDOM.createRoot(document.getElementById('app'));
root.render(<App/>);
