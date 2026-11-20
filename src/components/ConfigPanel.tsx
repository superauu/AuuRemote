import { ChevronRight, Eye, EyeOff, KeyRound, Plug, Save, Server, ShieldCheck, SlidersHorizontal, Trash2, X } from "lucide-react";
import { type ReactNode, type Ref, useState } from "react";
import { extractFingerprint } from "../errors";
import { useI18n } from "../i18n";
import type { BusyState, Draft, Notice } from "../types";

function Section({ label, icon, children }: { label: string; icon: ReactNode; children: ReactNode }) {
  return (
    <section className="config-section" aria-label={label}>
      <h3 className="section-heading">{icon}{label}</h3>
      {children}
    </section>
  );
}

function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <label className="flex flex-col gap-1.5 text-xs font-medium">
      {label}
      {children}
    </label>
  );
}

export function ConfigPanel({ open, onClose, hostField, draft, password, remember, selected, busy, desktop, notice, validDetails, onDraftChange, onPasswordChange, onRememberChange, onConnect, onSave, onRemove, width }: {
  open: boolean;
  onClose: () => void;
  hostField: Ref<HTMLInputElement>;
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
      id="connection-config"
      hidden={!open}
      className="panel-surface flex shrink-0 flex-col overflow-hidden"
      style={{ width }}
      aria-labelledby="connection-config-title"
    >
      <div className="panel-header">
        <SlidersHorizontal size={14} className="shrink-0 text-muted-foreground" aria-hidden="true" />
        <h2 id="connection-config-title" className="truncate">{selected ? t("config.edit") : t("config.new")}</h2>
        <button type="button" className="icon-btn ms-auto" aria-label={t("config.hide")} title={t("config.hide")} onClick={onClose}>
          <X size={15} aria-hidden="true" />
        </button>
      </div>
      <form onSubmit={onConnect} className="flex min-h-0 flex-1 flex-col">
        <div className="config-body min-h-0 flex-1 overflow-y-auto">
          <fieldset disabled={!!busy} className="config-fields flex min-w-0 flex-col">
            <Field label={t("config.name")}>
              <input
                className="field"
                value={draft.name}
                onChange={(event) => onDraftChange("name", event.target.value)}
                placeholder={t("config.namePlaceholder")}
                maxLength={80}
                autoComplete="off"
              />
            </Field>
            <Section label={t("config.section.server")} icon={<Server size={14} aria-hidden="true" />}>
              <div className="flex flex-col gap-3">
                <div className="grid grid-cols-[minmax(0,1fr)_72px] gap-2">
                  <Field label={t("config.host")}>
                    <input
                      className="field"
                      ref={hostField}
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
            <Section label={t("config.section.credentials")} icon={<KeyRound size={14} aria-hidden="true" />}>
              <div className="flex flex-col gap-1.5">
                <label htmlFor="connection-password" className="text-xs font-medium">{t("config.password")}</label>
                <div className="relative">
                  <input
                    id="connection-password"
                    className={`field pe-9${showPassword ? " font-mono" : ""}`}
                    type={showPassword ? "text" : "password"}
                    value={password}
                    onChange={(event) => onPasswordChange(event.target.value)}
                    autoComplete="off"
                    required
                  />
                  <button
                    type="button"
                    className="icon-btn absolute end-0.5 top-1/2 -translate-y-1/2"
                    title={showPassword ? t("config.hidePassword") : t("config.showPassword")}
                    aria-label={showPassword ? t("config.hidePassword") : t("config.showPassword")}
                    onClick={() => setShowPassword((current) => !current)}
                  >
                    {showPassword ? <EyeOff size={13} aria-hidden="true" /> : <Eye size={13} aria-hidden="true" />}
                  </button>
                </div>
              </div>
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
            <details open={certificateOpen} className="certificate-section">
              <summary className="section-heading">
                <ShieldCheck size={14} aria-hidden="true" />
                <span className="flex-1">{t("config.certificate")}</span>
                <ChevronRight size={14} aria-hidden="true" className="certificate-chevron" />
              </summary>
              <Field label={t("config.fingerprint")}>
                <input
                  className="field font-mono text-[11px]"
                  value={draft.fingerprint}
                  onChange={(event) => onDraftChange("fingerprint", event.target.value)}
                  placeholder={t("config.fingerprintPlaceholder")}
                  autoComplete="off"
                  spellCheck={false}
                />
              </Field>
            </details>
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
        <div className="panel-actions">
          <button type="submit" className="btn-primary w-full" disabled={!desktop || !validDetails || !password || !!busy}>
            <Plug size={13} aria-hidden="true" />
            {busy === "connecting" ? t("config.connecting") : t("config.connect")}
          </button>
          <div className="flex items-center gap-2">
            <button type="button" className="btn-outline flex-1" onClick={onSave} disabled={!desktop || !validDetails || !!busy}>
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
