// DevicesView.jsx — fleet monitoring.
//
// Plate tabs are hidden in this mode (set by app.jsx). Layout:
//
//   ┌──────────────┬────────────────────────────────────────────────────┐
//   │ Printer rail │  Per-printer monitor (camera, temps, job, queue)   │
//   └──────────────┴────────────────────────────────────────────────────┘
//
// The rail lists every configured printer with a status dot + (if printing)
// a thin progress strip. Selecting one fills the monitor on the right.
// Cross-axis: "Last printed from" lets users jump back to the originating
// plate in Prepare mode.

const { computeSlotIds: dvComputeSlotIds, slotShortLabel: dvSlotShortLabel, slotLongLabel: dvSlotLongLabel } = window.SLICER_DATA;

// ───────── Helpers ─────────

// Resolve which filament is spooled into each physical slot of a printer.
//
// The loadout is the printer's OWN driver status — the spools physically
// sitting in its AMS / on its toolheads, reported back over the connection.
// It lives on the printer object (`printer.loadout`: slotId → spool descriptor
// or null), independent of any plate. (The slicer's per-plate slotMap is a
// separate copy the user syncs FROM this.)
//
// Returns { known, slots: [{ slotId, isExt, shortLabel, longLabel, filament }],
// nozzleFilaments } where nozzleFilaments aligns 1:1 with the status'
// nozzleTemps for direct-extruder machines. On AMS printers the single nozzle
// is fed by whichever AMS spool is active, so its color is ambiguous → null.
function computeLoadout(printer, status) {
  const slotIds = dvComputeSlotIds({
    extruders: printer.extruders || 1,
    amsUnits: printer.amsUnits || 0,
  });
  const loadout = printer.loadout || null;
  const resolve = (sid) => (loadout && loadout[sid]) ? loadout[sid] : null;

  // The printer reports which slot is physically engaged at the nozzle — the
  // tool the grabber currently holds (multi-toolhead) or the spool primed up
  // to the hotend (AMS / single extruder). That's a persistent hardware state,
  // so it's reported whenever the printer is online, not just while printing.
  // `feeding` narrows that to "actually extruding right now" (printing).
  const offline = status && status.status === "offline";
  const activeSlotId = (status && !offline) ? (status.activeSlotId || null) : null;
  const feeding = status && status.status === "printing";
  const activeFilament = activeSlotId ? resolve(activeSlotId) : null;

  const slots = slotIds.map(sid => ({
    slotId: sid,
    isExt: sid === "ext" || sid.startsWith("ext:"),
    active: sid === activeSlotId,
    shortLabel: dvSlotShortLabel(sid, slotIds),
    longLabel: dvSlotLongLabel(sid),
    filament: resolve(sid),
  }));

  // Nozzle → filament mapping for the temp pills.
  //   • AMS machines have ONE nozzle fed by whichever AMS/ext spool is engaged,
  //     so its color is the active slot's filament (the primed spool, even idle).
  //   • Direct-extruder machines map each nozzle 1:1 to its toolhead's slot.
  const hasAms = (printer.amsUnits || 0) > 0;
  const extSlots = slots.filter(s => s.isExt);
  const nozzleFilaments = (status && status.nozzleTemps ? status.nozzleTemps : [])
    .map((_, i) => hasAms ? (i === 0 ? activeFilament : null) : (extSlots[i] ? extSlots[i].filament : null));

  return { known: !!loadout, slots, nozzleFilaments, hasAms, activeSlotId, activeFilament, feeding };
}

// One spool row in the loadout panel. The active (engaged) slot is marked with
// the accent row highlight — the filament type stays visible either way.
function LoadoutRow({ slot }) {
  const f = slot.filament;
  const active = slot.active && !!f;
  return (
    <div className={`device-loadout-row ${f ? "" : "empty"} ${active ? "active" : ""}`} title={f
      ? `${slot.longLabel} · ${f.brand || ""} ${f.product || f.label || ""}${f.colorName ? " (" + f.colorName + ")" : ""}${active ? " — currently engaged" : ""}`
      : `${slot.longLabel} — empty`}>
      <span className="device-loadout-swatch" style={{ background: f ? f.color : "transparent" }}/>
      <span className="device-loadout-slot">{slot.shortLabel}</span>
      <span className="device-loadout-name">{f ? (f.colorName || f.label) : "Empty"}</span>
      <span className="device-loadout-mat">{f ? f.material : "—"}</span>
    </div>
  );
}

