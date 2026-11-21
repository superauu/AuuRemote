import type { TranslationKey } from "./i18n";

export type Details = {
  name: string;
  host: string;
  port: number;
  username: string;
  fingerprint: string;
  /** "auto" (fit the window) or a "WIDTHxHEIGHT" string. */
  resolution: string;
};
export type Profile = Details & { id: number };
export type Draft = Omit<Details, "port"> & { port: string };

export type VaultStatus = {
  setup: boolean;
  unlocked: boolean;
  profileIds: number[];
};

/** One open RDP session; `id` matches the backend session id. */
export type Tab = {
  id: number;
  profileId?: number;
  label: string;
  target: string;
};

export type Notice = {
  kind: "info" | "error";
  titleKey: TranslationKey;
  /** Localized message from describeError; shown as the body. */
  message?: string;
  /** Raw backend detail, kept for fingerprint extraction and debugging. */
  raw?: string;
  code?: string;
};

export type BusyState = "loading" | "connecting" | "saving" | "removing";

export const blankDraft: Draft = { name: "", host: "", port: "3389", username: "", fingerprint: "", resolution: "auto" };

/** Resolution presets for the dropdown; "auto" fits the app window. */
export const RESOLUTION_OPTIONS = [
  "1920x1080", "1600x900", "1440x900", "1366x768", "1280x800", "1280x720", "1024x768",
] as const;

/** Turns a resolution value into concrete pixel dimensions (even, in-bounds). */
export function resolveSize(resolution: string): { width: number; height: number } {
  if (resolution !== "auto") {
    const [width, height] = resolution.split("x").map(Number);
    if (Number.isFinite(width) && Number.isFinite(height) && width >= 200 && height >= 200) {
      return { width, height };
    }
  }
  const dpr = window.devicePixelRatio || 1;
  return {
    width: Math.max(800, Math.min(4096, Math.round((window.innerWidth * dpr) / 2) * 2)),
    height: Math.max(600, Math.min(2160, Math.round((window.innerHeight * dpr) / 2) * 2)),
  };
}

export function targetAddress(host: string, port: string | number) {
  return `${host.includes(":") && !host.startsWith("[") ? `[${host}]` : host}:${port}`;
}

/** Strips the port back off a `targetAddress` string (IPv6 aware). */
export function targetHost(target: string) {
  if (target.startsWith("[")) return target.slice(1, target.indexOf("]"));
  const colon = target.lastIndexOf(":");
  return colon === -1 ? target : target.slice(0, colon);
}
