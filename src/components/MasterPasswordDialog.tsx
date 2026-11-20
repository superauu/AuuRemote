import { KeyRound } from "lucide-react";
import { useLayoutEffect, useRef, useState } from "react";
import { describeError } from "../errors";
import { useI18n } from "../i18n";

export type VaultDialogMode = "create" | "unlock" | "change";

export function MasterPasswordDialog({ mode, pending, error, onSubmit, onClose }: {
  mode: VaultDialogMode;
  pending: boolean;
  error?: unknown;
  onSubmit: (values: { oldPassword?: string; newPassword: string }) => void;
  onClose: () => void;
}) {
  const { t } = useI18n();
  const dialog = useRef<HTMLDialogElement>(null);
  const first = useRef<HTMLInputElement>(null);
  const [oldPassword, setOldPassword] = useState("");
  const [newPassword, setNewPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const [confirmMismatch, setConfirmMismatch] = useState(false);

  useLayoutEffect(() => {
    const element = dialog.current!;
    const previousFocus = document.activeElement;
    element.showModal();
    first.current?.focus();
    return () => {
      element.close();
      if (previousFocus instanceof HTMLElement) previousFocus.focus();
    };
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
    <dialog
      ref={dialog}
      className="dialog-card vault-dialog animate-fade-in"
      aria-labelledby="vault-title"
      aria-describedby="vault-description"
      onCancel={(event) => {
        event.preventDefault();
        if (!pending) onClose();
      }}
      onClick={(event) => {
        if (event.target === event.currentTarget && !pending) onClose();
      }}
    >
      <div className="p-5">
        <span className="vault-icon" aria-hidden="true"><KeyRound size={18} /></span>
        <h2 id="vault-title" className="mt-3 text-base font-semibold">{t(titleKey)}</h2>
        <p id="vault-description" className="mt-1 text-xs leading-[18px] text-muted-foreground">{t(descriptionKey)}</p>
        <form onSubmit={submit} className="mt-4 flex flex-col gap-3" autoComplete="off">
          {mode === "change" && (
            <label className="flex flex-col gap-1.5 text-xs font-medium">
              {t("vault.oldPassword")}
              <input
                ref={first}
                className="field"
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
              ref={mode !== "change" ? first : undefined}
              className="field"
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
                className="field"
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
              className="btn-outline"
              onClick={onClose}
              disabled={pending}
            >
              {t("vault.cancel")}
            </button>
            <button
              type="submit"
              className="btn-primary"
              disabled={pending}
            >
              {t(submitKey)}
            </button>
          </div>
        </form>
      </div>
    </dialog>
  );
}