// Filament loadout panel — what each physical slot is spooled with.
function LoadoutPanel({ status, loadout }) {
  if (status.status === "offline") {
    return (
      <div className="device-loadout">
        <div className="device-loadout-header">
          <span>Filament loadout</span>
        </div>
        <div className="device-loadout-empty dim">Unknown — printer offline.</div>
      </div>
    );
  }
  if (!loadout.known) {
    return (
      <div className="device-loadout">
        <div className="device-loadout-header">
          <span>Filament loadout</span>
        </div>
        <div className="device-loadout-empty dim">No recent slice on this printer — loadout unknown.</div>
      </div>
    );
  }
  const loadedCount = loadout.slots.filter(s => s.filament).length;
  return (
    <div className="device-loadout">
      <div className="device-loadout-header">
        <span>Filament loadout</span>
        <span className="device-loadout-count">{loadedCount}/{loadout.slots.length}</span>
      </div>
      <div className="device-loadout-list">
        {loadout.slots.map(s => <LoadoutRow key={s.slotId} slot={s}/>)}
      </div>
    </div>
  );
}

function statusMeta(status) {
  switch (status) {
    case "printing": return { label: "Printing", cls: "printing" };
    case "paused":   return { label: "Paused",   cls: "paused" };
    case "error":    return { label: "Error",    cls: "error" };
    case "offline":  return { label: "Offline",  cls: "offline" };
    case "idle":
    default:         return { label: "Idle",     cls: "idle" };
  }
}

function fmtDuration(ms) {
  const s = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  if (h > 0) return `${h}h ${String(m).padStart(2, "0")}m`;
  if (m > 0) return `${m}m ${String(sec).padStart(2, "0")}s`;
  return `${sec}s`;
}

// ───────── Printer rail (left) ─────────

function PrinterRail({ printers, statusMap, selectedId, setSelectedId, onAddPrinter }) {
  return (
    <aside className="device-rail">
      <div className="device-rail-header">
        <span>Printers</span>
        <span className="device-rail-count">{printers.length}</span>
      </div>
      <div className="device-rail-list">
        {printers.map(p => {
          const st = statusMap[p.id] || { status: "idle" };
          const meta = statusMeta(st.status);
          const isActive = p.id === selectedId;
          return (
            <button
              key={p.id}
              className={`device-card ${isActive ? "active" : ""}`}
              onClick={() => setSelectedId(p.id)}
              title={`${p.name} — ${meta.label}`}
            >
              <div className="device-card-row1">
                <span className={`device-status-dot ${meta.cls}`}/>
                <span className="device-card-name">{p.name}</span>
                <span className={`device-card-state ${meta.cls}`}>{meta.label}</span>
              </div>
              <div className="device-card-row2">
                <span className="device-card-model">{p.profileLabel}</span>
                {st.status === "printing" && st.currentJob && (
                  <span className="device-card-eta">{Math.round(st.progress)}%</span>
                )}
              </div>
              {st.status === "printing" && (
                <div className="device-card-progress">
                  <div className="device-card-progress-fill" style={{ width: `${st.progress}%` }}/>
                </div>
              )}
              {st.status === "error" && st.error && (
                <div className="device-card-error">{st.error}</div>
              )}
            </button>
          );
        })}
      </div>
      <button className="device-rail-add" onClick={onAddPrinter}>
        <svg width="11" height="11" viewBox="0 0 12 12" fill="none">
          <path d="M6 2v8M2 6h8" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round"/>
        </svg>
        Add printer
      </button>
    </aside>
  );
}

// ───────── Camera mock ─────────

