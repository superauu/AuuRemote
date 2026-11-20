import { type TranslationKey } from "./i18n";

/**
 * Backend commands reject with `{ code, detail }` (src-tauri/src/errors.rs).
 * This maps each code to a localized message key; the raw detail stays
 * available as debug context.
 */
export type ApiErrorShape = { code: string; detail?: string };

const ERROR_KEYS: Record<string, TranslationKey> = {
  invalidEndpoint: "errors.invalidEndpoint",
  invalidUsername: "errors.invalidUsername",
  invalidProfileName: "errors.invalidProfileName",
  invalidFingerprint: "errors.invalidFingerprint",
  invalidMasterPassword: "errors.invalidMasterPassword",
  passwordRequired: "errors.passwordRequired",
  connectTimeout: "errors.connectTimeout",
  connectionFailed: "errors.connectionFailed",
  certificateMismatch: "errors.certificateMismatch",
  sessionClosed: "errors.sessionClosed",
  resolutionUnsupported: "errors.resolutionUnsupported",
  sendTooLarge: "errors.sendTooLarge",
  noFrame: "errors.noFrame",
  configCorrupt: "errors.configCorrupt",
  vaultNotSetup: "errors.vaultNotSetup",
  vaultWrongPassword: "errors.vaultWrongPassword",
  vaultAlreadySetup: "errors.vaultAlreadySetup",
  vaultLocked: "errors.vaultLocked",
  profileMissing: "errors.profileMissing",
  internal: "errors.internal",
  ioError: "errors.ioError",
};

export function asApiError(error: unknown): ApiErrorShape | undefined {
  if (error && typeof error === "object" && "code" in error) {
    return error as ApiErrorShape;
  }
  return undefined;
}

type Translator = (key: TranslationKey | string, vars?: Record<string, string | number>) => string;

/** Localized message for an invoke rejection; falls back to String(error). */
export function describeError(error: unknown, t: Translator): string {
  const api = asApiError(error);
  if (api) {
    const key = ERROR_KEYS[api.code] ?? "errors.internal";
    return t(key, { detail: api.detail ?? "" });
  }
  return String(error);
}

/** Pulls the SHA-256 fingerprint out of a certificateMismatch detail. */
export function extractFingerprint(detail: string): string | undefined {
  return detail.match(/[0-9A-F]{2}(?::[0-9A-F]{2}){31}/)?.[0];
}
