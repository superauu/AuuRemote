import { KeyRound, LockKeyhole, LockOpen, Monitor } from "lucide-react";
import { useI18n } from "../i18n";
import type { VaultStatus } from "../types";

export function StatusBar({ sessionCount, vault }: {
  sessionCount: number;
  vault?: VaultStatus;
}) {
  const { t } = useI18n();
  const vaultKey = !vault?.setup ? "status.vault.notSetup" : vault.unlocked ? "status.vault.unlocked" : "status.vault.locked";
  const VaultIcon = !vault?.setup ? KeyRound : vault.unlocked ? LockOpen : LockKeyhole;

  return (
    <footer className="statusbar flex h-7 shrink-0 items-center justify-between gap-2 px-3 text-xs text-muted-foreground">
      <span className="flex items-center gap-1.5">
        <Monitor size={13} aria-hidden="true" className={sessionCount > 0 ? "text-primary" : ""} />
        {sessionCount > 0 ? t("status.connections", { count: sessionCount }) : t("status.idle")}
      </span>
      <span className="flex items-center gap-1.5">
        <VaultIcon size={13} aria-hidden="true" className={vault?.unlocked ? "text-connected" : ""} />
        {t(vaultKey)}
      </span>
    </footer>
  );
}
