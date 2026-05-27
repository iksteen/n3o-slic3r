// app.jsx — main shell. Wires TopBar / PlateTabs / ObjectsPanel / BuildPlate / SettingsPanel
// + Tweaks panel for theme/accountability/search variants.
//
// State is plate-centric: a project contains multiple Plates, each owning its
// own printer/bed/nozzle/objects/project-overrides. Switching plate tabs
// switches the entire workspace.
//
// Printer model: users define named *printers* (e.g. "Garage A1") based on
// hardware *profiles* (e.g. "Bambu Lab A1 mini"). The app boots with no
// printers and prompts the user to add one — that flow also seeds the first
// plate.

const { useState, useRef, useEffect, useMemo, useCallback } = React;
const { CASCADE_LAYERS, LAYER_BY_ID, CATEGORIES, ALL_SETTINGS, computeSlotIds, slotShortLabel, resolveObjectFilament } = window.SLICER_DATA;

const TWEAK_DEFAULTS = /*EDITMODE-BEGIN*/{
  "theme": "light",
  "accent": "cyan",
  "accountability": "rule",
  "search": "instant",
  "density": "regular"
}/*EDITMODE-END*/;

// The user's filament *library* — anything they could potentially load. Each
// physical loadout slot (`ext`, `AMS:1`..`AMS:4`) points at one of these by id.
// Ids match what `filamentSlug(brand, product, colorName)` would generate so
// re-picking the same filament from the catalog dedupes against this entry.
const INITIAL_FILAMENTS = [
  { id: "generic_pla_pure_white",  brand: "Generic", product: "PLA",  material: "PLA",  label: "Generic PLA",  colorName: "Pure White",  color: "#F2EFE7", nozzleTemp: 215, bedTemp: 60 },
  { id: "generic_petg_cool_grey",  brand: "Generic", product: "PETG", material: "PETG", label: "Generic PETG", colorName: "Cool Grey",   color: "#7A8794", nozzleTemp: 240, bedTemp: 80 },
  { id: "generic_asa_matte_black", brand: "Generic", product: "ASA",  material: "ASA",  label: "Generic ASA",  colorName: "Matte Black", color: "#1A1B1D", nozzleTemp: 250, bedTemp: 100 },
];

// Slot → filament loadout. Seed depends on the printer's installed AMS
// units — see `buildInitialSlotMap` below.
function buildInitialSlotMap(printer) {
  const slotIds = computeSlotIds({
    extruders: printer.extruders || 1,
    amsUnits: printer.amsUnits || 0,
  });
  const map = {};
  slotIds.forEach((slotId, i) => {
    // Pick a sensible default per slot. AMS slots get assorted materials,
    // ext gets generic PLA. Anything beyond the seed list defaults to PLA.
    const isExt = slotId === "ext" || slotId.startsWith("ext:");
    if (isExt) {
      map[slotId] = "generic_pla_pure_white";
      return;
    }
    // AMS slot. Index within the AMS unit (1–4).
    const m = slotId.match(/^AMS-[A-Z]:(\d+)$/);
    const within = m ? parseInt(m[1], 10) : 1;
    map[slotId] = within === 4
      ? "generic_asa_matte_black"
      : within >= 2
        ? "generic_petg_cool_grey"
        : "generic_pla_pure_white";
  });
  return map;
}

// Material → slot binding. The .3mf file labels regions M1, M2, … (the
// project's logical materials); the user maps each to whichever slot they
// want it printed from. M1 by convention is the default for new objects.
function buildInitialMaterialMap(slotIds) {
  const firstAmsA = slotIds.find(id => /^AMS-A:1$/.test(id));
  const secondAmsA = slotIds.find(id => /^AMS-A:2$/.test(id));
  return {
    "M1": firstAmsA || slotIds[0],
    "M2": secondAmsA || firstAmsA || slotIds[0],
  };
}

