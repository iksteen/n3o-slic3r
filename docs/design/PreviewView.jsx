// PreviewView.jsx — sliced preview of the active plate.
//
// Visually distinct from Prepare so the mode switch reads as a real context
// shift: a top-down SVG view with each object rendered as its footprint
// filled with horizontal infill stripes in its material color. A vertical
// layer slider on the right "scans" through the print; objects above the
// current scan line ghost out.
//
// The right-hand panel is a slim slice-info sidebar (not the full settings
// panel) — print time, filament, per-material breakdown.

// ───────── Filament color resolution ─────────

function resolveObjectColor(obj, materialMap, slotMap, filaments) {
  const materialId = obj.materialId || "M1";
  const slotId = materialMap[materialId];
  const filId = slotMap[slotId];
  const fil = filaments.find(f => f.id === filId);
  return fil?.color || "#999";
}

// ───────── Object footprints ─────────
//
// Crude per-kind shape so the preview reads as "these are real parts" and
// not just identical squares. Coords are in plate-mm relative to the
// object's (x, y) center, before rotZ is applied.
function footprintFor(kind) {
  switch (kind) {
    case "stl_mount":
      return [
        [-22, -14], [22, -14], [22, 14], [10, 14], [10, 8], [-10, 8], [-10, 14], [-22, 14],
      ];
    case "calicube":
      return [[-10, -10], [10, -10], [10, 10], [-10, 10]];
    case "stl_bracket":
      return [
        [-18, -10], [18, -10], [18, 10], [4, 10], [0, 18], [-4, 10], [-18, 10],
      ];
    default:
      return [[-12, -10], [12, -10], [12, 10], [-12, 10]];
  }
}

function pathFor(obj) {
  const pts = footprintFor(obj.kind);
  const c = Math.cos(obj.rotZ || 0), s = Math.sin(obj.rotZ || 0);
  return pts.map(([px, py], i) => {
    const x = px * c - py * s + (obj.x || 0);
    const y = px * s + py * c + (obj.y || 0);
    return `${i === 0 ? "M" : "L"}${x.toFixed(1)},${y.toFixed(1)}`;
  }).join(" ") + " Z";
}

function bboxOf(obj) {
  const pts = footprintFor(obj.kind);
  const c = Math.cos(obj.rotZ || 0), s = Math.sin(obj.rotZ || 0);
  let xMin = Infinity, xMax = -Infinity, yMin = Infinity, yMax = -Infinity;
  pts.forEach(([px, py]) => {
    const x = px * c - py * s + (obj.x || 0);
    const y = px * s + py * c + (obj.y || 0);
    if (x < xMin) xMin = x; if (x > xMax) xMax = x;
    if (y < yMin) yMin = y; if (y > yMax) yMax = y;
  });
  return { xMin, xMax, yMin, yMax };
}

// ───────── SlicedPreview viewport ─────────

