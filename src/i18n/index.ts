import { useCallback, useSyncExternalStore } from "react";
import { en } from "./en";
import { zh } from "./zh";

export type Lang = "zh" | "en";
export type TranslationKey = keyof typeof zh;

const dictionaries: Record<Lang, Record<string, string>> = {
  zh: zh as unknown as Record<string, string>,
  en,
};

const STORAGE_KEY = "auu_remote_language";
const listeners = new Set<() => void>();
let current: Lang = load();

function load(): Lang {
  try {
    const saved = localStorage.getItem(STORAGE_KEY);
    if (saved === "zh" || saved === "en") return saved;
  } catch {
    // Fall through to the system language.
  }
  return navigator.language?.toLowerCase().startsWith("zh") ? "zh" : "en";
}

function emit() {
  for (const listener of listeners) listener();
}

export function setLanguage(lang: Lang) {
  current = lang;
  try {
    localStorage.setItem(STORAGE_KEY, lang);
  } catch {
    // Keep the session-only value even when persistence fails.
  }
  document.documentElement.lang = lang === "zh" ? "zh-CN" : "en";
  emit();
}

/** Applies the resolved language to <html> once at startup. */
export function initLanguage() {
  document.documentElement.lang = current === "zh" ? "zh-CN" : "en";
}

/** Flat lookup with `{name}` interpolation; missing keys fall back to zh. */
export function translate(lang: Lang, key: TranslationKey | string, vars?: Record<string, string | number>): string {
  const text = dictionaries[lang]?.[key] ?? dictionaries.zh[key] ?? key;
  return vars
    ? text.replace(/\{(\w+)\}/g, (match, name: string) =>
        name in vars ? String(vars[name]) : match,
      )
    : text;
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function useI18n() {
  const lang = useSyncExternalStore(
    subscribe,
    () => current,
    () => "zh" as Lang,
  );
  const t = useCallback(
    (key: TranslationKey | string, vars?: Record<string, string | number>) =>
      translate(lang, key, vars),
    [lang],
  );
  return { lang, t };
}