// Hardware profiles — the catalog the "Add printer" dialog draws from.
// A profile is a template (manufacturer + model + defaults); a user printer
// is a NAMED instance of one of these.
//
// `extruders` — direct-drive toolheads on the machine (default 1).
// `amsMax` — maximum number of AMS units this printer can attach
//   (0 = AMS not supported; A1/A1 mini take 1 AMS lite; P1S/X1C take up to 4).
//   The user picks how many to install in the Add-Printer dialog.
const PRINTER_PROFILES = [
  // Bambu Lab
  { id: "bambu_a1_mini", brand: "Bambu Lab", brandShort: "BL", model: "A1 mini",   plateSize: [180,180,180], bedPlate: "Textured PEI",    nozzle: "0.4 mm stainless", extruders: 1, amsMax: 1, amsType: "AMS lite" },
  { id: "bambu_a1",      brand: "Bambu Lab", brandShort: "BL", model: "A1",        plateSize: [256,256,256], bedPlate: "Textured PEI",    nozzle: "0.4 mm stainless", extruders: 1, amsMax: 1, amsType: "AMS lite" },
  { id: "bambu_p1s",     brand: "Bambu Lab", brandShort: "BL", model: "P1S",       plateSize: [256,256,256], bedPlate: "Textured PEI",    nozzle: "0.4 mm stainless", extruders: 1, amsMax: 4, amsType: "AMS",       note: "Enclosed · up to 4 AMS units" },
  { id: "bambu_x1c",     brand: "Bambu Lab", brandShort: "BL", model: "X1 Carbon", plateSize: [256,256,256], bedPlate: "Engineering PEI", nozzle: "0.4 mm hardened",  extruders: 1, amsMax: 4, amsType: "AMS",       note: "Lidar · CF-capable · up to 4 AMS units" },
  // Snapmaker
  { id: "snapmaker_u1",      brand: "Snapmaker", brandShort: "SM", model: "U1",      plateSize: [200,200,200], bedPlate: "Magnetic PEI", nozzle: "0.4 mm hardened", extruders: 4, amsMax: 0, note: "4-tool quick-change" },
  { id: "snapmaker_artisan", brand: "Snapmaker", brandShort: "SM", model: "Artisan", plateSize: [400,400,400], bedPlate: "Glass",        nozzle: "0.4 mm brass",    extruders: 1, amsMax: 0 },
  // Prusa Research
  { id: "prusa_mk4",  brand: "Prusa", brandShort: "PR", model: "MK4",   plateSize: [250,210,220], bedPlate: "Satin Powder", nozzle: "0.4 mm hardened", extruders: 1, amsMax: 0 },
  { id: "prusa_mini", brand: "Prusa", brandShort: "PR", model: "MINI+", plateSize: [180,180,180], bedPlate: "Smooth PEI",   nozzle: "0.4 mm brass",    extruders: 1, amsMax: 0 },
  { id: "prusa_xl",   brand: "Prusa", brandShort: "PR", model: "XL",    plateSize: [360,360,360], bedPlate: "Satin Powder", nozzle: "0.4 mm hardened", extruders: 5, amsMax: 0, note: "Up to 5 toolheads" },
  // Voron (community)
  { id: "voron_2_4_350",     brand: "Voron", brandShort: "VO", model: "2.4 — 350",   plateSize: [350,350,350], bedPlate: "Textured PEI", nozzle: "0.4 mm brass CHT", extruders: 1, amsMax: 0 },
  { id: "voron_trident_250", brand: "Voron", brandShort: "VO", model: "Trident 250", plateSize: [250,250,250], bedPlate: "Textured PEI", nozzle: "0.4 mm brass",     extruders: 1, amsMax: 0 },
  // Creality
  { id: "creality_k1_max",  brand: "Creality", brandShort: "CR", model: "K1 Max",       plateSize: [300,300,300], bedPlate: "PEI",             nozzle: "0.4 mm hardened", extruders: 1, amsMax: 0 },
  { id: "creality_ender_3", brand: "Creality", brandShort: "CR", model: "Ender 3 V3 KE",plateSize: [220,220,250], bedPlate: "PC Spring Steel", nozzle: "0.4 mm brass",    extruders: 1, amsMax: 0 },
];

// Demo objects to seed the first plate so the slicer doesn't look empty after
// onboarding. Filament ids reference INITIAL_FILAMENTS.
const SEED_OBJECTS_FIRST_PLATE = [
  {
    id: "obj_seed_1", name: "front_mount_v3.stl",   kind: "stl_mount",  x: -45, y: -30, rotZ: 0, materialId: "M1",
    overrides: { infill_density: 45, wall_count: 5, print_temp: 235 },
  },
  {
    id: "obj_seed_2", name: "calibration_cube.stl", kind: "calicube",   x: 40,  y: -30, rotZ: 0, materialId: "M2",
    overrides: { infill_density: 100, top_layers: 3, bottom_layers: 3, support_enable: false, print_speed: 30 },
  },
  {
    id: "obj_seed_3", name: "fan_bracket_r2.stl",   kind: "stl_bracket",x: -20, y: 40,  rotZ: 0.5, materialId: "M1",
    overrides: { support_enable: true, support_density: 25, adhesion_type: "brim", brim_width: 12 },
  },
];

// Per-printer defaults — used when adding a new printer. Users can later
// customize these from the printer settings dialog.
const DEFAULT_START_GCODE = `; --- Start G-code ---
G28                  ; home all axes
G29                  ; auto bed leveling
M140 S[bed_temp]     ; set bed temp
M104 S[nozzle_temp]  ; set nozzle temp
M190 S[bed_temp]     ; wait for bed
M109 S[nozzle_temp]  ; wait for nozzle
G92 E0               ; zero extruder
G1 X5 Y20 Z0.3 F5000 ; move to purge line
G1 X5 Y200 E15 F1500 ; purge line
G92 E0               ; zero extruder
`;

const DEFAULT_END_GCODE = `; --- End G-code ---
M104 S0              ; cool nozzle
M140 S0              ; cool bed
G91                  ; relative positioning
G1 Z10 F600          ; lift nozzle
G90                  ; absolute positioning
G1 X0 Y220 F3000     ; park print
M84                  ; disable steppers
`;

const DEFAULT_LIMITS = {
  feedrateX: 500, feedrateY: 500, feedrateZ: 25, feedrateE: 100,
  acceleration: 5000, jerk: 20,
  minLayer: 0.08, maxLayer: 0.32,
};

