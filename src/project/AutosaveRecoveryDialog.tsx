// Startup autosave-recovery dialog (PR-5-10 UI).
//
// Mounts at the App root, gates the rest of the UI until the
// user has decided what to do with each recoverable autosave.
// Per the ticket's three-action surface:
//   - **Recover**: load the autosave via `project_load`.
//   - **Discard**: delete the file via `project_autosave_drop`.
//   - **Keep**: dismiss the entry from this session without
//     touching the file. Next launch will list it again.
//
// The dialog auto-dismisses (resolving the gate) when the user
// has handled every entry. An entry is "handled" when it has
// either been recovered (project_load + remove from list),
// discarded (drop + remove), or kept (just remove from local
// state; file stays on disk).
//
// The dialog itself doesn't enable the autosave worker — that's
// the caller's responsibility (App.tsx). Enabling happens at
// app start regardless of recovery: the new session writes to
// its own per-uuid autosave file, so a still-undecided recovery
// candidate doesn't collide.

import { useEffect, useMemo, useState } from "react";
import {
  autosaveDrop,
  autosaveList,
  projectLoad,
  type AutosaveEntry,
} from "./autosaveCommands";

export interface AutosaveRecoveryDialogProps {
  /** Fired when the user has decided every entry and the dialog
   * should dismiss. App.tsx un-gates the main UI on this. */
  onResolved: () => void;
}

/** Format a unix-seconds timestamp as a short relative-time
 * string. Exported for tests. */
export function formatRelative(unixSecs: number, nowMs = Date.now()): string {
  const deltaSecs = Math.max(0, Math.floor(nowMs / 1000) - unixSecs);
  if (deltaSecs < 60) return `${deltaSecs}s ago`;
  if (deltaSecs < 3600) return `${Math.floor(deltaSecs / 60)}m ago`;
  if (deltaSecs < 86400) return `${Math.floor(deltaSecs / 3600)}h ago`;
  return `${Math.floor(deltaSecs / 86400)}d ago`;
}

/** Format a byte count for the per-entry size hint. */
export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

type EntryState = {
  entry: AutosaveEntry;
  /** Set while the per-row action is in flight so the buttons
   * disable. */
  pending: boolean;
  /** Per-row error from a failed Recover / Discard call. */
  error: string | null;
};

export function AutosaveRecoveryDialog({
  onResolved,
}: AutosaveRecoveryDialogProps) {
  const [entries, setEntries] = useState<EntryState[] | null>(null);
  const [listError, setListError] = useState<string | null>(null);

  useEffect(() => {
    let mounted = true;
    autosaveList()
      .then((list) => {
        if (!mounted) return;
        if (list.length === 0) {
          // No recoveries — un-gate the UI immediately, never
          // render anything.
          onResolved();
          return;
        }
        setEntries(
          list.map((entry) => ({ entry, pending: false, error: null })),
        );
      })
      .catch((err) => {
        if (!mounted) return;
        setListError(String(err));
        // Don't gate the UI on a list failure — the user can
        // still work even if recovery is broken. Render the
        // error briefly + un-gate.
        console.error("[autosave] list failed", err);
        onResolved();
      });
    return () => {
      mounted = false;
    };
  }, [onResolved]);

  // Remove an entry from local state; once empty, the dialog
  // auto-resolves.
  const removeEntry = (uuid: string): void => {
    setEntries((prev) => {
      if (!prev) return prev;
      const next = prev.filter((e) => e.entry.uuid !== uuid);
      if (next.length === 0) {
        // Defer the parent callback to the next tick so the
        // unmount + transition stack stays clean.
        queueMicrotask(onResolved);
      }
      return next;
    });
  };

  const setEntryPending = (uuid: string, pending: boolean): void => {
    setEntries((prev) => {
      if (!prev) return prev;
      return prev.map((e) =>
        e.entry.uuid === uuid ? { ...e, pending } : e,
      );
    });
  };

  const setEntryError = (uuid: string, error: string | null): void => {
    setEntries((prev) => {
      if (!prev) return prev;
      return prev.map((e) =>
        e.entry.uuid === uuid ? { ...e, error } : e,
      );
    });
  };

  const handleRecover = (entry: AutosaveEntry): void => {
    setEntryPending(entry.uuid, true);
    setEntryError(entry.uuid, null);
    void projectLoad(entry.path)
      .then(() => removeEntry(entry.uuid))
      .catch((err) => {
        setEntryError(entry.uuid, String(err));
        setEntryPending(entry.uuid, false);
      });
  };

  const handleDiscard = (entry: AutosaveEntry): void => {
    setEntryPending(entry.uuid, true);
    setEntryError(entry.uuid, null);
    void autosaveDrop(entry.uuid)
      .then(() => removeEntry(entry.uuid))
      .catch((err) => {
        setEntryError(entry.uuid, String(err));
        setEntryPending(entry.uuid, false);
      });
  };

  const handleKeep = (entry: AutosaveEntry): void => {
    removeEntry(entry.uuid);
  };

  // Pre-resolve / fetching: render nothing, the gate is open
  // (we'll resolve as soon as the list comes back empty).
  if (entries === null) {
    return null;
  }

  return (
    <div className="autosave-recovery-overlay" role="dialog" aria-modal="true">
      <div className="autosave-recovery-dialog">
        <div className="autosave-recovery-head">
          <h2 className="autosave-recovery-title">Recover unsaved projects</h2>
          <p className="autosave-recovery-sub">
            We found {entries.length} autosaved project
            {entries.length === 1 ? "" : "s"} from a previous session.
          </p>
        </div>
        {listError && (
          <div className="autosave-recovery-list-error" role="alert">
            Listing autosaves failed: {listError}
          </div>
        )}
        <ul className="autosave-recovery-list">
          {entries.map(({ entry, pending, error }) => (
            <li key={entry.uuid} className="autosave-recovery-row">
              <div className="autosave-recovery-meta">
                <span className="autosave-recovery-time">
                  {formatRelative(entry.modified_unix_secs)}
                </span>
                <span className="autosave-recovery-size">
                  {formatBytes(entry.size_bytes)}
                </span>
                <span className="autosave-recovery-uuid" title={entry.path}>
                  {entry.uuid.slice(0, 8)}…
                </span>
              </div>
              <div className="autosave-recovery-actions">
                <button
                  type="button"
                  className="autosave-recovery-btn primary"
                  onClick={() => handleRecover(entry)}
                  disabled={pending}
                >
                  Recover
                </button>
                <button
                  type="button"
                  className="autosave-recovery-btn danger"
                  onClick={() => handleDiscard(entry)}
                  disabled={pending}
                >
                  Discard
                </button>
                <button
                  type="button"
                  className="autosave-recovery-btn"
                  onClick={() => handleKeep(entry)}
                  disabled={pending}
                  title="Leave the file on disk and decide later"
                >
                  Keep
                </button>
              </div>
              {error && (
                <div className="autosave-recovery-row-error" role="alert">
                  {error}
                </div>
              )}
            </li>
          ))}
        </ul>
      </div>
    </div>
  );
}

/** Compose-time hook: returns `{ resolved, markResolved }`. The
 * App should render the dialog while `!resolved`, swap to the
 * main UI once resolved. Wrapped here as a hook so App.tsx
 * doesn't have to spell out the boolean state by hand. */
export function useAutosaveRecoveryGate(): {
  resolved: boolean;
  markResolved: () => void;
} {
  const [resolved, setResolved] = useState(false);
  return useMemo(
    () => ({ resolved, markResolved: () => setResolved(true) }),
    [resolved],
  );
}
