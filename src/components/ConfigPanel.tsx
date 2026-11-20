import { ChevronDown, Plug, Save, Trash2 } from "lucide-react";
import { extractFingerprint } from "../errors";
import { useI18n } from "../i18n";
import { targetAddress, type BusyState, type Draft, type Notice } from "../types";

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <label className="flex flex-col gap-1.5 text-xs font-medium">
      {label}
      {children}
    </label>
  );
}

const inputClass =
  "h-8 w-full rounded-md border border-input bg-sunken px-2.5 text-xs placeholder:text-muted-foreground";

export function ConfigPanel({ draft, password, remember, selected, busy, desktop, notice, validDetails, onDraftChange, onPasswordChange, onRememberChange, onConnect, onSave, onRemove }: {
  draft: Draft;
  password: string;
  remember: boolean;
  selected?: { id: number };
  busy?: BusyState;
  desktop: boolean;
  notice?: Notice;
  validDetails: boolean;
  onDraftChange: <K extends keyof Draft>(key: K, value: Draft[K]) => void;
  onPasswordChange: (value: string) => void;
  onRememberChange: (value: boolean) => void;
  onConnect: (event: React.FormEvent<HTMLFormElement>) => void;
  onSave: () => void;
  onRemove: () => void;
}) {
  const { t } = useI18n();
  const certificateOpen = notice?.code === "certificateMismatch" || undefined;
  const fingerprint = notice?.code === "certificateMismatch" && notice.raw
    ? extractFingerprint(notice.raw)
    : undefined;

  return (
    <section className="flex w-[300px] shrink-0 flex-col border-l border-border/70" aria-label={selected ? t("config.edit") : t("config.new")}>
      <div className="panel-header">
        <span>{selected ? t("config.edit") : t("config.new")}</span>
        {selected && <span className="caption truncate">{draft.name}</span>}
      </div>
      <form onSubmit={onConnect} className="flex min-h-0 flex-1 flex-col overflow-y-auto p-3">
        <fieldset disabled={!!busy} className="flex flex-col gap-3">
          <Field label={t("config.name")}>
            <input
              className={inputClass}
              value={draft.name}
              onChange={(event) => onDraftChange("name", event.target.value)}
              placeholder={t("config.namePlaceholder")}
              maxLength={80}
              autoComplete="off"
            />
          </Field>
          <div className="grid grid-cols-[minmax(0,1fr)_80px] gap-2">
            <Field label={t("config.host")}>
              <input
                className={inputClass}
                value={draft.host}
                onChange={(event) => onDraftChange("host", event.target.value)}
                placeholder={t("config.hostPlaceholder")}
                autoComplete="off"
                spellCheck={false}
                required
              />
            </Field>
            <Field label={t("config.port")}>
              <input
                className={inputClass}
                type="number"
                min={1}
                max={65535}
                value={draft.port}
                onChange={(event) => onDraftChange("port", event.target.value)}
                required
              />
            </Field>
          </div>
          <Field label={t("config.username")}>
            <input
              className={inputClass}
              value={draft.username}
              onChange={(event) => onDraftChange("username", event.target.value)}
              placeholder={t("config.usernamePlaceholder")}
              autoComplete="off"
              spellCheck={false}
              required
            />
          </Field>
          <Field label={t("config.password")}>
            <input
              className={inputClass}
              type="password"
              value={password}
              onChange={(event) => onPasswordChange(event.target.value)}
              autoComplete="off"
              required
            />
          </Field>
          <label className="flex items-center gap-2 text-xs text-muted-foreground">
            <input
              type="checkbox"
              checked={remember}
              onChange={(event) => onRememberChange(event.target.checked)}
              className="h-3.5 w-3.5 accent-[hsl(var(--primary))]"
            />
            {t("config.rememberPassword")}
          </label>
          <details open={certificateOpen}>
            <summary className="flex cursor-pointer items-center gap-1 text-xs text-muted-foreground">
              <ChevronDown size={12} aria-hidden="true" className="transition-transform [[open]>&]:rotate-180" />
              {t("config.certificate")}
            </summary>
            <div className="mt-2 flex flex-col gap-1.5">
              <label className="flex flex-col gap-1.5 text-xs font-medium">
                {t("config.fingerprint")}
                <input
                  className={`${inputClass} font-mono`}
                  value={draft.fingerprint}
                  onChange={(event) => onDraftChange("fingerprint", event.target.value)}
                  placeholder={t("config.fingerprintPlaceholder")}
                  autoComplete="off"
                  spellCheck={false}
                />
              </label>
            </div>
          </details>
          <div className="flex flex-wrap items-center gap-2 pt-1">
            <button
              type="submit"
              className="inline-flex h-8 items-center gap-1.5 rounded-md border border-primary/60 bg-primary/90 px-3 text-xs font-medium text-primary-foreground hover:bg-primary active:scale-[0.98] dark:bg-primary/80"
              disabled={!desktop || !validDetails || !password || busy === "connecting"}
            >
              <Plug size={13} aria-hidden="true" />
              {busy === "connecting" ? t("config.connecting") : t("config.connect")}
            </button>
            <button
              type="button"
              className="inline-flex h-8 items-center gap-1.5 rounded-md border border-border bg-transparent px-3 text-xs font-medium hover:bg-accent active:scale-[0.98]"
              onClick={onSave}
              disabled={!desktop || !validDetails}
            >
              <Save size={13} aria-hidden="true" />
              {busy === "saving" ? t("config.saving") : t("config.save")}
            </button>
            {selected && (
              <button
                type="button"
                className="ms-auto inline-flex h-8 items-center gap-1.5 rounded-md border border-destructive/40 bg-transparent px-3 text-xs font-medium text-destructive hover:bg-destructive/10 active:scale-[0.98]"
                onClick={onRemove}
                disabled={!!busy}
              >
                <Trash2 size={13} aria-hidden="true" />
                {t("config.remove")}
              </button>
            )}
          </div>
        </fieldset>
        {!desktop && <p className="mt-3 text-xs text-muted-foreground">{t("config.needsDesktop")}</p>}
        {notice && (
          <div
            role="status"
            className={`mt-3 rounded-md border px-2.5 py-2 text-xs ${
              notice.kind === "error"
                ? "border-destructive/30 bg-destructive/10 text-destructive"
                : "border-border/70 bg-muted/60 text-muted-foreground"
            }`}
          >
            <strong className="font-medium">{t(notice.titleKey)}</strong>
            {notice.message && <p className="mt-1 break-words opacity-90">{notice.message}</p>}
            {fingerprint && (
              <p className="mt-1.5 break-all">
                {t("session.certificateHint")}
                <code className="mt-0.5 block font-mono text-[11px]">{fingerprint}</code>
              </p>
            )}
          </div>
        )}
        {selected && (
          <p className="mt-2 text-[11px] text-muted-foreground">
            {targetAddress(draft.host, draft.port)}
          </p>
        )}
      </form>
    </section>
  );
}
