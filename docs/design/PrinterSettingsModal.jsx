// PrinterSettingsModal.jsx — per-printer settings sheet.
// Reached by clicking the cog next to a printer in the picker.
// Sections: General (name + profile info), Start G-code, End G-code,
// Machine limits. Delete lives in the footer with an inline confirm.

function PrinterSettingsModal({ printer, allPrinters, onSave, onDelete, onClose }) {
  const { useState, useEffect, useRef } = React;
  if (!printer) return null;

  const [draft, setDraft] = useState({
    name: printer.name,
    nozzle: printer.nozzle,
    bedPlate: printer.bedPlate,
    amsUnits: printer.amsUnits || 0,
    startGcode: printer.startGcode || "",
    endGcode: printer.endGcode || "",
    limits: { ...(printer.limits || {}) },
  });
  const [active, setActive] = useState("general");
  const [confirmingDelete, setConfirmingDelete] = useState(false);

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
    });
    setActive("general");
    setConfirmingDelete(false);
  }, [printer.id]);

  // Esc to close
  useEffect(() => {
    const onKey = (e) => { if (e.key === "Escape") { e.stopPropagation(); onClose(); } };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const otherNames = allPrinters.filter(p => p.id !== printer.id).map(p => p.name);
  const trimmedName = draft.name.trim();
  const nameInUse = trimmedName && otherNames.includes(trimmedName);
  const dirty = (
    draft.name !== printer.name ||
    draft.nozzle !== printer.nozzle ||
    draft.amsUnits !== (printer.amsUnits || 0) ||
    draft.startGcode !== (printer.startGcode || "") ||
    draft.endGcode !== (printer.endGcode || "") ||
    JSON.stringify(draft.limits) !== JSON.stringify(printer.limits || {})
  );
  const canSave = !!trimmedName && !nameInUse && dirty;

  const handleSave = () => {
    if (!canSave) return;
    onSave(printer.id, {
      name: trimmedName,
      nozzle: draft.nozzle,
      amsUnits: draft.amsUnits,
      startGcode: draft.startGcode,
      endGcode: draft.endGcode,
      limits: draft.limits,
    });
  };

  const setLimit = (k, v) => setDraft(d => ({ ...d, limits: { ...d.limits, [k]: v } }));

  const otherCount = allPrinters.length - 1;
  const fallback = allPrinters.find(p => p.id !== printer.id);

  const sections = [
    { id: "general",  label: "General",       icon: "⚙" },
    { id: "start",    label: "Start G-code",  icon: "⊳" },
    { id: "end",      label: "End G-code",    icon: "⊲" },
    { id: "limits",   label: "Machine limits",icon: "↔" },
  ];

  return (
    <div className="modal-backdrop" onClick={onClose}>
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
          <button className="apm-close" onClick={onClose} aria-label="Close" title="Close (Esc)">
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
                className={`psm-nav-item ${active === s.id ? "active" : ""}`}
                onClick={() => setActive(s.id)}
                type="button"
              >
                <span className="psm-nav-icon">{s.icon}</span>
                <span>{s.label}</span>
              </button>
            ))}
          </nav>

          <section className="psm-content">
            {active === "general" && (
              <GeneralSection
                draft={draft} setDraft={setDraft}
                printer={printer} nameInUse={nameInUse}
              />
            )}
            {active === "start" && (
              <GcodeSection
                kind="start"
                value={draft.startGcode}
                onChange={(v) => setDraft(d => ({ ...d, startGcode: v }))}
              />
            )}
            {active === "end" && (
              <GcodeSection
                kind="end"
                value={draft.endGcode}
                onChange={(v) => setDraft(d => ({ ...d, endGcode: v }))}
              />
            )}
            {active === "limits" && (
              <LimitsSection limits={draft.limits} setLimit={setLimit}/>
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
                <button className="apm-btn" onClick={onClose} type="button">
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
      </div>
    </div>
  );
}

function GeneralSection({ draft, setDraft, printer, nameInUse }) {
  const AmsPicker = window.AmsPicker;
  return (
    <div className="psm-section">
      <div className="psm-field">
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

      <div className="psm-field">
        <label htmlFor="psm-nozzle">Default nozzle</label>
        <div className="apm-name-input">
          <input
            id="psm-nozzle"
            value={draft.nozzle}
            onChange={(e) => setDraft(d => ({ ...d, nozzle: e.target.value }))}
            placeholder="e.g. 0.4 mm hardened"
          />
        </div>
        <div className="apm-name-hint">Used for new plates assigned to this printer. Each plate can still override.</div>
      </div>

      {(printer.amsMax || 0) > 0 && (
        <div className="psm-field">
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

function GcodeSection({ kind, value, onChange }) {
  const help = kind === "start"
    ? "Runs before the first layer — homing, heating, purge line. Use [bed_temp], [nozzle_temp], [first_layer_temp] etc as placeholders."
    : "Runs after the last layer — cool down, park, retract. Avoid moves that crash into the print.";
  return (
    <div className="psm-section">
      <div className="psm-field">
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

function LimitsSection({ limits, setLimit }) {
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
              <div className="psm-limit-cell" key={item.k}>
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
