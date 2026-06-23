import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./index.css";
import { applyTheme, readStoredMode, resolveTheme } from "./theme/useTheme";

// Apply the persisted theme before React mounts so the first paint is
// already correct — avoids a light-mode flash when the stored
// preference is Dark.
applyTheme(resolveTheme(readStoredMode()));

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
