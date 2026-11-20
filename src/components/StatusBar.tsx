import { KeyRound, LockKeyhole, LockOpen } from "lucide-react";
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
    <footer className="flex h-7 shrink-0 items-center gap-2 border-t border-border/70 bg-sidebar px-3 text-[12px] text-muted-foreground">
      <span>{sessionCount > 0 ? t("status.connections", { count: sessionCount }) : t("status.idle")}</span>
      <span className="h-3 w-px bg-border" aria-hidden="true" />
      <span className="flex items-center gap-1.5">
        <VaultIcon size={12} aria-hidden="true" />
        {t(vaultKey)}
      </span>
    </footer>
  );
}