// ─── Mocked printer-status seeds ───
//
// The Devices view needs varied fake data so users can see what each state
// looks like. seedStatusFor returns a status object for a freshly-added
// printer based on its index — first printer idle (so adding ONE printer
// doesn't look misleading), later ones get printing / error / idle on
// rotation just for demo color.
const MOCK_FILES = [
  "balcony-clip-v4.gcode", "desk-grommet.gcode", "fan-bracket-r2.gcode",
  "cable-cover.gcode", "calibration-cube.gcode", "spool-holder.gcode",
];
function seedStatusFor(idx, printer) {
  // Per-toolhead nozzles. Snapmaker U1 has 4, Prusa XL has up to 5;
  // most printers have 1. Each toolhead gets its own temp pill.
  const extruders = Math.max(1, printer?.extruders | 0 || 1);
  const idleNozzles = Array.from({ length: extruders }, () => ({ current: 24, target: 0 }));
  // Idle baseline used by all states (incl. printing/error — they extend it).
  const base = {
    status: "idle",
    progress: 0,
    currentJob: null,
    queue: [],
    nozzleTemps: idleNozzles,
    bedTemp:    { current: 23, target: 0 },
    fanSpeed: 0,
    error: null,
  };
  const slot = idx % 3;
  if (slot === 1) {
    // Printing — for multi-toolhead printers, mark only T1 as actively hot
    // and the rest as idle. That mirrors how real multi-toolhead printers
    // work: only the toolheads in use for the current layer are warm.
    const printingNozzles = idleNozzles.map((_, i) =>
      i === 0
        ? { current: 215, target: 220 }
        : { current: 24, target: 0 }
    );
    return {
      ...base,
      status: "printing",
      progress: 18 + Math.random() * 30,
      nozzleTemps: printingNozzles,
      bedTemp:    { current: 58,  target: 60 },
      fanSpeed: 80,
      currentJob: {
        name: MOCK_FILES[idx % MOCK_FILES.length],
        plateName: "Plate 1",
        plateId: null,
        startedAtLabel: "12 min ago",
        durationMs: 1000 * 60 * 90, // 90 min total
        etaLabel: new Date(Date.now() + 60 * 60 * 1000).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" }),
        layersTotal: 240,
      },
      queue: [
        { name: MOCK_FILES[(idx + 1) % MOCK_FILES.length], durationMs: 1000 * 60 * 45 },
      ],
    };
  }
  if (slot === 2) {
    return {
      ...base,
      status: "error",
      error: "Bed leveling failed at probe point 3",
    };
  }
  return base;
}