function SlicedPreview({ plateSize, objects, filaments, materialMap, slotMap, layer, totalLayers }) {
  const [W, D] = plateSize;
  const PAD = 40; // px outside the plate
  // SVG viewBox uses plate coords centered on origin.
  const vbW = W + PAD * 2;
  const vbH = D + PAD * 2;

  // Scan progress: 0 = nothing shown, 1 = everything shown.
  const progress = totalLayers > 1 ? layer / (totalLayers - 1) : 1;

  return (
    <svg
      className="slice-canvas"
      viewBox={`${-vbW / 2} ${-vbH / 2} ${vbW} ${vbH}`}
      preserveAspectRatio="xMidYMid meet"
    >
      <defs>
        <pattern id="grid-cells" x="0" y="0" width="10" height="10" patternUnits="userSpaceOnUse">
          <path d="M10 0H0V10" fill="none" stroke="var(--slice-grid)" strokeWidth="0.4"/>
        </pattern>
        <pattern id="grid-major" x="0" y="0" width="50" height="50" patternUnits="userSpaceOnUse">
          <path d="M50 0H0V50" fill="none" stroke="var(--slice-grid-major)" strokeWidth="0.8"/>
        </pattern>
        {/* Per-color infill hatch patterns are generated below per object. */}
      </defs>

      {/* Plate */}
      <rect x={-W / 2} y={-D / 2} width={W} height={D} fill="var(--slice-bed)" stroke="var(--slice-bed-edge)" strokeWidth="1.5"/>
      <rect x={-W / 2} y={-D / 2} width={W} height={D} fill="url(#grid-cells)"/>
      <rect x={-W / 2} y={-D / 2} width={W} height={D} fill="url(#grid-major)"/>

      {/* Origin marker */}
      <g opacity="0.5">
        <circle cx="0" cy="0" r="1.4" fill="var(--slice-origin)"/>
        <path d="M-4 0H4M0 -4V4" stroke="var(--slice-origin)" strokeWidth="0.6"/>
      </g>

      {/* Objects */}
      {objects.map((obj, i) => {
        const color = resolveObjectColor(obj, materialMap, slotMap, filaments);
        const bb = bboxOf(obj);
        // Mock per-object "completion height" so the layer slider has
        // something to act on. Spread objects across the layer range.
        const objectHeight = 25 + (i % 3) * 8; // mm
        const objectStartLayer = Math.floor((i * totalLayers) / (objects.length + 1) * 0.15);
        const objectEndLayer = objectStartLayer + Math.floor(objectHeight / 0.2);
        const objProgress = Math.min(1, Math.max(0, (layer - objectStartLayer) / Math.max(1, objectEndLayer - objectStartLayer)));
        const visible = objProgress > 0;
        if (!visible) return null;

        const wallId = `wall-${i}`;
        const infillId = `infill-${i}`;
        const path = pathFor(obj);

        return (
          <g key={obj.id} className="slice-object" style={{ opacity: 0.65 + objProgress * 0.35 }}>
            <defs>
              <clipPath id={wallId}>
                <path d={path}/>
              </clipPath>
              <pattern
                id={infillId}
                x="0" y="0"
                width="2.4" height="2.4"
                patternUnits="userSpaceOnUse"
                patternTransform={`rotate(${(i % 2 === 0 ? 45 : -45)})`}
              >
                <line x1="0" y1="0" x2="0" y2="2.4" stroke={color} strokeWidth="1.1" opacity="0.95"/>
              </pattern>
            </defs>

            {/* Infill */}
            <g clipPath={`url(#${wallId})`}>
              <rect x={bb.xMin - 2} y={bb.yMin - 2}
                    width={bb.xMax - bb.xMin + 4} height={bb.yMax - bb.yMin + 4}
                    fill={`url(#${infillId})`}/>
              {/* Walls: concentric inset outlines */}
              {[0.6, 1.6, 2.6].map(inset => (
                <path key={inset} d={path} fill="none"
                      stroke={color} strokeWidth="0.7" opacity="0.85"/>
              ))}
            </g>

            {/* Hard outline */}
            <path d={path} fill="none" stroke={color} strokeWidth="1.4"
                  style={{ filter: "drop-shadow(0 0 0.4px rgba(0,0,0,0.2))" }}/>

            {/* Currently-printing scan line marker on the topmost object */}
            {objProgress < 1 && (
              <g clipPath={`url(#${wallId})`}>
                <rect x={bb.xMin - 2}
                      y={bb.yMin + (bb.yMax - bb.yMin) * (1 - objProgress) - 0.5}
                      width={bb.xMax - bb.xMin + 4}
                      height="1"
                      fill="var(--slice-scan)"
                      opacity="0.9"/>
              </g>
            )}
          </g>
        );
      })}

      {/* Plate dimensions */}
      <g className="slice-dims" style={{ pointerEvents: "none" }}>
        <text x="0" y={D / 2 + 18} textAnchor="middle" fontSize="9" fill="var(--slice-label)" fontFamily="var(--font-mono)">
          {W} mm
        </text>
        <text x={-W / 2 - 14} y="2" textAnchor="middle" fontSize="9" fill="var(--slice-label)" fontFamily="var(--font-mono)"
              transform={`rotate(-90, ${-W / 2 - 14}, 2)`}>
          {D} mm
        </text>
      </g>
    </svg>
  );
}

// ───────── Slice info panel (replaces SettingsPanel in Preview) ─────────

