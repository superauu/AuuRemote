import { KeyRound, Languages, LockKeyhole, LockOpen, Monitor, Moon, MousePointerClick, Sun } from "lucide-react";
import { setLanguage, useI18n } from "../i18n";
import { setTheme, type ThemeMode, useTheme } from "../theme";
import type { VaultStatus } from "../types";

const THEME_ORDER: ThemeMode[] = ["dark", "light", "system"];

function ThemeIcon({ mode }: { mode: ThemeMode }) {
  if (mode === "light") return <Sun size={14} aria-hidden="true" />;
  if (mode === "system") return <Monitor size={14} aria-hidden="true" />;
  return <Moon size={14} aria-hidden="true" />;
}

export function Toolbar({ vault, onNewConnection, onOpenVault }: {
  vault?: VaultStatus;
  onNewConnection: () => void;
  onOpenVault: () => void;
}) {
  const { lang, t } = useI18n();
  const { theme } = useTheme();
  const nextTheme = THEME_ORDER[(THEME_ORDER.indexOf(theme) + 1) % THEME_ORDER.length];
  const vaultMode = !vault?.setup ? "create" : vault.unlocked ? "change" : "unlock";
  const VaultIcon = !vault?.setup ? KeyRound : vault.unlocked ? LockOpen : LockKeyhole;
  const vaultLabel = !vault?.setup ? t("toolbar.vault.setup")
    : vault.unlocked ? t("toolbar.vault.unlocked") : t("toolbar.vault.locked");

  return (
    <header className="flex h-10 shrink-0 items-center gap-3 border-b border-border/70 px-3">
      <span className="brand-mark">AuuRemote</span>
      <button
        type="button"
        className="inline-flex h-7 items-center gap-1.5 rounded-md border border-primary/45 bg-primary/15 px-2.5 text-xs font-medium text-primary hover:bg-primary/25 active:scale-[0.98]"
        onClick={onNewConnection}
      >
        <MousePointerClick size={13} aria-hidden="true" />
        {t("toolbar.newConnection")}
      </button>
      <div className="ms-auto flex items-center gap-1">
        <button
          type="button"
          title={vaultLabel}
          aria-label={vaultLabel}
          className="inline-flex h-7 w-7 items-center justify-center rounded-md text-muted-foreground hover:bg-accent hover:text-accent-foreground active:scale-[0.98]"
          onClick={onOpenVault}
        >
          <VaultIcon size={14} aria-hidden="true" />
        </button>
        <button
          type="button"
          title={t("toolbar.language")}
          aria-label={t("toolbar.language")}
          className="inline-flex h-7 items-center gap-1 rounded-md px-1.5 text-xs font-semibold text-muted-foreground hover:bg-accent hover:text-accent-foreground active:scale-[0.98]"
          onClick={() => setLanguage(lang === "zh" ? "en" : "zh")}
        >
          <Languages size={14} aria-hidden="true" />
          {lang === "zh" ? "EN" : "中"}
        </button>
        <button
          type="button"
          title={t(`toolbar.theme.${nextTheme}`)}
          aria-label={t(`toolbar.theme.${nextTheme}`)}
          className="inline-flex h-7 w-7 items-center justify-center rounded-md text-muted-foreground hover:bg-accent hover:text-accent-foreground active:scale-[0.98]"
          onClick={() => setTheme(nextTheme)}
        >
          <ThemeIcon mode={theme} />
        </button>
      </div>
    </header>
  );
}
