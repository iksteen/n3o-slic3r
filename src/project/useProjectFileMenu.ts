import { message as messageDialog } from "@tauri-apps/plugin-dialog";
import { projectNew, projectLoad, projectSave, projectSaveAs } from "./projectFile";
import { openFile, saveFile } from "../ui/fileDialog";

// The project file menu (New / Open / Save / Save as) + its dialog
// wiring. `sourcePath` is the project's on-disk origin (null until first
// save); the hook derives the display name from it and routes a
// path-less Save through Save As.

export interface ProjectFileMenu {
  /** Basename of the source path, or "Untitled.n3o" when unsaved. */
  projectName: string;
  handleNewProject: () => Promise<void>;
  handleOpenProject: () => Promise<void>;
  handleSaveProject: () => Promise<void>;
  handleSaveProjectAs: () => Promise<void>;
}

export function useProjectFileMenu(
  sourcePath: string | null,
  /** For a crash-recovered project (sourcePath null): the pre-crash
   *  path. Save-As defaults here so Save writes back over the original
   *  instead of the recovery file. */
  recoveryOrigin: string | null = null,
): ProjectFileMenu {
  // The menu label shows the source path's basename, or "Untitled.n3o"
  // for an unsaved project.
  const projectName = sourcePath
    ? sourcePath.split(/[\\/]/).pop() || "Untitled.n3o"
    : "Untitled.n3o";
  // Open accepts native .n3o projects AND foreign Bambu/Orca .3mf (imported);
  // Save only writes the native .n3o container.
  const openFilters = [
    { name: "Project", extensions: ["n3o", "3mf"] },
    { name: "n3o project", extensions: ["n3o"] },
    { name: "Bambu / Orca 3MF", extensions: ["3mf"] },
  ];
  const saveFilters = [{ name: "n3o project", extensions: ["n3o"] }];

  // Surface a project file-op failure to the user (a native dialog), not
  // just the console — opening e.g. an OrcaSlicer .3mf via "Open project"
  // otherwise looks like nothing happened.
  const reportProjectError = (action: string, err: unknown): void => {
    console.error(`[project] ${action} failed`, err);
    void messageDialog(String(err), { title: `${action} failed`, kind: "error" });
  };

  const handleNewProject = async (): Promise<void> => {
    try {
      await projectNew(); // → project:loaded → session + scene resync
    } catch (err) {
      reportProjectError("New project", err);
    }
  };

  const handleOpenProject = async (): Promise<void> => {
    try {
      const picked = await openFile({ filters: openFilters });
      if (picked == null) return; // cancelled
      await projectLoad(picked); // → project:loaded → session refetch
    } catch (err) {
      reportProjectError("Open project", err);
    }
  };

  const handleSaveProjectAs = async (): Promise<void> => {
    try {
      const picked = await saveFile({
        title: "Save project as",
        // Full path → the dialog opens in the right folder: the current
        // save target, else a recovered project's pre-crash origin, else
        // just the bare name for a never-saved project. (source_path and
        // recovery_origin are mutually exclusive, so order is cosmetic.)
        defaultPath: sourcePath ?? recoveryOrigin ?? projectName,
        filters: saveFilters,
      });
      if (picked == null) return; // cancelled
      await projectSaveAs(picked); // adopts the new source_path
    } catch (err) {
      reportProjectError("Save project as", err);
    }
  };

  const handleSaveProject = async (): Promise<void> => {
    // No source path yet (Untitled) → behave like Save As.
    if (!sourcePath) {
      await handleSaveProjectAs();
      return;
    }
    try {
      await projectSave(sourcePath);
    } catch (err) {
      reportProjectError("Save project", err);
    }
  };

  return {
    projectName,
    handleNewProject,
    handleOpenProject,
    handleSaveProject,
    handleSaveProjectAs,
  };
}
