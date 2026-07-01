// Undo/redo — thin client over the backend history (`UndoHistory`).
//
// State is backend-authoritative: read `project_history_state` once on
// mount, then track `project:history_changed`. The hook also installs the
// global keyboard shortcuts (Ctrl/Cmd+Z, Ctrl/Cmd+Shift+Z, Ctrl+Y) so any
// screen gets undo/redo without per-view wiring.

import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { onEvents } from "../state/eventRouter";
import { shouldIgnoreHotkey } from "../ui/hotkeyInhibit";

interface HistoryState {
  can_undo: boolean;
  can_redo: boolean;
}

export interface UndoRedo {
  canUndo: boolean;
  canRedo: boolean;
  undo: () => void;
  redo: () => void;
  /** Platform-correct shortcut hints for tooltips ("⌘Z" / "Ctrl+Z"). */
  undoHint: string;
  redoHint: string;
}

const IS_MAC =
  typeof navigator !== "undefined" &&
  /mac/i.test(navigator.platform || navigator.userAgent);

const UNDO_HINT = IS_MAC ? "⌘Z" : "Ctrl+Z";
const REDO_HINT = IS_MAC ? "⇧⌘Z" : "Ctrl+Shift+Z";

/** `disabled` gates undo/redo off entirely (keyboard + reported can-undo/redo,
 *  which disables the toolbar buttons) — used while a modal editing session like
 *  the split tool is open, so a stray Ctrl+Z can't revert the scene underneath
 *  it. The tool's own shortcuts (Esc, Delete-connector) stay live. */
export function useUndoRedo(disabled = false): UndoRedo {
  const [state, setState] = useState<HistoryState>({
    can_undo: false,
    can_redo: false,
  });

  useEffect(() => {
    let active = true;
    invoke<HistoryState>("project_history_state")
      .then((s) => {
        if (active) setState(s);
      })
      .catch(() => {});
    const off = onEvents<HistoryState>(["project:history_changed"], (e) =>
      setState(e.payload),
    );
    return () => {
      active = false;
      off();
    };
  }, []);

  // Fire-and-forget: the backend emits project:restored + history_changed,
  // which drive the resync and this hook's state. A no-op step is harmless.
  const undo = useCallback(() => {
    void invoke("project_undo").catch((e) =>
      console.error("[undo] failed", e),
    );
  }, []);
  const redo = useCallback(() => {
    void invoke("project_redo").catch((e) =>
      console.error("[redo] failed", e),
    );
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent): void => {
      // Strict per-platform primary modifier: Cmd on macOS, Ctrl elsewhere
      // (so a Mac's Ctrl+Z stays available for terminal/native semantics).
      const primary = IS_MAC ? e.metaKey : e.ctrlKey;
      if (!primary || e.altKey) return;
      if (disabled || shouldIgnoreHotkey(e)) return;
      const key = e.key.toLowerCase();
      // Cmd/Ctrl+Shift+Z, or Ctrl+Y (Windows) → redo; Cmd/Ctrl+Z → undo.
      if ((key === "z" && e.shiftKey) || (!IS_MAC && key === "y")) {
        e.preventDefault();
        redo();
      } else if (key === "z" && !e.shiftKey) {
        e.preventDefault();
        undo();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [undo, redo, disabled]);

  return {
    canUndo: state.can_undo && !disabled,
    canRedo: state.can_redo && !disabled,
    undo,
    redo,
    undoHint: UNDO_HINT,
    redoHint: REDO_HINT,
  };
}
