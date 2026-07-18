import { useEffect, useState } from "react";

// Owns the prepare-tab viewport's tool state: the transform gizmo
// (move/rotate/scale), the armed placing tools (lay-flat / align /
// face-match / clone), the clone dialog request, and the match-face
// two-click sub-state. The load-bearing invariant — only one viewport
// tool is active at a time — lives here: arming a tool clears the gizmo
// and vice versa. The renderer-local tool *mode* is App-owned UI state
// (it never enters core/scene); this hook is just where it lives.

type GizmoMode = "none" | "move" | "rotate" | "scale";

type ViewportToolMode =
  | "none"
  | "layflat"
  | "alignX"
  | "alignY"
  | "facematch"
  | "clone"
  | "split"
  | "paint";

/** A pending clone-dialog request: the object ids to clone + whether to
 *  expand groups. `null` = no dialog open. */
interface CloneRequest {
  ids: number[];
  expandGroups: boolean;
}

export interface ViewportTools {
  gizmoMode: GizmoMode;
  tool: ViewportToolMode;
  clone: CloneRequest | null;
  faceMatchStep: boolean;
  setFaceMatchStep: (v: boolean) => void;
  /** Toggle a gizmo mode; clears any armed tool (mutual exclusion). */
  selectGizmo: (m: GizmoMode) => void;
  /** Toggle an armed tool; clears the gizmo (mutual exclusion). */
  selectTool: (t: ViewportToolMode) => void;
  /** Disarm the active tool (e.g. a tool reported done). */
  clearTool: () => void;
  /** A pick-to-clone click landed: disarm and open the dialog on it. */
  pickClone: (id: number) => void;
  /** Clone button: open on the current selection, or arm pick-to-clone
   *  when nothing is selected. */
  armClone: (selection: number[]) => void;
  /** Close the clone dialog without cloning. */
  closeClone: () => void;
}

export function useViewportTools(): ViewportTools {
  // The wgpu viewport's gizmo mode.
  const [gizmoMode, setGizmoMode] = useState<GizmoMode>("none");
  // wgpu viewport's armed placing tool (lay-flat / align). Mutually
  // exclusive with the gizmo: arming a tool clears the gizmo and vice versa.
  const [tool, setTool] = useState<ViewportToolMode>("none");
  // wgpu clone dialog (open with a set of ids; null = closed).
  const [clone, setClone] = useState<CloneRequest | null>(null);
  // Match-face two-click sub-state: true once the reference face is picked.
  const [faceMatchStep, setFaceMatchStep] = useState(false);

  // Reset the match-face step whenever the armed tool changes (arm/disarm/switch).
  useEffect(() => {
    setFaceMatchStep(false);
  }, [tool]);

  const selectGizmo = (m: GizmoMode): void => {
    setTool("none");
    setGizmoMode((cur) => (cur === m ? "none" : m));
  };
  const selectTool = (t: ViewportToolMode): void => {
    setGizmoMode("none");
    setTool((cur) => (cur === t ? "none" : t));
  };
  const clearTool = (): void => setTool("none");
  const pickClone = (id: number): void => {
    setTool("none");
    setClone({ ids: [id], expandGroups: true });
  };
  const armClone = (selection: number[]): void => {
    // With a selection, open the dialog on it; otherwise arm
    // pick-to-clone (the next clicked object's group).
    if (selection.length > 0) {
      setClone({ ids: selection, expandGroups: false });
    } else {
      setGizmoMode("none");
      setTool((cur) => (cur === "clone" ? "none" : "clone"));
    }
  };
  const closeClone = (): void => setClone(null);

  return {
    gizmoMode,
    tool,
    clone,
    faceMatchStep,
    setFaceMatchStep,
    selectGizmo,
    selectTool,
    clearTool,
    pickClone,
    armClone,
    closeClone,
  };
}
