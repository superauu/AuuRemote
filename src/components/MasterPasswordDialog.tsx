import { useEffect, useRef, useState } from "react";
import { describeError } from "../errors";
import { useI18n } from "../i18n";

export type VaultDialogMode = "create" | "unlock" | "change";

const inputClass =
  "h-8 w-full rounded-md border border-input bg-sunken px-2.5 text-xs placeholder:text-muted-foreground";

export function MasterPasswordDialog({ mode, pending, error, onSubmit, onClose }: {
  mode: VaultDialogMode;
  pending: boolean;
  error?: unknown;
  onSubmit: (values: { oldPassword?: string; newPassword: string }) => void;
  onClose: () => void;
}) {
  const { t } = useI18n();
  const first = useRef<HTMLInputElement>(null);
  const [oldPassword, setOldPassword] = useState("");
  const [newPassword, setNewPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const [confirmMismatch, setConfirmMismatch] = useState(false);

  useEffect(() => {
    first.current?.focus();
  }, []);

  const titleKey = mode === "create" ? "vault.createTitle" : mode === "unlock" ? "vault.unlockTitle" : "vault.changeTitle";
  const descriptionKey = mode === "create" ? "vault.createDescription" : mode === "unlock" ? "vault.unlockDescription" : "vault.changeDescription";
  const submitKey = mode === "create" ? "vault.createSubmit" : mode === "unlock" ? "vault.unlockSubmit" : "vault.changeSubmit";

  function submit(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (pending) return;
    if (mode !== "unlock" && newPassword !== confirm) {
      setConfirmMismatch(true);
      return;
    }
    onSubmit(mode === "unlock" ? { newPassword } : { oldPassword, newPassword });
  }

  return (
    <div className="dialog-scrim fixed inset-0 z-50 grid place-items-center p-4" onClick={onClose}>
      <div
        className="dialog-card w-full max-w-[380px] p-5 animate-fade-in"
        role="dialog"
        aria-modal="true"
        aria-label={t(titleKey)}
        onClick={(event) => event.stopPropagation()}
      >
        <h2 className="text-sm font-semibold">{t(titleKey)}</h2>
        <p className="mt-1 text-xs text-muted-foreground">{t(descriptionKey)}</p>
        <form onSubmit={submit} className="mt-4 flex flex-col gap-3" autoComplete="off">
          {mode === "change" && (
            <label className="flex flex-col gap-1.5 text-xs font-medium">
              {t("vault.oldPassword")}
              <input
                className={inputClass}
                type="password"
                value={oldPassword}
                onChange={(event) => setOldPassword(event.target.value)}
                autoComplete="off"
                required
              />
            </label>
          )}
          <label className="flex flex-col gap-1.5 text-xs font-medium">
            {mode === "change" ? t("vault.newPassword") : t("vault.masterPassword")}
            <input
              ref={first}
              className={inputClass}
              type="password"
              value={newPassword}
              onChange={(event) => setNewPassword(event.target.value)}
              autoComplete="new-password"
              required
            />
          </label>
          {mode !== "unlock" && (
            <label className="flex flex-col gap-1.5 text-xs font-medium">
              {t("vault.confirmPassword")}
              <input
                className={inputClass}
                type="password"
                value={confirm}
                onChange={(event) => {
                  setConfirm(event.target.value);
                  setConfirmMismatch(false);
                }}
                autoComplete="new-password"
                required
              />
            </label>
          )}
          {(confirmMismatch || error !== undefined) && (
            <p className="text-xs text-destructive" role="status">
              {confirmMismatch ? t("vault.confirmMismatch") : describeError(error, t)}
            </p>
          )}
          <div className="mt-1 flex items-center justify-end gap-2">
            <button
              type="button"
              className="inline-flex h-8 items-center rounded-md border border-border px-3 text-xs font-medium hover:bg-accent active:scale-[0.98]"
              onClick={onClose}
              disabled={pending}
            >
              {t("vault.cancel")}
            </button>
            <button
              type="submit"
              className="inline-flex h-8 items-center rounded-md border border-primary/60 bg-primary/90 px-3 text-xs font-medium text-primary-foreground hover:bg-primary active:scale-[0.98] dark:bg-primary/80"
              disabled={pending}
            >
              {t(submitKey)}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}
