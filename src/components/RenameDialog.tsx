import { useLayoutEffect, useRef, useState } from "react";
import { useI18n } from "../i18n";

export function RenameDialog({ initial, onSubmit, onClose }: {
  initial: string;
  onSubmit: (value: string) => void;
  onClose: () => void;
}) {
  const { t } = useI18n();
  const dialog = useRef<HTMLDialogElement>(null);
  const first = useRef<HTMLInputElement>(null);
  const [value, setValue] = useState(initial);

  useLayoutEffect(() => {
    const element = dialog.current!;
    const previousFocus = document.activeElement;
    element.showModal();
    first.current?.focus();
    first.current?.select();
    return () => {
      element.close();
      if (previousFocus instanceof HTMLElement) previousFocus.focus();
    };
  }, []);

  return (
    <dialog
      ref={dialog}
      className="dialog-card vault-dialog animate-fade-in"
      aria-labelledby="rename-title"
      onCancel={(event) => {
        event.preventDefault();
        onClose();
      }}
      onClick={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <div className="p-5">
        <h2 id="rename-title" className="text-base font-semibold">{t("tabs.renameTitle")}</h2>
        <form
          onSubmit={(event) => {
            event.preventDefault();
            onSubmit(value);
          }}
          className="mt-4 flex flex-col gap-3"
          autoComplete="off"
        >
          <label className="flex flex-col gap-1.5 text-xs font-medium">
            {t("tabs.renameLabel")}
            <input
              ref={first}
              className="field"
              value={value}
              maxLength={80}
              onChange={(event) => setValue(event.target.value)}
              autoComplete="off"
              spellCheck={false}
            />
          </label>
          <div className="flex items-center justify-end gap-2">
            <button type="button" className="btn-outline" onClick={onClose}>
              {t("tabs.renameCancel")}
            </button>
            <button type="submit" className="btn-primary" disabled={!value.trim()}>
              {t("tabs.renameSubmit")}
            </button>
          </div>
        </form>
      </div>
    </dialog>
  );
}