function CameraPanel({ device, status }) {
  const live = status.status === "printing" || status.status === "paused";
  return (
    <div className={`device-camera ${live ? "live" : "off"}`}>
      <div className="device-camera-frame">
        {live ? (
          <>
            <div className="device-camera-scene">
              {/* faux plate-from-above gradient */}
              <div className="device-camera-bed"/>
              <div className="device-camera-toolhead" style={{
                left: `${30 + Math.sin(status.progress * 0.4) * 22}%`,
                top:  `${42 + Math.cos(status.progress * 0.3) * 18}%`,
              }}/>
              <div className="device-camera-scanlines"/>
            </div>
            <div className="device-camera-rec">
              <span className="device-camera-rec-dot"/> LIVE
            </div>
            <div className="device-camera-time">
              {new Date().toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" })}
            </div>
          </>
        ) : (
          <div className="device-camera-off-msg">
            <svg width="32" height="32" viewBox="0 0 32 32" fill="none" opacity="0.4">
              <rect x="3" y="8" width="20" height="16" rx="2" stroke="currentColor" strokeWidth="1.5"/>
              <path d="M23 13l6-3v12l-6-3z" stroke="currentColor" strokeWidth="1.5" strokeLinejoin="round"/>
              <path d="M3 3l26 26" stroke="currentColor" strokeWidth="1.5"/>
            </svg>
            <div>Camera offline</div>
            <div className="dim">{device.name} is {statusMeta(status.status).label.toLowerCase()}</div>
          </div>
        )}
      </div>
    </div>
  );
}

// ───────── Stats column ─────────

function TempPill({ label, current, target, compact, filament }) {
  const heating = current < target - 1;
  const cooling = current > target + 1 && target === 0;
  return (
    <div className={`device-temp ${compact ? "compact" : ""} ${heating ? "heating" : ""} ${cooling ? "cooling" : ""}`}>
      <div className="device-temp-label">
        {filament !== undefined && (
          <span
            className={`device-temp-swatch ${filament ? "" : "empty"}`}
            style={{ background: filament ? filament.color : "transparent" }}
            title={filament
              ? `${filament.label}${filament.colorName ? " · " + filament.colorName : ""}`
              : "No filament loaded"}
          />
        )}
        {label}
      </div>
      <div className="device-temp-value">
        <span className="device-temp-current">{Math.round(current)}°</span>
        <span className="device-temp-arrow">→</span>
        <span className="device-temp-target">{Math.round(target)}°</span>
      </div>
      <div className="device-temp-bar">
        <div className="device-temp-bar-fill" style={{ width: `${Math.min(100, (current / Math.max(target, 60)) * 100)}%` }}/>
      </div>
    </div>
  );
}

function StatsColumn({ status, loadout }) {
  if (status.status === "offline") {
    return (
      <div className="device-stats device-stats-offline">
        <div className="dim">No telemetry — printer is offline.</div>
      </div>
    );
  }
  const nozzles = status.nozzleTemps || [];
  const nozzleFils = (loadout && loadout.nozzleFilaments) || [];
  const multi = nozzles.length > 1;
  return (
    <div className="device-stats">
      {multi ? (
        <div className="device-temp-group">
          <div className="device-temp-group-label">Nozzles</div>
          <div className={`device-temp-grid n-${nozzles.length}`}>
            {nozzles.map((nt, i) => (
              <TempPill key={i} label={`T${i + 1}`} current={nt.current ?? 24} target={nt.target ?? 0} compact filament={nozzleFils[i] || null}/>
            ))}
          </div>
        </div>
      ) : (
        <TempPill label="Nozzle" current={nozzles[0]?.current ?? 24} target={nozzles[0]?.target ?? 0} filament={loadout ? (nozzleFils[0] || null) : undefined}/>
      )}
      <TempPill label="Bed" current={status.bedTemp?.current ?? 23} target={status.bedTemp?.target ?? 0}/>
      <div className="device-fan">
        <span className="device-fan-label">Part fan</span>
        <span className="device-fan-value">{status.fanSpeed ?? 0}%</span>
        <div className="device-fan-bar">
          <div className="device-fan-bar-fill" style={{ width: `${status.fanSpeed ?? 0}%` }}/>
        </div>
      </div>
    </div>
  );
}

// ───────── Current job ─────────

function CurrentJobPanel({ status, onJumpToPlate }) {
  if (!status.currentJob) {
    return (
      <div className="device-job device-job-empty">
        <div className="device-job-empty-title">No job running</div>
        <div className="dim">Slice a plate and use <b>Send to printer</b> to start one.</div>
      </div>
    );
  }
  const job = status.currentJob;
  const elapsed = Math.round((status.progress / 100) * job.durationMs);
  const remaining = job.durationMs - elapsed;
  return (
    <div className="device-job">
      <div className="device-job-header">
        <div>
          <div className="device-job-name">{job.name}</div>
          <div className="device-job-meta">
            from{" "}
            <button className="device-job-jump" onClick={() => onJumpToPlate(job.plateId)}>
              {job.plateName}
            </button>
            <span className="dim"> · started {job.startedAtLabel}</span>
          </div>
        </div>
        <div className="device-job-percent">{Math.round(status.progress)}%</div>
      </div>
      <div className="device-job-progress">
        <div className="device-job-progress-fill" style={{ width: `${status.progress}%` }}/>
      </div>
      <div className="device-job-times">
        <div>
          <div className="device-job-time-label">Elapsed</div>
          <div className="device-job-time-value">{fmtDuration(elapsed)}</div>
        </div>
        <div>
          <div className="device-job-time-label">Remaining</div>
          <div className="device-job-time-value">{fmtDuration(remaining)}</div>
        </div>
        <div>
          <div className="device-job-time-label">Layer</div>
          <div className="device-job-time-value">{Math.round((status.progress / 100) * job.layersTotal)} / {job.layersTotal}</div>
        </div>
        <div>
          <div className="device-job-time-label">ETA</div>
          <div className="device-job-time-value">{job.etaLabel}</div>
        </div>
      </div>
    </div>
  );
}

// ───────── Monitor (right pane) ─────────

function DeviceMonitor({ device, status, onPause, onResume, onStop, onJumpToPlate, onEditPrinter }) {
  if (!device) {
    return (
      <div className="device-monitor device-monitor-empty">
        <div className="dim">Select a printer.</div>
      </div>
    );
  }
  const meta = statusMeta(status.status);
  const printing = status.status === "printing";
  const paused = status.status === "paused";
  const loadout = computeLoadout(device, status);
  return (
    <div className="device-monitor">
      <div className="device-monitor-header">
        <div className="device-monitor-title-block">
          <div className="device-monitor-eyebrow">{device.profileLabel}</div>
          <div className="device-monitor-title">{device.name}</div>
          <div className="device-monitor-sub">
            <span className={`device-status-dot ${meta.cls}`}/>
            <span className={`device-monitor-state ${meta.cls}`}>{meta.label}</span>
          </div>
        </div>

        <div className="device-monitor-controls">
          {printing && (
            <button className="device-ctl" onClick={() => onPause(device.id)}>
              <svg width="11" height="11" viewBox="0 0 12 12" fill="none">
                <rect x="3" y="2.5" width="2" height="7" fill="currentColor"/>
                <rect x="7" y="2.5" width="2" height="7" fill="currentColor"/>
              </svg>
              Pause
            </button>
          )}
          {paused && (
            <button className="device-ctl primary" onClick={() => onResume(device.id)}>
              <svg width="11" height="11" viewBox="0 0 12 12" fill="none">
                <path d="M3 2l7 4-7 4V2z" fill="currentColor"/>
              </svg>
              Resume
            </button>
          )}
          {(printing || paused) && (
            <button className="device-ctl danger" onClick={() => onStop(device.id)}>
              <svg width="11" height="11" viewBox="0 0 12 12" fill="none">
                <rect x="2.5" y="2.5" width="7" height="7" fill="currentColor"/>
              </svg>
              Stop
            </button>
          )}
          <button className="device-ctl ghost" title="Printer settings" onClick={() => onEditPrinter(device.id)}>
            <svg width="11" height="11" viewBox="0 0 14 14" fill="none">
              <circle cx="7" cy="7" r="2" stroke="currentColor" strokeWidth="1.3"/>
              <path d="M7 1v2M7 11v2M1 7h2M11 7h2M2.5 2.5l1.5 1.5M10 10l1.5 1.5M2.5 11.5L4 10M10 4l1.5-1.5" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round"/>
            </svg>
          </button>
        </div>
      </div>

      <div className="device-monitor-body">
        <div className="device-monitor-left">
          <CameraPanel device={device} status={status}/>
          <CurrentJobPanel status={status} onJumpToPlate={onJumpToPlate}/>
        </div>
        <div className="device-monitor-right">
          <StatsColumn status={status} loadout={loadout}/>
          <LoadoutPanel status={status} loadout={loadout}/>
        </div>
      </div>
    </div>
  );
}

// ───────── Root ─────────

function DevicesView({
  printers,
  statusMap,
  selectedDeviceId,
  setSelectedDeviceId,
  onAddPrinter,
  onEditPrinter,
  onPause, onResume, onStop,
  onJumpToPlate,
}) {
  const device = printers.find(p => p.id === selectedDeviceId) || printers[0];
  const status = statusMap[device?.id] || { status: "idle" };
  return (
    <div className="devices-view">
      <PrinterRail
        printers={printers}
        statusMap={statusMap}
        selectedId={device?.id}
        setSelectedId={setSelectedDeviceId}
        onAddPrinter={onAddPrinter}
      />
      <DeviceMonitor
        device={device}
        status={status}
        onPause={onPause}
        onResume={onResume}
        onStop={onStop}
        onJumpToPlate={onJumpToPlate}
        onEditPrinter={onEditPrinter}
      />
    </div>
  );
}

window.DevicesView = DevicesView;
