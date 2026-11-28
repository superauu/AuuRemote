import { KeyRound, LockKeyhole, LockOpen, Monitor } from "lucide-react";
import { useI18n } from "../i18n";
import type { Tab, VaultStatus } from "../types";

export function StatusBar({ activeTab, sessionSize, sessionCount, vault }: {
  /** The focused session tab, mirroring auugit's "current repo" segment. */
  activeTab?: Tab;
  /** Live remote framebuffer size of the active session, when known. */
  sessionSize?: { x: number; y: number };
  sessionCount: number;
  vault?: VaultStatus;
}) {
  const { t } = useI18n();
  const vaultKey = !vault?.setup ? "status.vault.notSetup" : vault.unlocked ? "status.vault.unlocked" : "status.vault.locked";
  const VaultIcon = !vault?.setup ? KeyRound : vault.unlocked ? LockOpen : LockKeyhole;

  return (
    <footer
      aria-label={t("window.statusbar")}
      className="statusbar flex h-7 shrink-0 items-center justify-between gap-4 px-3 text-[12px] text-muted-foreground select-none"
    >
      <div className="flex min-w-0 items-center gap-3 overflow-hidden">
        {activeTab ? (
          <>
            <span className="statusbar-segment font-medium text-foreground" title={activeTab.label}>
              <Monitor size={12} aria-hidden="true" className="shrink-0 text-primary" />
              <span className="max-w-48 truncate">{activeTab.label}</span>
            </span>
            <span className="statusbar-separator" aria-hidden="true" />
            <span className="statusbar-segment font-mono" title={activeTab.target}>
              <span className="max-w-64 truncate">{activeTab.target}</span>
            </span>
            {sessionSize && (
              <>
                <span className="statusbar-separator" aria-hidden="true" />
                <span className="statusbar-segment tabular-nums">
                  {sessionSize.x} × {sessionSize.y}
                </span>
              </>
            )}
          </>
        ) : (
          <span className="statusbar-segment">{t("status.idle")}</span>
        )}
      </div>
      <div className="flex shrink-0 items-center gap-3">
        <span className="statusbar-segment">
          <VaultIcon size={12} aria-hidden="true" className={vault?.unlocked ? "text-connected" : ""} />
          {t(vaultKey)}
        </span>
        <span className="statusbar-separator" aria-hidden="true" />
        <span className="statusbar-segment">
          {sessionCount > 0 ? t("status.connections", { count: sessionCount }) : t("status.idle")}
        </span>
      </div>
    </footer>
  );
}
