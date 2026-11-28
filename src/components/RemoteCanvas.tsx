import { invoke } from "@tauri-apps/api/core";
import { AlertTriangle, Camera, LoaderCircle, PlugZap, RefreshCw, Rocket, ShieldCheck } from "lucide-react";
import { type FormEvent, type KeyboardEvent, type PointerEvent, useCallback, useEffect, useRef, useState } from "react";
import { describeError, extractFingerprint } from "../errors";
import { useI18n } from "../i18n";
import { scanCode } from "../keyboard";
import { fitSize, type Tab } from "../types";

type Position = { x: number; y: number };
type SessionStatus =
  | { state: "connecting" | "waitingForFrame" | "ready" }
  | { state: "ended"; code?: string; detail?: string };
type Input =
  | ({ type: "move" } & Position)
  | ({ type: "button"; button: number; pressed: boolean } & Position)
  | ({ type: "wheel"; units: number; vertical: boolean } & Position)
  | { type: "key"; scanCode: number; pressed: boolean }
  | { type: "text"; text: string }
  | { type: "remoteRun" | "releaseAll" }
  | { type: "resize"; width: number; height: number };

function toRemote(canvas: HTMLCanvasElement, clientX: number, clientY: number): Position {
  const bounds = canvas.getBoundingClientRect();
  return {
    x: Math.min(canvas.width - 1, Math.max(0, Math.floor((clientX - bounds.left) * canvas.width / bounds.width))),
    y: Math.min(canvas.height - 1, Math.max(0, Math.floor((clientY - bounds.top) * canvas.height / bounds.height))),
  };
}

