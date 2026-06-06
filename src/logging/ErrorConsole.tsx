// Floating error console — lifted from docs/dev/design/app.jsx (ConsolePane +
// the bottom-right toggle). Rendered inside the canvas frame so its overlays
// anchor to the 3D view. Open/closed + auto-open-on-error live in the log
// store, so it survives being re-mounted in the prepare vs preview frame.

import { useEffect, useRef } from "react";
import {
  clearLogs,
  setConsoleOpen,
  useConsoleOpen,
  useLogs,
  type LogEntry,
} from "./logStore";

function fmtTime(ts: number): string {
  const d = new Date(ts);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
}

// Sits along the bottom of the viewport (33vh). Auto-scrolls to the newest
// entry whenever logs change, so the latest message stays in view.
function ConsolePane({
  logs,
  onClose,
}: {
  logs: LogEntry[];
  onClose: () => void;
}): React.JSX.Element {
  const scrollerRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const el = scrollerRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [logs]);

  return (
    <div className="console-pane" role="log" aria-label="Console">
      <div className="console-pane-head">
        <span className="console-pane-title">Console</span>
        <span className="console-pane-count">
          {logs.length} entr{logs.length === 1 ? "y" : "ies"}
        </span>
        <span style={{ flex: 1 }} />
        <button
          className="console-pane-btn"
          onClick={() => clearLogs()}
          title="Clear console"
        >
          Clear
        </button>
        <button
          className="console-pane-btn"
          onClick={onClose}
          title="Hide console"
          aria-label="Close console"
        >
          <svg viewBox="0 0 12 12" width="11" height="11" fill="none" aria-hidden="true">
            <path
              d="M2 2l8 8M10 2l-8 8"
              stroke="currentColor"
              strokeWidth="1.4"
              strokeLinecap="round"
            />
          </svg>
        </button>
      </div>
      <div className="console-pane-body" ref={scrollerRef}>
        {logs.length === 0 && <div className="console-empty">— no messages —</div>}
        {logs.map((l) => (
          <div key={l.id} className={`console-line console-line-${l.level}`}>
            <span className="console-line-time">{fmtTime(l.ts)}</span>
            <span className="console-line-level">{l.level.toUpperCase()}</span>
            <span className="console-line-msg">{l.msg}</span>
          </div>
        ))}
      </div>
    </div>
  );
}

export function ErrorConsole(): React.JSX.Element {
  const logs = useLogs();
  const open = useConsoleOpen();
  const errorCount = logs.reduce((n, l) => n + (l.level === "error" ? 1 : 0), 0);
  const warnCount = logs.reduce((n, l) => n + (l.level === "warn" ? 1 : 0), 0);

  if (!open) {
    return (
      <button
        className="console-toggle"
        onClick={() => setConsoleOpen(true)}
        title="Show console"
      >
        <svg viewBox="0 0 14 14" fill="none" aria-hidden="true">
          <path
            d="M2.5 3.5l3 3-3 3M6.5 10h5"
            stroke="currentColor"
            strokeWidth="1.4"
            strokeLinecap="round"
            strokeLinejoin="round"
          />
        </svg>
        <span>Console</span>
        {errorCount > 0 && (
          <span className="console-toggle-badge error">{errorCount}</span>
        )}
        {errorCount === 0 && warnCount > 0 && (
          <span className="console-toggle-badge warn">{warnCount}</span>
        )}
      </button>
    );
  }

  return <ConsolePane logs={logs} onClose={() => setConsoleOpen(false)} />;
}
