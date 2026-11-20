import { KeyRound, Languages, LockKeyhole, LockOpen, Monitor, Moon, PanelRight, Plus, Sun } from "lucide-react";
import type { Ref } from "react";
import { setLanguage, useI18n } from "../i18n";
import { setTheme, type ThemeMode, useTheme } from "../theme";
import type { VaultStatus } from "../types";

const THEME_ORDER: ThemeMode[] = ["dark", "light", "system"];

function ThemeIcon({ mode }: { mode: ThemeMode }) {
  if (mode === "light") return <Sun size={14} aria-hidden="true" />;
  if (mode === "system") return <Monitor size={14} aria-hidden="true" />;
  return <Moon size={14} aria-hidden="true" />;
}

export function Toolbar({ vault, onOpenVault, onNew, configOpen, configToggle, onToggleConfig }: {
  vault?: VaultStatus;
  onOpenVault: () => void;
  onNew: () => void;
  configOpen: boolean;
  configToggle: Ref<HTMLButtonElement>;
  onToggleConfig: () => void;
}) {
  const { lang, t } = useI18n();
  const { theme } = useTheme();
  const nextTheme = THEME_ORDER[(THEME_ORDER.indexOf(theme) + 1) % THEME_ORDER.length];
  const VaultIcon = !vault?.setup ? KeyRound : vault.unlocked ? LockOpen : LockKeyhole;
  const vaultLabel = !vault?.setup ? t("toolbar.vault.setup")
    : vault.unlocked ? t("toolbar.vault.unlocked") : t("toolbar.vault.locked");

  return (
    <header className="app-toolbar flex h-12 shrink-0 items-center gap-2 px-3">
      <span className="app-icon-tile" aria-hidden="true">
        <Monitor size={15} strokeWidth={2} />
      </span>
      <span className="brand-mark select-none">AuuRemote</span>
      <span className="toolbar-divider mx-2" aria-hidden="true" />
      <button type="button" className="btn-outline toolbar-new" onClick={onNew}>
        <Plus size={15} aria-hidden="true" />
        {t("toolbar.newConnection")}
      </button>
      <div className="ms-auto flex items-center gap-1">
        <button
          type="button"
          className="icon-btn"
          title={vaultLabel}
          aria-label={vaultLabel}
          onClick={onOpenVault}
        >
          <VaultIcon size={14} aria-hidden="true" />
        </button>
        <button
          type="button"
          className="icon-btn !w-auto gap-1 px-1.5 text-xs font-semibold"
          title={t("toolbar.language")}
          aria-label={t("toolbar.language")}
          onClick={() => setLanguage(lang === "zh" ? "en" : "zh")}
        >
          <Languages size={14} aria-hidden="true" />
          {lang === "zh" ? "EN" : "中"}
        </button>
        <button
          type="button"
          className="icon-btn"
          title={t(`toolbar.theme.${nextTheme}`)}
          aria-label={t(`toolbar.theme.${nextTheme}`)}
          onClick={() => setTheme(nextTheme)}
        >
          <ThemeIcon mode={theme} />
        </button>
        <span className="toolbar-divider mx-1" aria-hidden="true" />
        <button
          ref={configToggle}
          type="button"
          className="icon-btn"
          title={t(configOpen ? "config.hide" : "config.show")}
          aria-label={t(configOpen ? "config.hide" : "config.show")}
          aria-expanded={configOpen}
          aria-controls="connection-config"
          onClick={onToggleConfig}
        >
          <PanelRight size={16} aria-hidden="true" />
        </button>
      </div>
    </header>
  );
}