export function RemoteCanvas({ tab, active, autoResize, onSessionSize, onDisconnect, onReconnect, onTrustCertificate }: {
  tab: Tab;
  /** Only the active tab pulls frames; background tabs poll status at a low rate. */
  active: boolean;
  /** "auto" sessions follow the canvas size by pushing Display Control layouts. */
  autoResize: boolean;
  /** Reports the live remote framebuffer size for the status bar. */
  onSessionSize?: (id: number, size: { x: number; y: number } | undefined) => void;
  onDisconnect: (id: number, hadFrame: boolean) => void;
  onReconnect: (tab: Tab) => void;
  onTrustCertificate: (tab: Tab, fingerprint: string) => void;
}) {
  const { t } = useI18n();
  const { id, target } = tab;
  const canvas = useRef<HTMLCanvasElement>(null);
  const viewport = useRef<HTMLDivElement>(null);
  const lastResize = useRef<Position | undefined>(undefined);
  const autoResizeRef = useRef(autoResize);
  const disconnectButton = useRef<HTMLButtonElement>(null);
  const pending = useRef<Input[]>([]);
  const sending = useRef(false);
  const scheduled = useRef<number | undefined>(undefined);
  const timer = useRef<number | undefined>(undefined);
  const kick = useRef<() => void>(() => {});
  const live = useRef(false);
  const hasFrame = useRef(false);
  const activeRef = useRef(active);
  const [waitingForFrame, setWaitingForFrame] = useState(false);
  const [size, setSize] = useState<Position>();
  const [ended, setEnded] = useState<{ code?: string; detail?: string }>();
  const certificateFingerprint = ended?.code === "certificateMismatch"
    ? extractFingerprint(ended.detail ?? "")
    : undefined;
  const [text, setText] = useState("");
  const [capture, setCapture] = useState("");
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    activeRef.current = active;
    if (active && live.current) {
      // Switching back to this tab should not wait out the background interval.
      kick.current();
    }
  }, [active]);

  useEffect(() => {
    autoResizeRef.current = autoResize;
  }, [autoResize]);

  useEffect(() => {
    onSessionSize?.(id, ended ? undefined : size);
  }, [id, size, ended, onSessionSize]);

  const markEnded = useCallback((code: string | undefined, detail: string | undefined) => {
    if (!live.current) return;
    live.current = false;
    pending.current = [];
    window.clearTimeout(scheduled.current);
    setEnded({ code, detail });
  }, []);

  const send = useCallback(async () => {
    scheduled.current = undefined;
    if (sending.current || !live.current) return;
    sending.current = true;
    try {
      while (live.current && pending.current.length > 0) {
        await invoke("rdp_input", { id, events: pending.current.splice(0, 64) });
      }
    } catch (error) {
      const api = error as { code?: string; detail?: string };
      markEnded(api?.code, api?.detail ?? String(error));
    } finally {
      sending.current = false;
    }
  }, [id, markEnded]);

  // Pushes a Display Control layout matching the canvas area. Called from the
  // debounced resize observer and once right after the first frame, so an
  // "auto" session fits the canvas exactly instead of the window estimate.
  // Each push costs a session reactivation, so changes below 32px are skipped.
  const pushResize = useCallback(() => {
    const container = viewport.current;
    if (!container || !autoResizeRef.current || !live.current || !hasFrame.current) return;
    const rect = container.getBoundingClientRect();
    if (rect.width <= 0 || rect.height <= 0) return;
    const { width, height } = fitSize(rect.width, rect.height);
    const last = lastResize.current;
    if (last && Math.abs(last.x - width) < 32 && Math.abs(last.y - height) < 32) return;
    lastResize.current = { x: width, y: height };
    invoke("rdp_input", { id, events: [{ type: "resize", width, height }] }).catch((error) => {
      const api = error as { code?: string; detail?: string };
      markEnded(api?.code, api?.detail ?? String(error));
    });
  }, [id, markEnded]);

  const queue = useCallback((event: Input) => {
    if (!live.current || !size) return;
    const events = pending.current;
    if (event.type === "move" && events.at(-1)?.type === "move") {
      events[events.length - 1] = event;
    } else {
      if (events.length >= 256) {
        markEnded("sendTooLarge", "远端输入队列已满，会话已断开。");
        return;
      }
      events.push(event);
    }
    if (!sending.current && scheduled.current === undefined) {
      scheduled.current = window.setTimeout(() => { void send(); }, 8);
    }
  }, [markEnded, send, size]);

  useEffect(() => {
    const element = canvas.current;
    const context = element?.getContext("2d", { alpha: false });
    if (!element || !context) {
      live.current = false;
      setEnded({ code: "internal", detail: "无法初始化远端画布。" });
      return;
    }
    const surface = element;
    const renderer = context;
    let mounted = true;
    let after = 0;
    let firstFrame = true;
    live.current = true;

    async function nextFrame() {
      if (!live.current) return;
      try {
        if (after === 0) {
          const status = await invoke<SessionStatus>("rdp_status", { id });
          if (!mounted || !live.current) return;
          if (status.state === "ended") {
            markEnded(status.code, status.detail);
            return;
          }
          if (status.state !== "ready") {
            setWaitingForFrame(status.state === "waitingForFrame");
            timer.current = window.setTimeout(() => { void nextFrame(); }, 250);
            return;
          }
        } else if (!activeRef.current) {
          // Background tab: watch for remote-side ends, skip frame pulls.
          const status = await invoke<SessionStatus>("rdp_status", { id });
          if (!mounted || !live.current) return;
          if (status.state === "ended") {
            markEnded(status.code, status.detail);
            return;
          }
          timer.current = window.setTimeout(() => { void nextFrame(); }, 500);
          return;
        }
        const buffer = await invoke<ArrayBuffer>("rdp_frame", { id, after });
        if (!mounted || !live.current) return;
        if (buffer.byteLength > 0) {
          const header = new DataView(buffer);
          after = header.getUint32(0, true);
          const width = header.getUint16(4, true);
          const height = header.getUint16(6, true);
          if (surface.width !== width || surface.height !== height || surface.dataset.ready !== "true") {
            surface.width = width;
            surface.height = height;
            surface.dataset.ready = "true";
            setSize({ x: width, y: height });
          }
          renderer.putImageData(new ImageData(new Uint8ClampedArray(buffer, 8), width, height), 0, 0);
          hasFrame.current = true;
          if (firstFrame) {
            firstFrame = false;
            // Exact-fit once frames flow; further pushes come from the observer.
            if (autoResizeRef.current) pushResize();
          }
        }
        timer.current = window.setTimeout(() => { void nextFrame(); }, activeRef.current ? 33 : 500);
      } catch (error) {
        if (mounted) {
          const api = error as { code?: string; detail?: string };
          markEnded(api?.code, api?.detail ?? String(error));
        }
      }
    }

    void nextFrame();
    kick.current = () => {
      window.clearTimeout(timer.current);
      timer.current = window.setTimeout(() => { void nextFrame(); }, 0);
    };
    return () => {
      mounted = false;
      live.current = false;
      kick.current = () => {};
      pending.current = [];
      window.clearTimeout(timer.current);
      window.clearTimeout(scheduled.current);
    };
  }, [id, markEnded, pushResize]);

  // "auto" sessions push a new Display Control layout once the canvas area
  // settles into a different size; the remote resizes itself and later frames
  // arrive at the new dimensions.
  useEffect(() => {
    const container = viewport.current;
    if (!container || !autoResize) return;
    let scheduled = 0;
    const observer = new ResizeObserver(() => {
      window.clearTimeout(scheduled);
      scheduled = window.setTimeout(() => pushResize(), 300);
    });
    observer.observe(container);
    return () => {
      observer.disconnect();
      window.clearTimeout(scheduled);
    };
  }, [autoResize, pushResize]);

  useEffect(() => {
    const surface = canvas.current;
    if (!surface || !size) return;
    function wheel(event: WheelEvent) {
      event.preventDefault();
      const vertical = Math.abs(event.deltaY) >= Math.abs(event.deltaX);
      const delta = vertical ? event.deltaY : event.deltaX;
      if (delta === 0) return;
      queue({ type: "wheel", ...toRemote(surface!, event.clientX, event.clientY),
        units: (vertical ? -1 : 1) * Math.sign(delta) * 120, vertical });
    }
    function release() { queue({ type: "releaseAll" }); }
    surface.addEventListener("wheel", wheel, { passive: false });
    window.addEventListener("blur", release);
    return () => {
      surface.removeEventListener("wheel", wheel);
      window.removeEventListener("blur", release);
    };
  }, [queue, size]);

  function button(event: PointerEvent<HTMLCanvasElement>, pressed: boolean) {
    if (!size || ended || event.button < 0 || event.button > 4) return;
    event.preventDefault();
    if (pressed) {
      event.currentTarget.focus();
      event.currentTarget.setPointerCapture(event.pointerId);
    }
    queue({ type: "button", pressed, button: event.button,
      ...toRemote(event.currentTarget, event.clientX, event.clientY) });
  }

  function keyboard(event: KeyboardEvent<HTMLCanvasElement>, pressed: boolean) {
    if (pressed && event.ctrlKey && event.altKey && (event.code === "Escape" || event.key === "Escape")) {
      event.preventDefault();
      disconnectButton.current?.focus();
      return;
    }
    // F11 toggles the app window fullscreen locally (handled globally in App);
    // it must not also reach the remote desktop.
    if (event.key === "F11") {
      event.preventDefault();
      return;
    }
    const code = scanCode(event);
    // The canvas sends physical keys to the remote keyboard, including while a
    // local IME is active. The text box is the entry point for composed text.
    if (code === undefined) return;
    event.preventDefault();
    queue({ type: "key", scanCode: code, pressed });
  }

  function sendText(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!text) return;
    queue({ type: "text", text });
    setText("");
    canvas.current?.focus();
  }

  async function screenshot() {
    setSaving(true);
    setCapture("");
    try {
      if (await invoke<boolean>("rdp_screenshot", { id })) setCapture(t("session.screenshotSaved"));
    } catch (error) {
      setCapture(describeError(error, t));
    } finally {
      setSaving(false);
    }
  }

  const statusLine = ended
    ? describeError({ code: ended.code ?? "connectionFailed", detail: ended.detail ?? "" }, t)
    : size ? t("session.connected", { width: size.x, height: size.y })
    : waitingForFrame ? t("session.waitingFrame") : t("session.connecting");

  return (
    <div className="flex h-full flex-col">
      <div className="session-toolbar flex h-10 shrink-0 items-center justify-between gap-2 px-3">
        <div className="flex min-w-0 items-center gap-2">
          <span
            className={`state-dot ${ended ? "bg-ended" : size ? "bg-connected" : "bg-connecting animate-pulse-subtle"}`}
            aria-hidden="true"
          />
          <strong className="truncate font-mono text-xs font-medium" title={target}>{target}</strong>
          <span className="session-status truncate text-xs text-muted-foreground" title={statusLine}>{statusLine}</span>
        </div>
        <div className="flex shrink-0 items-center gap-1">
          <button
            type="button"
            className="session-action"
            title={t("session.winR")}
            aria-label={t("session.winR")}
            disabled={!size || !!ended}
            onClick={() => {
              queue({ type: "remoteRun" });
              canvas.current?.focus();
            }}
          >
            <Rocket size={13} aria-hidden="true" />
            <span>{t("session.winR")}</span>
          </button>
          <button
            type="button"
            className="session-action"
            title={t("session.screenshot")}
            aria-label={t("session.screenshot")}
            disabled={!size || !!ended || saving}
            onClick={() => void screenshot()}
          >
            <Camera size={13} aria-hidden="true" />
            <span>{t("session.screenshot")}</span>
          </button>
          <button
            ref={disconnectButton}
            type="button"
            className="session-action"
            title={size ? t("session.disconnect") : t("session.cancel")}
            aria-label={size ? t("session.disconnect") : t("session.cancel")}
            onClick={() => onDisconnect(id, hasFrame.current)}
          >
            <PlugZap size={13} aria-hidden="true" />
            <span>{size ? t("session.disconnect") : t("session.cancel")}</span>
          </button>
        </div>
      </div>
      <div ref={viewport} className="relative grid min-h-0 flex-1 place-items-center overflow-hidden bg-sunken">
        <canvas
          ref={canvas}
          hidden={!size}
          tabIndex={0}
          aria-label={t("session.canvasLabel")}
          className={`max-h-full max-w-full touch-none ${ended ? "opacity-40" : ""}`}
          onContextMenu={(event) => event.preventDefault()}
          onPointerMove={(event) => queue({ type: "move", ...toRemote(event.currentTarget, event.clientX, event.clientY) })}
          onPointerDown={(event) => button(event, true)}
          onPointerUp={(event) => button(event, false)}
          onPointerCancel={() => queue({ type: "releaseAll" })}
          onKeyDown={(event) => keyboard(event, true)}
          onKeyUp={(event) => keyboard(event, false)}
          onBlur={() => queue({ type: "releaseAll" })}
        />
        {!size && !ended && (
          <div className="absolute inset-0 grid place-items-center p-4" role="status">
            <div className="flex flex-col items-center gap-3 text-xs text-muted-foreground">
              <LoaderCircle size={24} aria-hidden="true" className="animate-spin text-primary motion-reduce:animate-none" />
              {statusLine}
            </div>
          </div>
        )}
        {ended && (
          <div className="absolute inset-0 grid place-items-center p-4">
            <div className="dialog-card w-full max-w-[430px] p-5 text-center animate-fade-in">
              <AlertTriangle size={22} aria-hidden="true" className="mx-auto text-ended" />
              <h2 className="mt-2 text-base font-semibold">{t("session.ended")}</h2>
              <p className="mt-1.5 break-words text-xs text-muted-foreground">{statusLine}</p>
              {ended.code === "certificateMismatch" && (
                <p className="mt-1.5 break-all text-[11px] text-muted-foreground">
                  {t("session.certificateHint")}
                  <code className="mt-0.5 block font-mono">{certificateFingerprint ?? ""}</code>
                </p>
              )}
              <div className="mt-4 flex flex-wrap items-center justify-center gap-2">
                {ended.code === "certificateMismatch" && certificateFingerprint && (
                  <button
                    type="button"
                    className="btn-primary"
                    title={t("session.trustCertificateTitle")}
                    onClick={() => onTrustCertificate(tab, certificateFingerprint)}
                  >
                    <ShieldCheck size={13} aria-hidden="true" />
                    {t("session.trustCertificate")}
                  </button>
                )}
                {tab.profileId !== undefined && (
                  <button type="button" className="btn-outline" onClick={() => onReconnect(tab)}>
                    <RefreshCw size={13} aria-hidden="true" />
                    {t("session.reconnectButton")}
                  </button>
                )}
                <button type="button" className="btn-outline" onClick={() => onDisconnect(id, hasFrame.current)}>
                  {t("tabs.close")}
                </button>
              </div>
            </div>
          </div>
        )}
      </div>
      <footer className="session-footer flex h-10 shrink-0 items-center justify-between gap-2 px-3">
        <form
          className="flex w-[380px] min-w-0 items-center gap-2"
          onSubmit={sendText}
        >
          <input
            aria-label={t("session.sendLabel")}
            placeholder={t("session.sendPlaceholder")}
            value={text}
            onChange={(event) => setText(event.target.value)}
            maxLength={64}
            disabled={!size || !!ended}
            autoComplete="off"
            className="field !h-7"
          />
          <button
            type="submit"
            className="btn-outline !h-7 shrink-0"
            disabled={!size || !!ended || !text}
          >
            {t("session.send")}
          </button>
        </form>
        <span className="session-hint truncate text-xs text-muted-foreground" data-capture={!!capture} role="status">
          {capture || t("session.hint")}
        </span>
      </footer>
    </div>
  );
}
