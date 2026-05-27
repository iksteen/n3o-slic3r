// SendToPrinterModal.jsx — picker that bridges Preview → Devices.
//
// Lists every configured printer with its current status and a quick fit
// check (does the plate's printer assignment match the device?). On send,
// the parent jumps to Devices with the chosen printer focused and queues
// the job on that printer's status.

function SendToPrinterModal({ plate, printers, statusMap, onSend, onClose }) {
  const { useState } = React;
  // Default to the plate's assigned printer if it exists; else the first printer.
  const defaultId = printers.find(p => p.id === plate.printerId)?.id || printers[0]?.id;
  const [selectedId, setSelectedId] = useState(defaultId);

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal stp-modal" onClick={e => e.stopPropagation()}>
        <header className="stp-header">
          <div>
            <div className="stp-eyebrow">Send to printer</div>
            <div className="stp-title">{plate.name}</div>
            <div className="stp-sub dim">
              Sliced for <b>{plate.printer}</b> · {plate.objects.length} object{plate.objects.length !== 1 ? "s" : ""}
            </div>
          </div>
          <button className="modal-close" onClick={onClose} title="Cancel">
            <svg width="12" height="12" viewBox="0 0 12 12" fill="none">
              <path d="M2.5 2.5l7 7M9.5 2.5l-7 7" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round"/>
            </svg>
          </button>
        </header>

        <div className="stp-list">
          {printers.map(p => {
            const st = statusMap[p.id] || { status: "idle" };
            const meta = statusMetaInline(st.status);
            const fits = p.id === plate.printerId;
            const canSend = st.status === "idle" || st.status === "printing";
            const selected = p.id === selectedId;
            return (
              <button
                key={p.id}
                className={`stp-row ${selected ? "selected" : ""} ${!canSend ? "disabled" : ""}`}
                onClick={() => canSend && setSelectedId(p.id)}
                disabled={!canSend}
              >
                <span className={`stp-radio ${selected ? "on" : ""}`}/>
                <div className="stp-row-main">
                  <div className="stp-row-name">
                    {p.name}
                    {fits && <span className="stp-tag fit">matches plate</span>}
                    {!fits && <span className="stp-tag mismatch" title="Plate was sliced for a different printer">different model</span>}
                  </div>
                  <div className="stp-row-meta">{p.profileLabel}</div>
                </div>
                <div className="stp-row-status">
                  <span className={`device-status-dot ${meta.cls}`}/>
                  <span className={`stp-row-state ${meta.cls}`}>{meta.label}</span>
                  {st.status === "printing" && (
                    <span className="stp-row-eta">queue → {(st.queue?.length || 0) + 1}</span>
                  )}
                </div>
              </button>
            );
          })}
        </div>

        <footer className="stp-footer">
          <span className="dim stp-hint">
            Job will {statusMap[selectedId]?.status === "printing" ? "queue behind the current job" : "start immediately"}.
          </span>
          <div className="stp-footer-actions">
            <button className="modal-btn" onClick={onClose}>Cancel</button>
            <button
              className="modal-btn primary"
              onClick={() => onSend(selectedId)}
              disabled={!selectedId}
            >
              Send
            </button>
          </div>
        </footer>
      </div>
    </div>
  );
}

function statusMetaInline(status) {
  switch (status) {
    case "printing": return { label: "Printing", cls: "printing" };
    case "paused":   return { label: "Paused",   cls: "paused" };
    case "error":    return { label: "Error",    cls: "error" };
    case "offline":  return { label: "Offline",  cls: "offline" };
    default:         return { label: "Idle",     cls: "idle" };
  }
}

window.SendToPrinterModal = SendToPrinterModal;
