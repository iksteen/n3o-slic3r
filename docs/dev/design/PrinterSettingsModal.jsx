// PrinterSettingsModal.jsx — per-printer settings sheet.
// Reached by clicking the cog next to a printer in the picker.
// Sections: General (name + profile info), Connection (network — only for
// printers n3o can talk to), Start G-code, End G-code, Machine limits.
// Delete lives in the footer with an inline confirm.

// Network connection support, keyed by hardware profile id. Printers absent
// from this map can't be driven over the network in this build, so their
// Connection section is hidden entirely.
//   needsAccessCode — show the access-code field (Bambu LAN auth)
//   needsPort       — show the port field (defaults to 80)
//   lanNote         — show the LAN-only / developer-mode requirement callout
const CONNECTION_SPECS = {
  bambu_a1:      { needsAccessCode: true,  lanNote: true },
  bambu_a1_mini: { needsAccessCode: true,  lanNote: true },
  snapmaker_u1:  { needsAccessCode: false, needsPort: true, lanNote: false },
};

const DEFAULT_PORT = 80;

function PrinterSettingsModal({ printer, allPrinters, onSave, onDelete, onClose }) {
  const { useState, useEffect, useRef } = React;
  if (!printer) return null;

  // Which printer profiles n3o can talk to over the network, and what each
  // connection needs. Profiles not listed here have no Connection section.
  const connSpec = CONNECTION_SPECS[printer.profileId] || null;

  const [draft, setDraft] = useState({
    name: printer.name,
    nozzle: printer.nozzle,
    bedPlate: printer.bedPlate,
    amsUnits: printer.amsUnits || 0,
    startGcode: printer.startGcode || "",
    endGcode: printer.endGcode || "",
    limits: { ...(printer.limits || {}) },
    connection: { ip: "", accessCode: "", port: DEFAULT_PORT, ...(printer.connection || {}) },
  });
  const [active, setActive] = useState("general");
  const [confirmingDelete, setConfirmingDelete] = useState(false);
  const [confirmingDiscard, setConfirmingDiscard] = useState(false);

  // Reset when switching to a different printer
  useEffect(() => {
    setDraft({
      name: printer.name,
      nozzle: printer.nozzle,
      bedPlate: printer.bedPlate,
      amsUnits: printer.amsUnits || 0,
      startGcode: printer.startGcode || "",
      endGcode: printer.endGcode || "",
      limits: { ...(printer.limits || {}) },
      connection: { ip: "", accessCode: "", port: DEFAULT_PORT, ...(printer.connection || {}) },
    });
    setActive("general");
    setConfirmingDelete(false);
    setConfirmingDiscard(false);
  }, [printer.id]);

  // Esc closes — but if there are unsaved edits, ask first instead.
  useEffect(() => {
    const onKey = (e) => {
      if (e.key !== "Escape") return;
      e.stopPropagation();
      if (confirmingDiscard) { setConfirmingDiscard(false); return; }
      requestClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  const otherNames = allPrinters.filter(p => p.id !== printer.id).map(p => p.name);
  const trimmedName = draft.name.trim();
  const nameInUse = trimmedName && otherNames.includes(trimmedName);
  const dirty = (
    draft.name !== printer.name ||
    draft.nozzle !== printer.nozzle ||
    draft.amsUnits !== (printer.amsUnits || 0) ||
    draft.startGcode !== (printer.startGcode || "") ||
    draft.endGcode !== (printer.endGcode || "") ||
    JSON.stringify(draft.limits) !== JSON.stringify(printer.limits || {}) ||
    (!!connSpec && JSON.stringify(draft.connection) !== JSON.stringify({ ip: "", accessCode: "", port: DEFAULT_PORT, ...(printer.connection || {}) }))
  );
  const canSave = !!trimmedName && !nameInUse && dirty;

  const handleSave = () => {
    if (!canSave) return;
    const patch = {
      name: trimmedName,
      nozzle: draft.nozzle,
      amsUnits: draft.amsUnits,
      startGcode: draft.startGcode,
      endGcode: draft.endGcode,
      limits: draft.limits,
    };
    if (connSpec) {
      patch.connection = { ip: draft.connection.ip };
      if (connSpec.needsAccessCode) patch.connection.accessCode = draft.connection.accessCode;
      if (connSpec.needsPort) patch.connection.port = draft.connection.port;
    }
    onSave(printer.id, patch);
  };

  // Close requests funnel through here so unsaved edits prompt a confirm
  // instead of silently discarding work.
  const requestClose = () => {
    if (dirty) { setConfirmingDiscard(true); return; }
    onClose();
  };

  const setConn = (k, v) => setDraft(d => ({ ...d, connection: { ...d.connection, [k]: v } }));

  const setLimit = (k, v) => setDraft(d => ({ ...d, limits: { ...d.limits, [k]: v } }));

  const otherCount = allPrinters.length - 1;
  const fallback = allPrinters.find(p => p.id !== printer.id);

  // Per-field / per-section change tracking, so the nav can flag which
  // sections hold unsaved edits and each field can mark itself changed.
  const baseConn = { ip: "", accessCode: "", port: DEFAULT_PORT, ...(printer.connection || {}) };
  const limitKeys = ["feedrateX","feedrateY","feedrateZ","feedrateE","acceleration","jerk","minLayer","maxLayer"];
  const limitChanged = {};
  limitKeys.forEach(k => {
    limitChanged[k] = (printer.limits?.[k] ?? "") !== (draft.limits?.[k] ?? "");
  });
  const changed = {
    name: draft.name !== printer.name,
    amsUnits: draft.amsUnits !== (printer.amsUnits || 0),
    startGcode: draft.startGcode !== (printer.startGcode || ""),
    endGcode: draft.endGcode !== (printer.endGcode || ""),
    conn: {
      ip: (draft.connection.ip || "") !== (baseConn.ip || ""),
      accessCode: (draft.connection.accessCode || "") !== (baseConn.accessCode || ""),
      port: String(draft.connection.port ?? "") !== String(baseConn.port ?? ""),
    },
    limits: limitChanged,
  };
  const sectionDirty = {
    general: changed.name || changed.amsUnits,
    connection: !!connSpec && (changed.conn.ip
      || (connSpec.needsAccessCode && changed.conn.accessCode)
      || (connSpec.needsPort && changed.conn.port)),
    start: changed.startGcode,
    end: changed.endGcode,
    limits: limitKeys.some(k => limitChanged[k]),
  };

  const sections = [
    { id: "general",  label: "General",       icon: "⚙" },
    ...(connSpec ? [{ id: "connection", label: "Connection", icon: "⇄" }] : []),
    { id: "start",    label: "Start G-code",  icon: "⊳" },
    { id: "end",      label: "End G-code",    icon: "⊲" },
    { id: "limits",   label: "Machine limits",icon: "↔" },
  ];

  return (
    <div className="modal-backdrop" onClick={requestClose}>
      <div
        className="printer-settings-modal"
        onClick={(e) => e.stopPropagation()}
        role="dialog"
        aria-modal="true"
        aria-labelledby="psm-title"
      >
        <header className="psm-header">
          <div className="psm-header-mark" data-brand={printer.brand}>
            <span>{printer.brandShort}</span>
          </div>
          <div className="psm-header-text">
            <h2 id="psm-title">{printer.name}</h2>
            <p>
              Based on <span className="psm-profile-label">{printer.profileLabel}</span>
              &nbsp;·&nbsp;
              <span className="psm-mono">{printer.plateSize[0]} × {printer.plateSize[1]} × {printer.plateSize[2]} mm</span>
            </p>
          </div>
          <button className="apm-close" onClick={requestClose} aria-label="Close" title="Close (Esc)">
            <svg width="14" height="14" viewBox="0 0 14 14" fill="none">
              <path d="M3 3l8 8M11 3l-8 8" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round"/>
            </svg>
          </button>
        </header>

        <div className="psm-body">
          <nav className="psm-nav" aria-label="Settings sections">
            {sections.map(s => (
              <button
                key={s.id}
                className={`psm-nav-item ${active === s.id ? "active" : ""} ${sectionDirty[s.id] ? "dirty" : ""}`}
                onClick={() => setActive(s.id)}
                type="button"
              >
                <span className="psm-nav-icon">{s.icon}</span>
                <span>{s.label}</span>
                {sectionDirty[s.id] && <span className="psm-nav-dot" title="Unsaved changes" aria-label="Unsaved changes"></span>}
              </button>
            ))}
          </nav>

          <section className="psm-content">
            {active === "general" && (
              <GeneralSection
                draft={draft} setDraft={setDraft}
                printer={printer} nameInUse={nameInUse}
                changed={changed}
              />
            )}
            {active === "connection" && connSpec && (
              <ConnectionSection
                printer={printer}
                spec={connSpec}
                connection={draft.connection}
                setConn={setConn}
                changed={changed.conn}
              />
            )}
            {active === "start" && (
              <GcodeSection
                kind="start"
                value={draft.startGcode}
                onChange={(v) => setDraft(d => ({ ...d, startGcode: v }))}
                changed={changed.startGcode}
              />
            )}
            {active === "end" && (
              <GcodeSection
                kind="end"
                value={draft.endGcode}
                onChange={(v) => setDraft(d => ({ ...d, endGcode: v }))}
                changed={changed.endGcode}
              />
            )}
            {active === "limits" && (
              <LimitsSection limits={draft.limits} setLimit={setLimit} changed={changed.limits}/>
            )}
          </section>
        </div>

        <footer className="psm-footer">
          {confirmingDelete ? (
            <div className="psm-confirm">
              <div className="psm-confirm-text">
                {allPrinters.length <= 1 ? (
                  <>
                    <strong>Can't delete the only printer.</strong>
                    <span> Add another first.</span>
                  </>
                ) : (
                  <>
                    <strong>Delete "{printer.name}"?</strong>
                    {otherCount > 0 && (
                      <span> Plates using it will be reassigned to <span className="psm-mono">{fallback.name}</span>.</span>
                    )}
                  </>
                )}
              </div>
              <div className="psm-confirm-actions">
                <button className="apm-btn" onClick={() => setConfirmingDelete(false)} type="button">
                  Cancel
                </button>
                {allPrinters.length > 1 && (
                  <button
                    className="apm-btn danger"
                    onClick={() => { onDelete(printer.id); }}
                    type="button"
                  >
                    Delete printer
                  </button>
                )}
              </div>
            </div>
          ) : (
            <>
              <button
                className="psm-delete-trigger"
                onClick={() => setConfirmingDelete(true)}
                type="button"
                title={allPrinters.length <= 1 ? "Can't delete the only printer" : "Delete this printer"}
              >
                <svg width="12" height="12" viewBox="0 0 14 14" fill="none">
                  <path d="M3 4h8M5 4V2.5a1 1 0 0 1 1-1h2a1 1 0 0 1 1 1V4M4.5 4l.5 7a1 1 0 0 0 1 1h3a1 1 0 0 0 1-1l.5-7" stroke="currentColor" strokeWidth="1.2" strokeLinecap="round" strokeLinejoin="round"/>
                </svg>
                Delete printer
              </button>
              <div className="apm-actions">
                <button className="apm-btn" onClick={requestClose} type="button">
                  {dirty ? "Cancel" : "Close"}
                </button>
                <button
                  className="apm-btn primary"
                  onClick={handleSave}
                  disabled={!canSave}
                  type="button"
                >
                  Save changes
                </button>
              </div>
            </>
          )}
        </footer>

        {confirmingDiscard && (
          <div className="psm-discard-overlay" onClick={() => setConfirmingDiscard(false)}>
            <div
              className="psm-discard-card"
              onClick={(e) => e.stopPropagation()}
              role="alertdialog"
              aria-modal="true"
              aria-labelledby="psm-discard-title"
            >
              <h3 id="psm-discard-title" className="psm-discard-title">Unsaved changes</h3>
              <p className="psm-discard-body">
                You have unsaved changes to <strong>{printer.name}</strong>. Closing now will discard them.
              </p>
              <div className="psm-discard-actions">
                <button className="apm-btn" onClick={() => setConfirmingDiscard(false)} type="button">
                  Keep editing
                </button>
                <button
                  className="apm-btn danger"
                  onClick={() => { setConfirmingDiscard(false); onClose(); }}
                  type="button"
                >
                  Discard changes
                </button>
                <button
                  className="apm-btn primary"
                  onClick={() => { if (canSave) { handleSave(); } setConfirmingDiscard(false); onClose(); }}
                  disabled={!canSave}
                  type="button"
                >
                  Save &amp; close
                </button>
              </div>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

function GeneralSection({ draft, setDraft, printer, nameInUse, changed = {} }) {
  const AmsPicker = window.AmsPicker;
  return (
    <div className="psm-section">
      <div className={`psm-field ${changed.name ? "changed" : ""}`}>
        <label htmlFor="psm-name">Display name</label>
        <div className={`apm-name-input ${nameInUse ? "error" : ""}`}>
          <input
            id="psm-name"
            value={draft.name}
            onChange={(e) => setDraft(d => ({ ...d, name: e.target.value }))}
          />
        </div>
        {nameInUse ? (
          <div className="apm-name-hint error">Another printer already uses this name.</div>
        ) : (
          <div className="apm-name-hint">How this printer shows up in the picker and on plate tabs.</div>
        )}
      </div>

      {(printer.amsMax || 0) > 0 && (
        <div className={`psm-field ${changed.amsUnits ? "changed" : ""}`}>
          <label>{printer.amsType || "AMS"} configuration</label>
          <AmsPicker
            amsMax={printer.amsMax}
            amsType={printer.amsType || "AMS"}
            value={draft.amsUnits || 0}
            onChange={(n) => setDraft(d => ({ ...d, amsUnits: n }))}
          />
        </div>
      )}

      <div className="psm-readonly">
        <div className="psm-readonly-row">
          <span>Profile</span>
          <span className="psm-mono">{printer.profileLabel}</span>
        </div>
        <div className="psm-readonly-row">
          <span>Build volume</span>
          <span className="psm-mono">{printer.plateSize[0]} × {printer.plateSize[1]} × {printer.plateSize[2]} mm</span>
        </div>
        {(printer.extruders || 1) > 1 && (
          <div className="psm-readonly-row">
            <span>Extruders</span>
            <span className="psm-mono">{printer.extruders} toolheads</span>
          </div>
        )}
      </div>
    </div>
  );
}

function ConnectionSection({ printer, spec, connection, setConn, changed = {} }) {
  return (
    <div className="psm-section">
      <div className={`psm-field ${changed.ip ? "changed" : ""}`}>
        <label htmlFor="psm-conn-ip">IP address</label>
        <div className="apm-name-input">
          <input
            id="psm-conn-ip"
            value={connection.ip}
            onChange={(e) => setConn("ip", e.target.value)}
            placeholder="e.g. 192.168.1.42"
            inputMode="decimal"
            autoComplete="off"
            spellCheck={false}
          />
        </div>
        <div className="apm-name-hint">
          The local network address of your {printer.profileLabel}. Find it on the printer's screen under network settings.
        </div>
      </div>

      {spec.needsPort && (
        <div className={`psm-field ${changed.port ? "changed" : ""}`}>
          <label htmlFor="psm-conn-port">Port</label>
          <div className="apm-name-input">
            <input
              id="psm-conn-port"
              value={connection.port}
              onChange={(e) => setConn("port", e.target.value.replace(/[^0-9]/g, ""))}
              placeholder="80"
              inputMode="numeric"
              autoComplete="off"
              spellCheck={false}
            />
          </div>
          <div className="apm-name-hint">
            The port n3o connects on. Defaults to 80; change it only if you've remapped the printer's HTTP interface.
          </div>
        </div>
      )}

      {spec.needsAccessCode && (
        <div className={`psm-field ${changed.accessCode ? "changed" : ""}`}>
          <label htmlFor="psm-conn-code">Access code</label>
          <div className="apm-name-input">
            <input
              id="psm-conn-code"
              value={connection.accessCode}
              onChange={(e) => setConn("accessCode", e.target.value)}
              placeholder="8-character code"
              autoComplete="off"
              spellCheck={false}
            />
          </div>
          <div className="apm-name-hint">
            Shown on the printer under the LAN-only access settings. Used to authenticate this connection.
          </div>
        </div>
      )}

      {spec.lanNote && (
        <div className="psm-conn-note" role="note">
          <svg className="psm-conn-note-ico" width="15" height="15" viewBox="0 0 16 16" fill="none" aria-hidden="true">
            <path d="M8 1.5l5.5 2.4v3.2c0 3.2-2.3 6-5.5 7-3.2-1-5.5-3.8-5.5-7V3.9L8 1.5z" stroke="currentColor" strokeWidth="1.2" strokeLinejoin="round"/>
            <path d="M8 6v3.2M8 11.2h.01" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round"/>
          </svg>
          <div className="psm-conn-note-body">
            <strong>Requires LAN-only mode with Developer Mode enabled.</strong>
            <span> On the printer, turn on <em>LAN Only Mode</em> and <em>Developer Mode</em> before connecting — n3o can't reach it over the cloud.</span>
          </div>
        </div>
      )}
    </div>
  );
}

function GcodeSection({ kind, value, onChange, changed = false }) {
  const help = kind === "start"
    ? "Runs before the first layer — homing, heating, purge line. Use [bed_temp], [nozzle_temp], [first_layer_temp] etc as placeholders."
    : "Runs after the last layer — cool down, park, retract. Avoid moves that crash into the print.";
  return (
    <div className="psm-section">
      <div className={`psm-field ${changed ? "changed" : ""}`}>
        <label htmlFor={`psm-gcode-${kind}`}>{kind === "start" ? "Start G-code" : "End G-code"}</label>
        <textarea
          id={`psm-gcode-${kind}`}
          className="psm-gcode"
          value={value}
          onChange={(e) => onChange(e.target.value)}
          spellCheck={false}
          rows={14}
          placeholder={kind === "start"
            ? "G28 ; home all axes\nG29 ; auto bed level\n…"
            : "M104 S0 ; turn off nozzle\nM140 S0 ; turn off bed\n…"}
        />
        <div className="apm-name-hint">{help}</div>
      </div>
    </div>
  );
}

function LimitsSection({ limits, setLimit, changed = {} }) {
  const fields = [
    { group: "Max feedrate", items: [
      { k: "feedrateX",   label: "X axis", unit: "mm/s" },
      { k: "feedrateY",   label: "Y axis", unit: "mm/s" },
      { k: "feedrateZ",   label: "Z axis", unit: "mm/s" },
      { k: "feedrateE",   label: "Extruder", unit: "mm/s" },
    ]},
    { group: "Motion", items: [
      { k: "acceleration",label: "Max accel", unit: "mm/s²" },
      { k: "jerk",        label: "Max jerk",  unit: "mm/s"  },
    ]},
    { group: "Layer height", items: [
      { k: "minLayer",    label: "Min layer", unit: "mm" },
      { k: "maxLayer",    label: "Max layer", unit: "mm" },
    ]},
  ];

  return (
    <div className="psm-section">
      {fields.map(group => (
        <div className="psm-limit-group" key={group.group}>
          <div className="psm-limit-group-label">{group.group}</div>
          <div className="psm-limit-grid">
            {group.items.map(item => (
              <div className={`psm-limit-cell ${changed[item.k] ? "changed" : ""}`} key={item.k}>
                <label htmlFor={`psm-l-${item.k}`}>{item.label}</label>
                <div className="psm-limit-input">
                  <input
                    id={`psm-l-${item.k}`}
                    type="number"
                    value={limits[item.k] ?? ""}
                    step={item.k.startsWith("minLayer") || item.k.startsWith("maxLayer") ? "0.01" : "1"}
                    onChange={(e) => {
                      const v = e.target.value;
                      setLimit(item.k, v === "" ? null : Number(v));
                    }}
                  />
                  <span className="psm-limit-unit">{item.unit}</span>
                </div>
              </div>
            ))}
          </div>
        </div>
      ))}
      <div className="apm-name-hint" style={{ marginTop: 8 }}>
        These caps clip slicer-generated G-code so nothing exceeds your machine's safe envelope.
      </div>
    </div>
  );
}

window.PrinterSettingsModal = PrinterSettingsModal;
