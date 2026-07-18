// Theme state. Three user-facing modes (System / Light / Dark) collapse
// to two resolved themes (light / dark) which we surface as the
// `[data-theme="…"]` attribute on the document element. Tailwind's
// `dark:` variant is rebound to the same attribute via @custom-variant in
// index.css, so a single mechanism drives both the CSS-variable token
// swap and any `dark:`-utility-based rules.
//
// Persistence is `localStorage` only — it can be read synchronously in
// main.tsx before React mounts, eliminating the light-mode flash a
// Tauri-backed pref would cause.

import { useEffect, useSyncExternalStore } from "react";

export type ThemeMode = "system" | "light" | "dark";
export type ResolvedTheme = "light" | "dark";

const STORAGE_KEY = "n3o.theme";
const MODES: readonly ThemeMode[] = ["system", "light", "dark"] as const;

function isMode(v: unknown): v is ThemeMode {
  return typeof v === "string" && (MODES as readonly string[]).includes(v);
}

export function readStoredMode(): ThemeMode {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    return isMode(raw) ? raw : "system";
  } catch {
    return "system";
  }
}

function storeMode(mode: ThemeMode): void {
  try {
    localStorage.setItem(STORAGE_KEY, mode);
  } catch {
    // Storage may be disabled (private mode, etc.); falling back to
    // in-memory state is fine — the choice just won't survive a reload.
  }
}

function prefersDark(): boolean {
  return (
    typeof window !== "undefined" &&
    typeof window.matchMedia === "function" &&
    window.matchMedia("(prefers-color-scheme: dark)").matches
  );
}

export function resolveTheme(mode: ThemeMode): ResolvedTheme {
  if (mode === "system") return prefersDark() ? "dark" : "light";
  return mode;
}

export function applyTheme(resolved: ResolvedTheme): void {
  document.documentElement.dataset.theme = resolved;
}

// Module-level state shared across all useTheme() consumers in the
// renderer. A tiny pub-sub keeps the menu and any future indicator in
// sync without a Context.
let currentMode: ThemeMode = readStoredMode();
const listeners = new Set<() => void>();

function emit(): void {
  for (const fn of listeners) fn();
}

function subscribe(fn: () => void): () => void {
  listeners.add(fn);
  return () => {
    listeners.delete(fn);
  };
}

function setMode(mode: ThemeMode): void {
  currentMode = mode;
  storeMode(mode);
  applyTheme(resolveTheme(mode));
  emit();
}

function getMode(): ThemeMode {
  return currentMode;
}

export interface UseThemeResult {
  mode: ThemeMode;
  setMode: (mode: ThemeMode) => void;
  resolved: ResolvedTheme;
}

export function useTheme(): UseThemeResult {
  const mode = useSyncExternalStore(subscribe, getMode, getMode);

  // When mode is "system", track OS changes live so the app flips
  // alongside the OS theme without the user re-opening the menu.
  useEffect(() => {
    if (mode !== "system") return;
    if (typeof window === "undefined" || typeof window.matchMedia !== "function") return;
    const mql = window.matchMedia("(prefers-color-scheme: dark)");
    const onChange = (): void => {
      applyTheme(resolveTheme("system"));
    };
    mql.addEventListener("change", onChange);
    return () => mql.removeEventListener("change", onChange);
  }, [mode]);

  return { mode, setMode, resolved: resolveTheme(mode) };
}
