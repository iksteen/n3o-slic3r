// Shared non-blocking progress window — floats over the canvas's lower-left
// while a long operation is in flight. Both the slice-progress
// (src/slice/SliceProgressWindow.tsx) and send-upload-progress
// (src/driver/SendProgressWindow.tsx) UIs render through this, so the chrome
// (spinner + title + bar) lives in one place. Styling: `.progress-window*` in
// src/index.css. The window owns the operation's controls (e.g. Cancel) via
// the `action` slot — the topbar action button just disables while it runs.

export interface ProgressWindowProps {
  /** Head label, e.g. "Slicing", "Sending…". */
  title: string;
  /** 0..100; clamped here. */
  percent: number;
  /** Optional head chip after the title (slice: "N objects"). */
  count?: React.ReactNode;
  /** Optional lower-left content (slice: the active stage chip). */
  footer?: React.ReactNode;
  /** Optional lower-right control(s) for the operation — e.g. a Cancel
   *  button. Lives in the window, not the topbar. */
  action?: React.ReactNode;
}

export function ProgressWindow({
  title,
  percent,
  count,
  footer,
  action,
}: ProgressWindowProps): React.JSX.Element {
  const pct = Math.max(0, Math.min(100, percent));
  return (
    <div className="progress-window" role="status" aria-live="polite">
      <div className="progress-window-head">
        <span className="progress-window-spinner" aria-hidden />
        <span className="progress-window-title">{title}</span>
        {count != null && <span className="progress-window-count">{count}</span>}
        <span className="progress-window-pct">{pct}%</span>
      </div>
      <div className="progress-window-track">
        <div className="progress-window-fill" style={{ width: `${pct}%` }} />
      </div>
      {footer != null && (
        <div className="progress-window-stages">{footer}</div>
      )}
      {action != null && (
        <div className="progress-window-actions">{action}</div>
      )}
    </div>
  );
}
