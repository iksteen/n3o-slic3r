import React from "react";
import ReactDOM from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import App from "./App";
import "./index.css";
import { applyTheme, readStoredMode, resolveTheme } from "./theme/useTheme";

// Apply the persisted theme before React mounts so the first paint is
// already correct — avoids a light-mode flash when the stored
// preference is Dark.
applyTheme(resolveTheme(readStoredMode()));

// Resolve the Strategy-A wgpu-viewport flag (Linux, N3O_WGPU=1) before mounting
// so App can mount the wgpu canvas instead of the Three.js viewport.
async function boot() {
  let wgpu = false;
  try {
    wgpu = await invoke<boolean>("wgpu_viewport_enabled");
  } catch {
    /* non-Tauri / command missing → stay on Three.js */
  }
  (window as typeof window & { __N3O_WGPU?: boolean }).__N3O_WGPU = wgpu;

  ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
    <React.StrictMode>
      <App />
    </React.StrictMode>,
  );
}

void boot();
