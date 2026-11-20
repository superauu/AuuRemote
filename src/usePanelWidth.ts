import { useState } from "react";

/** Persisted, clamped panel width (localStorage-backed). */
export function usePanelWidth(key: string, fallback: number, min: number, max: number) {
  const [width, setWidth] = useState<number>(() => {
    try {
      const saved = Number(localStorage.getItem(key));
      if (Number.isFinite(saved) && saved >= min && saved <= max) return saved;
    } catch {
      // Fall through to the default.
    }
    return fallback;
  });

  const update = (next: number) => {
    const clamped = Math.min(max, Math.max(min, Math.round(next)));
    setWidth(clamped);
    try {
      localStorage.setItem(key, String(clamped));
    } catch {
      // Keep the session-only value even when persistence fails.
    }
  };

  return [width, update] as const;
}