function PreviewInfoPanel({ plate, sliceStats, filaments, slotMap, materialMap, onSendToPrinter, onReslice }) {
  // Per-material breakdown derived from sliceStats.
  return (
    <aside className="preview-info">
      <div className="preview-info-header">
        <div className="preview-info-eyebrow">Sliced result</div>
        <div className="preview-info-title">{plate.name}</div>
        <div className="preview-info-sub">
          <span>{plate.printer}</span>
          <span className="dim">·</span>
          <span>{plate.nozzle}</span>
        </div>
      </div>

      <div className="preview-stat-grid">
        <div className="preview-stat">
          <div className="preview-stat-label">Print time</div>
          <div className="preview-stat-value">{sliceStats.timeLabel}</div>
        </div>
        <div className="preview-stat">
          <div className="preview-stat-label">Filament</div>
          <div className="preview-stat-value">{sliceStats.totalWeight}g</div>
          <div className="preview-stat-sub">{sliceStats.totalLength}m</div>
        </div>
        <div className="preview-stat">
          <div className="preview-stat-label">Layers</div>
          <div className="preview-stat-value">{sliceStats.layers}</div>
          <div className="preview-stat-sub">{sliceStats.layerHeight} mm</div>
        </div>
        <div className="preview-stat">
          <div className="preview-stat-label">Cost</div>
          <div className="preview-stat-value">${sliceStats.cost}</div>
        </div>
      </div>

      <div className="preview-section-title">By material</div>
      <div className="preview-mat-list">
        {sliceStats.byMaterial.map((m) => (
          <div className="preview-mat-row" key={m.slotId}>
            <span className="preview-mat-dot" style={{ background: m.color }}/>
            <span className="preview-mat-slot">{(m.slotLabel || m.slotId).toUpperCase()}</span>
            <span className="preview-mat-name" title={m.filament}>{m.filament}</span>
            <span className="preview-mat-weight">{m.weight}g</span>
          </div>
        ))}
      </div>

      <div className="preview-section-title">Bounding box</div>
      <div className="preview-bbox">
        <span>{sliceStats.bbox.x.toFixed(0)} × {sliceStats.bbox.y.toFixed(0)} × {sliceStats.bbox.z.toFixed(0)} mm</span>
        <span className="dim">{sliceStats.bbox.objects} obj</span>
      </div>

      <div className="preview-actions">
        <button className="preview-btn ghost" onClick={onReslice}>
          <svg width="11" height="11" viewBox="0 0 12 12" fill="none">
            <path d="M2 6a4 4 0 1 0 1.2-2.8M2 1.5v3h3" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round" strokeLinejoin="round"/>
          </svg>
          Re-slice
        </button>
        <button className="preview-btn primary" onClick={onSendToPrinter}>
          <svg width="11" height="11" viewBox="0 0 12 12" fill="none">
            <path d="M2 7l4 4 4-4M6 1v10" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round"/>
          </svg>
          Send to printer
        </button>
      </div>
    </aside>
  );
}

// ───────── PreviewView root ─────────

