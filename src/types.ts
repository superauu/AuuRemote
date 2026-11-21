import type { TranslationKey } from "./i18n";

export type Details = {
  name: string;
  host: string;
  port: number;
  username: string;
  fingerprint: string;
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

export const blankDraft: Draft = { name: "", host: "", port: "3389", username: "", fingerprint: "" };

export function targetAddress(host: string, port: string | number) {
  return `${host.includes(":") && !host.startsWith("[") ? `[${host}]` : host}:${port}`;
}

/** Strips the port back off a `targetAddress` string (IPv6 aware). */
export function targetHost(target: string) {
  if (target.startsWith("[")) return target.slice(1, target.indexOf("]"));
  const colon = target.lastIndexOf(":");
  return colon === -1 ? target : target.slice(0, colon);
}