function App() {
  const [t, setTweak] = useTweaks(TWEAK_DEFAULTS);
  const [contextLayer, setContextLayer] = useState("project");
  const [printers, setPrinters] = useState([]);         // user-named printers
  const [plates, setPlates] = useState([]);             // plates require a printer
  const [activePlateId, setActivePlateId] = useState(null);
  const [filaments, setFilaments] = useState(INITIAL_FILAMENTS);
  const [slicing, setSlicing] = useState(false);
  const [showAddPrinter, setShowAddPrinter] = useState(false);
  const [addPrinterSeedId, setAddPrinterSeedId] = useState(null);
  const [editingPrinterId, setEditingPrinterId] = useState(null);
  const [filamentPickerSlot, setFilamentPickerSlot] = useState(null); // slotId currently being edited
  const cameraResetRef = useRef(null);

  // ─── Navigation: three primary modes ───
  // 'prepare' = editor, 'preview' = sliced result, 'devices' = fleet monitor.
  const [mode, setMode] = useState("prepare");
  const [selectedDeviceId, setSelectedDeviceId] = useState(null);
  const [showSendToPrinter, setShowSendToPrinter] = useState(false);

  // ─── Simulated per-printer runtime status ───
  // Keyed by printerId: { status, progress, currentJob, queue, nozzleTemp,
  // bedTemp, fanSpeed, error }. Mocked for the prototype — incremented by a
  // ticker below so the Devices monitor feels alive.
  const [printerStatus, setPrinterStatus] = useState({});

  // Tick every second to advance any printing jobs. Keeps the demo lively.
  useEffect(() => {
    const id = setInterval(() => {
      setPrinterStatus(prev => {
        let changed = false;
        const next = { ...prev };
        for (const [pid, st] of Object.entries(prev)) {
          if (st.status !== "printing") continue;
          // ~0.4% progress per second → ~4 min to complete in demo
          const progress = Math.min(100, st.progress + 0.4);
          // Drift temps toward target
          const drift = (cur, tgt) => cur + (tgt - cur) * 0.12;
          const nozzleTemps = st.nozzleTemps
            ? st.nozzleTemps.map(nt => ({ ...nt, current: drift(nt.current, nt.target) }))
            : st.nozzleTemps;
          const bedTemp = st.bedTemp ? { ...st.bedTemp, current: drift(st.bedTemp.current, st.bedTemp.target) } : st.bedTemp;
          if (progress >= 100) {
            // Job finished — promote next queue item, or go idle.
            const queue = st.queue || [];
            if (queue.length > 0) {
              const [nextJob, ...rest] = queue;
              next[pid] = { ...st, progress: 0, currentJob: nextJob, queue: rest };
            } else {
              next[pid] = {
                ...st, status: "idle", progress: 0, currentJob: null,
                nozzleTemps: nozzleTemps.map(nt => ({ ...nt, target: 0 })),
                bedTemp: { current: bedTemp.current, target: 0 },
                fanSpeed: 0,
              };
            }
          } else {
            next[pid] = { ...st, progress, nozzleTemps, bedTemp };
          }
          changed = true;
        }
        return changed ? next : prev;
      });
    }, 1000);
    return () => clearInterval(id);
  }, []);

  // Apply theme + accent tokens
  useEffect(() => {
    const root = document.documentElement;
    root.setAttribute("data-theme", t.theme);
    if (t.accent === "cyan") root.removeAttribute("data-accent");
    else root.setAttribute("data-accent", t.accent);
  }, [t.theme, t.accent]);

  // Cmd/Ctrl+N opens the Add-Printer dialog from anywhere in the app.
  useEffect(() => {
    const onKey = (e) => {
      const isAddPrinter = (e.metaKey || e.ctrlKey) && (e.key === "n" || e.key === "N");
      if (isAddPrinter && !e.shiftKey && !e.altKey) {
        e.preventDefault();
        setShowAddPrinter(true);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // Adding a printer is the user's first concrete action when the app boots
  // empty. If this is the first printer, also seed a starter plate so the
  // workspace feels alive.
  const handleAddPrinter = useCallback(({ profileId, name, amsUnits }) => {
    const profile = PRINTER_PROFILES.find(p => p.id === profileId);
    if (!profile) return;
    const printerId = `prn_${Date.now()}_${Math.random().toString(36).slice(2, 7)}`;
    const installedAms = Math.max(0, Math.min(amsUnits ?? 0, profile.amsMax || 0));
    const newPrinter = {
      id: printerId,
      name,
      profileId,
      profileLabel: `${profile.brand} ${profile.model}`,
      brand: profile.brand,
      brandShort: profile.brandShort,
      bedPlate: profile.bedPlate,
      nozzle: profile.nozzle,
      plateSize: profile.plateSize,
      extruders: profile.extruders || 1,
      amsMax: profile.amsMax || 0,
      amsType: profile.amsType || null,
      amsUnits: installedAms,
      startGcode: DEFAULT_START_GCODE,
      endGcode: DEFAULT_END_GCODE,
      limits: { ...DEFAULT_LIMITS },
    };
    setPrinters(prev => {
      const next = [...prev, newPrinter];
      if (prev.length === 0) {
        // First printer: seed an initial plate using it.
        const plateId = `plate_${Date.now()}`;
        const slotMap = buildInitialSlotMap(newPrinter);
        const slotIds = Object.keys(slotMap);
        setPlates([{
          id: plateId,
          name: "Plate 1",
          printerId,
          printer: name,
          bedPlate: profile.bedPlate,
          nozzle: profile.nozzle,
          plateSize: profile.plateSize,
          extruders: newPrinter.extruders,
          amsUnits: newPrinter.amsUnits,
          objects: SEED_OBJECTS_FIRST_PLATE,
          userOverrides: { top_layers: { user: 6 } },
          selectedId: null,
          slotMap,
          materialMap: buildInitialMaterialMap(slotIds),
        }]);
        setActivePlateId(plateId);
      }
      return next;
    });
    // Seed a runtime status entry for the new printer. For demo color we
    // vary the initial state across the first few printers: 1st idle,
    // 2nd printing, 3rd in error. Everything past that defaults to idle.
    setPrinterStatus(prev => {
      const idx = Object.keys(prev).length;
      const seed = seedStatusFor(idx, newPrinter);
      return { ...prev, [printerId]: seed };
    });
    if (!selectedDeviceId) setSelectedDeviceId(printerId);
    setShowAddPrinter(false);
    setAddPrinterSeedId(null);
  }, []);

  // Update a printer's settings. If the name changes, sync the display name
  // into any plate that references this printer so plate tabs stay in sync.
  // If amsUnits changes, sync that too and prune any slotMap / materialMap
  // entries that now reference removed slots.
  const handleUpdatePrinter = useCallback((id, patch) => {
    setPrinters(prev => prev.map(p => p.id === id ? { ...p, ...patch } : p));
    if (patch.name || patch.nozzle || patch.amsUnits !== undefined) {
      setPlates(prev => prev.map(plate => {
        if (plate.printerId !== id) return plate;
        const next = { ...plate };
        if (patch.name)   next.printer = patch.name;
        if (patch.nozzle) next.nozzle  = patch.nozzle;
        if (patch.amsUnits !== undefined && patch.amsUnits !== plate.amsUnits) {
          next.amsUnits = patch.amsUnits;
          // Recompute valid slot ids and prune away anything no longer
          // physically present.
          const validIds = new Set(computeSlotIds({
            extruders: plate.extruders || 1,
            amsUnits: patch.amsUnits,
          }));
          const oldSlotMap = plate.slotMap || {};
          const newSlotMap = {};
          Object.entries(oldSlotMap).forEach(([k, v]) => {
            if (validIds.has(k)) newSlotMap[k] = v;
          });
          // If new slots appeared (AMS unit added), seed them with PLA.
          validIds.forEach(slotId => {
            if (!(slotId in newSlotMap)) {
              newSlotMap[slotId] = "generic_pla_pure_white";
            }
          });
          next.slotMap = newSlotMap;
          // Re-route any materialMap entries that pointed at a now-removed
          // slot. Fall back to the first valid slot.
          const oldMaterialMap = plate.materialMap || {};
          const fallback = validIds.values().next().value;
          const newMaterialMap = {};
          Object.entries(oldMaterialMap).forEach(([m, slotId]) => {
            newMaterialMap[m] = validIds.has(slotId) ? slotId : fallback;
          });
          next.materialMap = newMaterialMap;
        }
        return next;
      }));
    }
    setEditingPrinterId(null);
  }, []);

  // Delete a printer. Refuses to delete the last printer (need at least one).
  // Reassigns any plates that reference the deleted printer to the first
  // remaining printer, propagating its nozzle / bed / plate size.
  const handleDeletePrinter = useCallback((id) => {
    setPrinters(prev => {
      if (prev.length <= 1) return prev;
      const remaining = prev.filter(p => p.id !== id);
      const fallback = remaining[0];
      setPlates(plates => plates.map(plate => plate.printerId === id ? {
        ...plate,
        printerId: fallback.id,
        printer: fallback.name,
        bedPlate: fallback.bedPlate,
        nozzle: fallback.nozzle,
        plateSize: fallback.plateSize,
      } : plate));
      return remaining;
    });
    setPrinterStatus(prev => {
      const next = { ...prev };
      delete next[id];
      return next;
    });
    if (selectedDeviceId === id) setSelectedDeviceId(null);
    setEditingPrinterId(null);
  }, [selectedDeviceId]);

  // ─── Device control handlers ───
  const handlePauseDevice  = useCallback((id) => {
    setPrinterStatus(prev => prev[id] ? { ...prev, [id]: { ...prev[id], status: "paused" } } : prev);
  }, []);
  const handleResumeDevice = useCallback((id) => {
    setPrinterStatus(prev => prev[id] ? { ...prev, [id]: { ...prev[id], status: "printing" } } : prev);
  }, []);
  const handleStopDevice   = useCallback((id) => {
    setPrinterStatus(prev => prev[id] ? {
      ...prev,
      [id]: {
        ...prev[id],
        status: "idle", progress: 0, currentJob: null,
        nozzleTemps: (prev[id].nozzleTemps || []).map(nt => ({ ...nt, target: 0 })),
        bedTemp:    { ...prev[id].bedTemp,    target: 0 },
        fanSpeed: 0,
      },
    } : prev);
  }, []);

  // ─── Cross-axis: send the current plate's slice to a printer ───
  // Mocks queuing — if the target is idle, start immediately; if it's
  // printing, append to its queue. Then jump to Devices with the chosen
  // printer focused.
  const handleSendPlateToPrinter = useCallback((plateId, printerId) => {
    const plate = plates.find(p => p.id === plateId);
    if (!plate) return;
    const job = {
      name: `${plate.name.toLowerCase().replace(/\s+/g, "-")}.gcode`,
      plateId: plate.id,
      plateName: plate.name,
      startedAtLabel: "just now",
      durationMs: 1000 * 60 * (30 + plate.objects.length * 12),
      etaLabel: new Date(Date.now() + (30 + plate.objects.length * 12) * 60 * 1000)
        .toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" }),
      layersTotal: 40 + plate.objects.length * 35,
    };
    setPrinterStatus(prev => {
      const cur = prev[printerId] || seedStatusFor(0, null);
      if (cur.status === "printing" || cur.status === "paused") {
        // Queue behind the current job.
        return {
          ...prev,
          [printerId]: { ...cur, queue: [...(cur.queue || []), { name: job.name, durationMs: job.durationMs }] },
        };
      }
      // Start immediately. Heat only T1 by default — real per-extruder
      // targets would come from the slice header; this prototype keeps it
      // simple.
      const nozzleTemps = (cur.nozzleTemps || [{ current: 24, target: 0 }])
        .map((nt, i) => i === 0 ? { current: nt.current ?? 24, target: 220 } : { current: nt.current ?? 24, target: 0 });
      return {
        ...prev,
        [printerId]: {
          ...cur,
          status: "printing",
          progress: 0,
          currentJob: job,
          nozzleTemps,
          bedTemp:    { current: cur.bedTemp?.current ?? 23,    target: 60 },
          fanSpeed: 80,
          error: null,
        },
      };
    });
    setSelectedDeviceId(printerId);
    setShowSendToPrinter(false);
    setMode("devices");
  }, [plates]);

  // Jump from a device's current job back to its source plate in Prepare.
  const handleJumpToPlate = useCallback((plateId) => {
    if (!plateId) return;
    if (plates.some(p => p.id === plateId)) {
      setActivePlateId(plateId);
      setMode("prepare");
    }
  }, [plates]);

  // ─── Empty state route ───
  if (printers.length === 0) {
    return (
      <div className="app app-onboarding">
        <TopBar projectName="untitled.3mf"/>

        <PrintersEmptyState
          profiles={PRINTER_PROFILES}
          onAdd={() => setShowAddPrinter(true)}
        />

        <div className="statusbar onboarding-statusbar">
          <span className="dot warn"/>
          <span>No printers yet</span>
          <span>·</span>
          <span>Press <span className="kbd-inline">⌘ N</span> to add one</span>
          <span className="spacer"/>
          <span>n3o-slic3r · v0.4.1-prototype</span>
        </div>

        {showAddPrinter && (
          <AddPrinterModal
            profiles={PRINTER_PROFILES}
            existingNames={printers.map(p => p.name)}
            onAdd={handleAddPrinter}
            onClose={() => setShowAddPrinter(false)}
            initialProfileId={addPrinterSeedId}
          />
        )}

        <TweaksPanel>
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

  // ─── Normal app route ───
  return <SlicerWorkspace
    t={t} setTweak={setTweak}
    contextLayer={contextLayer} setContextLayer={setContextLayer}
    printers={printers} setPrinters={setPrinters}
    plates={plates} setPlates={setPlates}
    activePlateId={activePlateId} setActivePlateId={setActivePlateId}
    filaments={filaments}
    setFilaments={setFilaments}
    filamentPickerSlot={filamentPickerSlot}
    setFilamentPickerSlot={setFilamentPickerSlot}
    slicing={slicing} setSlicing={setSlicing}
    cameraResetRef={cameraResetRef}
    showAddPrinter={showAddPrinter} setShowAddPrinter={setShowAddPrinter}
    handleAddPrinter={handleAddPrinter}
    addPrinterSeedId={addPrinterSeedId} setAddPrinterSeedId={setAddPrinterSeedId}
    editingPrinterId={editingPrinterId} setEditingPrinterId={setEditingPrinterId}
    handleUpdatePrinter={handleUpdatePrinter}
    handleDeletePrinter={handleDeletePrinter}
    mode={mode} setMode={setMode}
    selectedDeviceId={selectedDeviceId} setSelectedDeviceId={setSelectedDeviceId}
    showSendToPrinter={showSendToPrinter} setShowSendToPrinter={setShowSendToPrinter}
    printerStatus={printerStatus}
    handlePauseDevice={handlePauseDevice}
    handleResumeDevice={handleResumeDevice}
    handleStopDevice={handleStopDevice}
    handleSendPlateToPrinter={handleSendPlateToPrinter}
    handleJumpToPlate={handleJumpToPlate}
  />;
}

function SlicerWorkspace({
  t, setTweak,
  contextLayer, setContextLayer,
  printers, setPrinters,
  plates, setPlates,
  activePlateId, setActivePlateId,
  filaments,
  setFilaments,
  filamentPickerSlot, setFilamentPickerSlot,
  slicing, setSlicing,
  cameraResetRef,
  showAddPrinter, setShowAddPrinter,
  handleAddPrinter,
  addPrinterSeedId, setAddPrinterSeedId,
  editingPrinterId, setEditingPrinterId,
  handleUpdatePrinter,
  handleDeletePrinter,
  mode, setMode,
  selectedDeviceId, setSelectedDeviceId,
  showSendToPrinter, setShowSendToPrinter,
  printerStatus,
  handlePauseDevice, handleResumeDevice, handleStopDevice,
  handleSendPlateToPrinter, handleJumpToPlate,
}) {
  // Derived: active plate + its slot accessors
  const activePlate = useMemo(
    () => plates.find(p => p.id === activePlateId) || plates[0],
    [plates, activePlateId]
  );

  const patchPlate = useCallback((id, patch) => {
    setPlates(prev => prev.map(p => p.id === id ? { ...p, ...(typeof patch === "function" ? patch(p) : patch) } : p));
  }, [setPlates]);

  const objects = activePlate.objects;
  const selectedId = activePlate.selectedId;
  const userOverrides = activePlate.userOverrides;
  const plateSize = activePlate.plateSize;
  const printer = activePlate.printer;
  const bedPlate = activePlate.bedPlate;
  const nozzle = activePlate.nozzle;
  const slotMap = activePlate.slotMap || {};
  const materialMap = activePlate.materialMap || {};
  const slotIds = useMemo(
    () => computeSlotIds({
      extruders: activePlate.extruders || 1,
      amsUnits: activePlate.amsUnits || 0,
    }),
    [activePlate.extruders, activePlate.amsUnits]
  );

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

  // Plate management — new plates re-use the most recent user printer.
  const addPlate = useCallback(() => {
    if (printers.length === 0) { setShowAddPrinter(true); return; }
    const lastPrinter = printers[printers.length - 1];
    const id = `plate_${Date.now()}`;
    const slotMap = buildInitialSlotMap(lastPrinter);
    const slotIds = Object.keys(slotMap);
    setPlates(prev => [...prev, {
      id, name: `Plate ${prev.length + 1}`,
      printerId: lastPrinter.id,
      printer: lastPrinter.name,
      bedPlate: lastPrinter.bedPlate, nozzle: lastPrinter.nozzle, plateSize: lastPrinter.plateSize,
      extruders: lastPrinter.extruders || 1,
      amsUnits: lastPrinter.amsUnits || 0,
      objects: [], userOverrides: {}, selectedId: null,
      slotMap,
      materialMap: buildInitialMaterialMap(slotIds),
    }]);
    setActivePlateId(id);
  }, [printers, setPlates, setActivePlateId, setShowAddPrinter]);
  const closePlate = useCallback((id) => {
    setPlates(prev => {
      if (prev.length <= 1) return prev;
      const next = prev.filter(p => p.id !== id);
      if (activePlateId === id) setActivePlateId(next[0].id);
      return next;
    });
  }, [activePlateId, setPlates, setActivePlateId]);
  const renamePlate = useCallback((id, name) => {
    patchPlate(id, { name });
  }, [patchPlate]);
  const setPlatePrinter = useCallback((id, printerId) => {
    if (printerId === "__new__") { setShowAddPrinter(true); return; }
    const userPrn = printers.find(p => p.id === printerId);
    if (!userPrn) return;
    patchPlate(id, {
      printerId: userPrn.id,
      printer: userPrn.name,
      bedPlate: userPrn.bedPlate, nozzle: userPrn.nozzle, plateSize: userPrn.plateSize,
      extruders: userPrn.extruders || 1,
      amsUnits: userPrn.amsUnits || 0,
      // drop any per-toolhead nozzle overrides — they don't apply to the new printer
      nozzles: undefined,
    });
  }, [patchPlate, printers, setShowAddPrinter]);

  // Filaments actually used by current objects (with use-counts). With the
  // two-step indirection the "thing in use" is the *material id* on the
  // object, not the filament — multiple materials can resolve to the same
  // slot, and a slot can be (re)bound to a different filament at any time.
  const materialsInUse = useMemo(() => {
    const counts = {};
    objects.forEach(o => {
      const k = o.materialId || "M1";
      counts[k] = (counts[k] || 0) + 1;
    });
    return Object.entries(counts)
      .sort(([a],[b]) => a.localeCompare(b))
      .map(([materialId, useCount]) => {
        const { slotId, filament } = resolveObjectFilament(
          { materialId }, materialMap, slotMap, filaments
        );
        return { materialId, slotId, filament, useCount };
      });
  }, [objects, filaments, materialMap, slotMap]);

  // Setters for the slot → filament loadout and the material → slot binding.
  const setSlotFilament = useCallback((slotId, filamentId) => {
    patchPlate(activePlateId, p => ({ slotMap: { ...(p.slotMap || {}), [slotId]: filamentId } }));
  }, [activePlateId, patchPlate]);
  const setMaterialSlot = useCallback((materialId, slotId) => {
    patchPlate(activePlateId, p => ({ materialMap: { ...(p.materialMap || {}), [materialId]: slotId } }));
  }, [activePlateId, patchPlate]);

  // Picker handler — receives a fully-described filament from the catalog,
  // registers it in the project library (deduping by id), and binds it to
  // the slot the picker was opened from.
  const pickFilamentForSlot = useCallback((slotId, picked) => {
    setFilaments(prev => {
      if (prev.some(f => f.id === picked.id)) return prev;
      return [...prev, picked];
    });
    patchPlate(activePlateId, p => ({
      slotMap: { ...(p.slotMap || {}), [slotId]: picked.id },
    }));
    setFilamentPickerSlot(null);
  }, [activePlateId, patchPlate, setFilaments, setFilamentPickerSlot]);

  const noop = (label) => () => console.log(`[swap] ${label}`);

  const setObjectOverride = useCallback((settingId, value) => {
    if (!selectedId) return;
    setObjects(prev => prev.map(o => o.id === selectedId
      ? { ...o, overrides: { ...(o.overrides || {}), [settingId]: value } }
      : o
    ));
  }, [selectedId, setObjects]);

  const resetObjectOverride = useCallback((settingId) => {
    if (!selectedId) return;
    setObjects(prev => prev.map(o => {
      if (o.id !== selectedId) return o;
      const next = { ...(o.overrides || {}) };
      delete next[settingId];
      return { ...o, overrides: next };
    }));
  }, [selectedId, setObjects]);

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

  const overrideCount = Object.values(userOverrides).reduce(
    (n, layers) => n + Object.keys(layers).length, 0
  );

  // Picker entries for SettingsPanel — user printers + an "Add new" sentinel.
  const printerPickerEntries = useMemo(() => ([
    ...printers.map(p => ({
      id: p.id, name: p.name, plateSize: p.plateSize, bedPlate: p.bedPlate, nozzle: p.nozzle,
      profileLabel: p.profileLabel,
    })),
    { id: "__new__", name: "+ New printer…", isAddNew: true },
  ]), [printers]);

  // Mode-aware top-bar actions + status badges.
  const printingCount = useMemo(
    () => Object.values(printerStatus).filter(s => s.status === "printing").length,
    [printerStatus]
  );
  const errorCount = useMemo(
    () => Object.values(printerStatus).filter(s => s.status === "error").length,
    [printerStatus]
  );

  let primaryAction = null;
  let secondaryAction = null;
  if (mode === "prepare") {
    primaryAction = {
      label: slicing ? "Slicing…" : "Slice",
      onClick: () => {
        onSlice();
        // Auto-flip to Preview when the slice finishes.
        setTimeout(() => setMode("preview"), 1850);
      },
      title: "Slice the active plate",
      disabled: slicing,
    };
  } else if (mode === "preview") {
    primaryAction = {
      label: "Send to printer",
      onClick: () => setShowSendToPrinter(true),
      title: "Queue this slice on one of your printers",
      icon: false,
    };
    secondaryAction = {
      label: "Re-slice",
      onClick: onSlice,
      title: "Re-run slicing with current settings",
    };
  }
  // devices mode: no global CTA — controls live per-printer in the monitor.

  return (
    <div className={`app app-mode-${mode}`}>
      <TopBar
        projectName="untitled.3mf"
        primary={primaryAction}
        secondary={secondaryAction}
      />

      <PlateTabs
        plates={plates}
        activePlateId={activePlateId}
        setActivePlateId={(id) => {
          setActivePlateId(id);
          if (mode === "devices") setMode("prepare");
        }}
        addPlate={() => { if (mode === "devices") setMode("prepare"); addPlate(); }}
        closePlate={closePlate}
        renamePlate={renamePlate}
        devicesActive={mode === "devices"}
        onSelectDevices={() => setMode("devices")}
        deviceCount={printers.length}
        printingCount={printingCount}
        errorCount={errorCount}
      />

      {mode === "prepare" && (
        <div className="workspace">
        <ObjectsPanel
          objects={objects}
          setObjects={setObjects}
          selectedId={selectedId}
          setSelectedId={setSelectedId}
          filaments={filaments}
          slotMap={slotMap}
          materialMap={materialMap}
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
            slotMap={slotMap}
            materialMap={materialMap}
            onCameraReset={cameraResetRef}
          />

          <div className="viewport-toolbar">
            {/* Prepare / Preview toggle — the per-plate "what am I looking
               at" switch. Lives in the canvas, not the topbar, because it's
               a property of THIS plate's view, not the whole app. */}
            <div className="vp-mode-toggle" role="tablist">
              <button
                className={`vp-mode ${mode === "prepare" ? "active" : ""}`}
                onClick={() => setMode("prepare")}
                title="Prepare — arrange objects, configure settings"
                role="tab"
                aria-selected={mode === "prepare"}
              >
                Prepare
              </button>
              <button
                className={`vp-mode ${mode === "preview" ? "active" : ""}`}
                onClick={() => setMode("preview")}
                title="Preview — inspect the sliced result"
                role="tab"
                aria-selected={mode === "preview"}
              >
                Preview
              </button>
            </div>
            <div className="vp-sep"/>
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
          extruders={activePlate.extruders || 1}
          nozzles={activePlate.nozzles}
          filamentsInUse={materialsInUse}
          materialsInUse={materialsInUse}
          slotIds={slotIds}
          slotMap={slotMap}
          materialMap={materialMap}
          onOpenSlotPicker={(slotId) => setFilamentPickerSlot(slotId)}
          onSyncFilaments={() => new Promise(r => setTimeout(r, 400))}
          setMaterialSlot={setMaterialSlot}
          printerPresets={printerPickerEntries}
          onSwapPrinter={(printerId) => setPlatePrinter(activePlateId, printerId)}
          onEditPrinter={(printerId) => setEditingPrinterId(printerId)}
          onSwapBedPlate={noop("bedPlate")}
          onSwapNozzle={noop("nozzle")}
          onSwapFilament={(id) => console.log("[swap] filament", id)}
        />
      </div>
      )}

      {mode === "preview" && activePlate && (
        <PreviewView
          plate={activePlate}
          filaments={filaments}
          onSendToPrinter={() => setShowSendToPrinter(true)}
          onReslice={onSlice}
          onExitPreview={() => setMode("prepare")}
          objectsPanel={
            <ObjectsPanel
              objects={objects}
              setObjects={setObjects}
              selectedId={selectedId}
              setSelectedId={setSelectedId}
              filaments={filaments}
              slotMap={slotMap}
              materialMap={materialMap}
              printerName={printer}
              countObjectOverrides={countObjectOverrides}
              plateSize={plateSize}
              readOnly={true}
            />
          }
        />
      )}

      {mode === "devices" && (
        <DevicesView
          printers={printers}
          statusMap={printerStatus}
          selectedDeviceId={selectedDeviceId || printers[0]?.id}
          setSelectedDeviceId={setSelectedDeviceId}
          onAddPrinter={() => setShowAddPrinter(true)}
          onEditPrinter={(id) => setEditingPrinterId(id)}
          onPause={handlePauseDevice}
          onResume={handleResumeDevice}
          onStop={handleStopDevice}
          onJumpToPlate={handleJumpToPlate}
        />
      )}

      <div className="statusbar">
        <span className={`dot ${errorCount > 0 ? "warn" : ""}`}/>
        <span>Ready</span>
        <span>·</span>
        <span>{printers.length} printer{printers.length !== 1 ? "s" : ""}</span>
        {mode === "devices" ? (
          <>
            <span>·</span>
            <span>{printingCount} printing</span>
            {errorCount > 0 && (<><span>·</span><span style={{ color: "var(--danger, #d6322a)" }}>{errorCount} error{errorCount !== 1 ? "s" : ""}</span></>)}
          </>
        ) : (
          <>
            <span>·</span>
            <span>{objects.length} object{objects.length !== 1 ? "s" : ""} on plate</span>
            <span>·</span>
            <span>{overrideCount} active override{overrideCount !== 1 ? "s" : ""}</span>
          </>
        )}
        <span className="spacer"/>
        {mode === "prepare" && (
          <>
            <span>Editing {LAYER_BY_ID[contextLayer].label.toLowerCase()} layer</span>
            <span>·</span>
          </>
        )}
        {mode === "preview" && (
          <>
            <span>Preview · sliced result</span>
            <span>·</span>
          </>
        )}
        {mode === "devices" && (
          <>
            <span>Fleet monitor</span>
            <span>·</span>
          </>
        )}
        <span>v0.4.1-prototype</span>
      </div>

      {showAddPrinter && (
        <AddPrinterModal
          profiles={PRINTER_PROFILES}
          existingNames={printers.map(p => p.name)}
          onAdd={handleAddPrinter}
          onClose={() => { setShowAddPrinter(false); setAddPrinterSeedId(null); }}
          initialProfileId={addPrinterSeedId}
        />
      )}

      {editingPrinterId && (
        <PrinterSettingsModal
          printer={printers.find(p => p.id === editingPrinterId)}
          allPrinters={printers}
          onSave={handleUpdatePrinter}
          onDelete={handleDeletePrinter}
          onClose={() => setEditingPrinterId(null)}
        />
      )}

      {filamentPickerSlot && (
        <FilamentPickerModal
          slotId={filamentPickerSlot}
          currentFilamentId={slotMap[filamentPickerSlot]}
          onPick={(filament) => pickFilamentForSlot(filamentPickerSlot, filament)}
          onClose={() => setFilamentPickerSlot(null)}
        />
      )}

      {showSendToPrinter && activePlate && (
        <SendToPrinterModal
          plate={activePlate}
          printers={printers}
          statusMap={printerStatus}
          onSend={(printerId) => handleSendPlateToPrinter(activePlate.id, printerId)}
          onClose={() => setShowSendToPrinter(false)}
        />
      )}

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
