import { useSyncExternalStore } from "react";

export type ThemeMode = "dark" | "light" | "system";

interface ThemeState {
  theme: ThemeMode;
  resolved: "dark" | "light";
}

const STORAGE_KEY = "auu_remote_theme";
const listeners = new Set<() => void>();
let state: ThemeState = { theme: readSaved(), resolved: "dark" };
state.resolved = resolve(state.theme);

function readSaved(): ThemeMode {
  try {
    const saved = localStorage.getItem(STORAGE_KEY);
    if (saved === "dark" || saved === "light" || saved === "system") return saved;
  } catch {
    // localStorage unavailable; fall through to the default.
  }
  // Default to the dark workspace, matching auugit.
  return "dark";
}

function resolve(mode: ThemeMode): "dark" | "light" {
  if (mode === "system" && window.matchMedia) {
    return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
  }
  return mode === "light" ? "light" : "dark";
}

function apply(theme: "dark" | "light", animate: boolean) {
  const root = document.documentElement;
  const toggle = () => root.classList.toggle("dark", theme === "dark");
  // Manual switches cross-fade via View Transitions; system follow-ups and
  // reduced-motion users get the class change without animation.
  const reduceMotion = window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;
  if (animate && !reduceMotion && "startViewTransition" in document) {
    document.startViewTransition(toggle);
  } else {
    toggle();
  }
}

function emit() {
  for (const listener of listeners) listener();
}

export function setTheme(theme: ThemeMode) {
  try {
    localStorage.setItem(STORAGE_KEY, theme);
  } catch {
    // Keep the session-only value even when persistence fails.
  }
  const resolved = resolve(theme);
  apply(resolved, true);
  state = { theme, resolved };
  emit();
}

/** Registers the OS color-scheme listener; returns its cleanup. */
export function initTheme(): () => void {
  apply(state.resolved, false);
  if (!window.matchMedia) return () => {};
  const media = window.matchMedia("(prefers-color-scheme: dark)");
  const listener = () => {
    if (state.theme !== "system") return;
    const resolved = resolve("system");
    apply(resolved, false);
    state = { ...state, resolved };
    emit();
  };
  media.addEventListener("change", listener);
  return () => media.removeEventListener("change", listener);
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function useTheme(): ThemeState {
  return useSyncExternalStore(subscribe, () => state);
}
