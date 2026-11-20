import { ChevronDown, Eye, EyeOff, Plug, Save, Trash2 } from "lucide-react";
import { useState } from "react";
import { extractFingerprint } from "../errors";
import { useI18n } from "../i18n";
import { targetAddress, type BusyState, type Draft, type Notice } from "../types";

function Section({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex flex-col gap-2">
      <span className="field-caption">{label}</span>
      {children}
    </div>
  );
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <label className="flex flex-col gap-1.5 text-xs font-medium">
      {label}
      {children}
    </label>
  );
}

export function ConfigPanel({ draft, password, remember, selected, busy, desktop, notice, validDetails, onDraftChange, onPasswordChange, onRememberChange, onConnect, onSave, onRemove, width }: {
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
  width: number;
}) {
  const { t } = useI18n();
  const [showPassword, setShowPassword] = useState(false);
  const certificateOpen = notice?.code === "certificateMismatch" || undefined;
  const fingerprint = notice?.code === "certificateMismatch" && notice.raw
    ? extractFingerprint(notice.raw)
    : undefined;

  return (
    <section
      className="flex flex-col overflow-hidden panel-surface"
      style={{ width, borderLeft: "1px solid var(--glass-divider)" }}
      aria-label={selected ? t("config.edit") : t("config.new")}
    >
      <div className="panel-header">
        <span className="truncate">{selected ? t("config.edit") : t("config.new")}</span>
        {selected && (
          <span className="count-pill ms-auto max-w-[140px] truncate !normal-case" title={draft.name}>
            {targetAddress(draft.host, draft.port)}
          </span>
        )}
      </div>
      <form onSubmit={onConnect} className="flex min-h-0 flex-1 flex-col">
        <div className="min-h-0 flex-1 overflow-y-auto p-3">
          <fieldset disabled={!!busy} className="flex flex-col gap-4">
            <Section label={t("config.name")}>
              <input
                className="field"
                value={draft.name}
                onChange={(event) => onDraftChange("name", event.target.value)}
                placeholder={t("config.namePlaceholder")}
                maxLength={80}
                autoComplete="off"
              />
            </Section>
            <Section label={t("config.section.server")}>
              <div className="flex flex-col gap-2.5">
                <div className="grid grid-cols-[minmax(0,1fr)_86px] gap-2">
                  <Field label={t("config.host")}>
                    <input
                      className="field"
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
                      className="field"
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
                    className="field"
                    value={draft.username}
                    onChange={(event) => onDraftChange("username", event.target.value)}
                    placeholder={t("config.usernamePlaceholder")}
                    autoComplete="off"
                    spellCheck={false}
                    required
                  />
                </Field>
              </div>
            </Section>
            <Section label={t("config.section.credentials")}>
              <Field label={t("config.password")}>
                <div className="relative">
                  <input
                    className="field !pe-8 font-mono"
                    type={showPassword ? "text" : "password"}
                    value={password}
                    onChange={(event) => onPasswordChange(event.target.value)}
                    autoComplete="off"
                    required
                  />
                  <button
                    type="button"
                    className="icon-btn !absolute end-0.5 top-1/2 !h-6 !w-6 -translate-y-1/2"
                    title={showPassword ? t("config.hidePassword") : t("config.showPassword")}
                    aria-label={showPassword ? t("config.hidePassword") : t("config.showPassword")}
                    onClick={() => setShowPassword((current) => !current)}
                    tabIndex={-1}
                  >
                    {showPassword ? <EyeOff size={13} aria-hidden="true" /> : <Eye size={13} aria-hidden="true" />}
                  </button>
                </div>
              </Field>
              <label className="flex cursor-pointer items-center gap-2 text-xs text-muted-foreground">
                <input
                  type="checkbox"
                  checked={remember}
                  onChange={(event) => onRememberChange(event.target.checked)}
                  className="h-3.5 w-3.5 accent-[hsl(var(--primary))]"
                />
                {t("config.rememberPassword")}
              </label>
            </Section>
            <Section label={t("config.certificate")}>
              <details open={certificateOpen}>
                <summary className="flex cursor-pointer items-center gap-1 text-xs text-muted-foreground">
                  <ChevronDown size={12} aria-hidden="true" className="transition-transform [[open]>&]:rotate-180" />
                  {t("config.fingerprint")}
                </summary>
                <input
                  className="field mt-2 font-mono text-[11px]"
                  value={draft.fingerprint}
                  onChange={(event) => onDraftChange("fingerprint", event.target.value)}
                  placeholder={t("config.fingerprintPlaceholder")}
                  autoComplete="off"
                  spellCheck={false}
                />
              </details>
            </Section>
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
        </div>
        <div
          className="flex shrink-0 flex-col gap-2 p-3"
          style={{ borderTop: "1px solid var(--glass-divider)" }}
        >
          <button type="submit" className="btn-primary w-full" disabled={!desktop || !validDetails || !password || busy === "connecting"}>
            <Plug size={13} aria-hidden="true" />
            {busy === "connecting" ? t("config.connecting") : t("config.connect")}
          </button>
          <div className="flex items-center gap-2">
            <button type="button" className="btn-outline flex-1" onClick={onSave} disabled={!desktop || !validDetails}>
              <Save size={13} aria-hidden="true" />
              {busy === "saving" ? t("config.saving") : t("config.save")}
            </button>
            {selected && (
              <button
                type="button"
                className="btn-outline btn-danger-outline !px-2.5"
                title={t("config.remove")}
                aria-label={t("config.remove")}
                onClick={onRemove}
                disabled={!!busy}
              >
                <Trash2 size={13} aria-hidden="true" />
              </button>
            )}
          </div>
        </div>
      </form>
    </section>
  );
}