function PreviewView({ plate, filaments, onSendToPrinter, onReslice, onExitPreview, objectsPanel }) {
  const { useState, useMemo } = React;

  // Mock slice stats derived from the plate's actual contents.
  const sliceStats = useMemo(() => buildSliceStats(plate, filaments), [plate, filaments]);

  // Layer scrubber state. Starts at the end so the user sees the full result.
  const totalLayers = sliceStats.layers;
  const [layer, setLayer] = useState(totalLayers - 1);
  // Currently-shown layer height in mm.
  const layerZ = (layer * 0.2).toFixed(2);
  // Estimated elapsed time at this layer.
  const elapsedMin = Math.round((layer / Math.max(1, totalLayers)) * sliceStats.timeMinutes);

  return (
    <div className="preview-workspace">
      {objectsPanel}
      <div className="viewport viewport-preview">
        <SlicedPreview
          plateSize={plate.plateSize}
          objects={plate.objects}
          filaments={filaments}
          materialMap={plate.materialMap || {}}
          slotMap={plate.slotMap || {}}
          layer={layer}
          totalLayers={totalLayers}
        />

        {/* Viewport toolbar mirrors the editor: the Prepare/Preview toggle
           is the per-plate view switch and lives in the same spot in both
           modes so the user keeps their footing. The "sliced · top-down"
           badge is now a sibling of the toggle inside the same toolbar to
           keep them aligned. */}
        <div className="viewport-toolbar">
          <div className="vp-mode-toggle" role="tablist">
            <button className="vp-mode" onClick={onExitPreview} role="tab" aria-selected="false">
              Prepare
            </button>
            <button className="vp-mode active" role="tab" aria-selected="true">
              Preview
            </button>
          </div>
        </div>

        {/* Top-left mode badge */}
        <div className="preview-mode-badge">
          <span className="preview-mode-dot"/>
          Sliced · top-down
        </div>

        {/* Layer slider */}
        <div className="layer-scrubber">
          <div className="layer-scrubber-readout">
            <div className="layer-scrubber-num">{layer + 1} <span className="dim">/ {totalLayers}</span></div>
            <div className="layer-scrubber-z">Z {layerZ} mm</div>
            <div className="layer-scrubber-time">~{Math.floor(elapsedMin / 60)}h {elapsedMin % 60}m</div>
          </div>
          <input
            type="range"
            min="0"
            max={totalLayers - 1}
            value={layer}
            onChange={(e) => setLayer(parseInt(e.target.value, 10))}
            className="layer-scrubber-input"
            orient="vertical"
          />
          <div className="layer-scrubber-axis-marks">
            {[0, 25, 50, 75, 100].map(p => (
              <div key={p} className="layer-scrubber-mark" style={{ bottom: `${p}%` }}>
                <span className="layer-scrubber-tick"/>
                <span className="layer-scrubber-tick-label">{Math.round(p * (totalLayers - 1) / 100)}</span>
              </div>
            ))}
          </div>
        </div>
      </div>

      <PreviewInfoPanel
        plate={plate}
        sliceStats={sliceStats}
        filaments={filaments}
        onSendToPrinter={onSendToPrinter}
        onReslice={onReslice}
      />
    </div>
  );
}

// ───────── Mock slice stats ─────────

function buildSliceStats(plate, filaments) {
  // Per-material grams: weight by the number of objects assigned to each
  // material id. Filament density assumed ~1.24 g/cm³, average object
  // volume ~12 cm³ — these are pure mocks for the prototype.
  const slotMap = plate.slotMap || {};
  const materialMap = plate.materialMap || {};
  const objs = plate.objects || [];
  const slotIds = Object.keys(slotMap);
  const { slotShortLabel } = window.SLICER_DATA;

  const byMaterialMap = {};
  objs.forEach((o, i) => {
    const matId = o.materialId || "M1";
    const slotId = materialMap[matId];
    if (!slotId) return;
    if (!byMaterialMap[slotId]) {
      const filId = slotMap[slotId];
      const fil = filaments.find(f => f.id === filId);
      byMaterialMap[slotId] = {
        slotId,
        slotLabel: slotShortLabel ? slotShortLabel(slotId, slotIds) : slotId,
        color: fil?.color || "#888",
        filament: fil?.label || fil?.name || "—",
        weight: 0, objects: 0,
      };
    }
    byMaterialMap[slotId].weight += 8 + (i % 3) * 4;
    byMaterialMap[slotId].objects += 1;
  });
  const byMaterial = Object.values(byMaterialMap);
  const totalWeight = byMaterial.reduce((n, m) => n + m.weight, 0) || 14;
  const totalLength = (totalWeight / 3).toFixed(1);
  // Time: ~3.2 min/g rough mock
  const timeMinutes = Math.round(totalWeight * 3.2);
  const hours = Math.floor(timeMinutes / 60);
  const mins = timeMinutes % 60;
  const timeLabel = hours > 0 ? `${hours}h ${mins}m` : `${mins} min`;
  const cost = (totalWeight * 0.045).toFixed(2);
  // Layer count from tallest object footprint (mocked)
  const layers = 40 + objs.length * 35;
  const bbox = {
    x: Math.min(plate.plateSize[0], 40 + objs.length * 18),
    y: Math.min(plate.plateSize[1], 40 + objs.length * 14),
    z: 18 + objs.length * 6,
    objects: objs.length,
  };
  return {
    timeLabel, timeMinutes,
    totalWeight, totalLength,
    layers, layerHeight: "0.20",
    cost, bbox,
    byMaterial,
  };
}

window.PreviewView = PreviewView;
