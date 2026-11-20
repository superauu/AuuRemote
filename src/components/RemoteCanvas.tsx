import { invoke } from "@tauri-apps/api/core";
import { AlertTriangle, Camera, PlugZap, RefreshCw, Rocket } from "lucide-react";
import { type FormEvent, type KeyboardEvent, type PointerEvent, useCallback, useEffect, useRef, useState } from "react";
import { describeError } from "../errors";
import { useI18n } from "../i18n";
import { scanCode } from "../keyboard";
import type { Tab } from "../types";

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
  | { type: "remoteRun" | "releaseAll" };

function toRemote(canvas: HTMLCanvasElement, clientX: number, clientY: number): Position {
  const bounds = canvas.getBoundingClientRect();
  return {
    x: Math.min(canvas.width - 1, Math.max(0, Math.floor((clientX - bounds.left) * canvas.width / bounds.width))),
    y: Math.min(canvas.height - 1, Math.max(0, Math.floor((clientY - bounds.top) * canvas.height / bounds.height))),
  };
}

export function RemoteCanvas({ tab, active, onDisconnect, onReconnect }: {
  tab: Tab;
  /** Only the active tab pulls frames; background tabs poll status at a low rate. */
  active: boolean;
  onDisconnect: (id: number, hadFrame: boolean) => void;
  onReconnect: (tab: Tab) => void;
}) {
  const { t } = useI18n();
  const { id, target } = tab;
  const canvas = useRef<HTMLCanvasElement>(null);
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
  }, [id, markEnded]);

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
      <div className="flex h-9 shrink-0 items-center justify-between gap-2 border-b border-border/70 bg-sidebar px-3">
        <div className="flex min-w-0 items-center gap-2">
          <span
            className={`state-dot ${ended ? "bg-ended" : size ? "bg-connected animate-pulse-subtle" : "bg-connecting animate-pulse-subtle"}`}
            aria-hidden="true"
          />
          <strong className="truncate text-xs" title={target}>{target}</strong>
          <span className="hidden truncate text-[11px] text-muted-foreground sm:block">{statusLine}</span>
        </div>
        <div className="flex shrink-0 items-center gap-1">
          <button
            type="button"
            className="inline-flex h-7 items-center rounded-md px-2 text-[12px] font-medium text-muted-foreground hover:bg-accent hover:text-accent-foreground active:scale-[0.98]"
            disabled={!size || !!ended}
            onClick={() => {
              queue({ type: "remoteRun" });
              canvas.current?.focus();
            }}
          >
            <Rocket size={13} aria-hidden="true" className="me-1" />
            {t("session.winR")}
          </button>
          <button
            type="button"
            className="inline-flex h-7 items-center rounded-md px-2 text-[12px] font-medium text-muted-foreground hover:bg-accent hover:text-accent-foreground active:scale-[0.98]"
            disabled={!size || !!ended || saving}
            onClick={() => void screenshot()}
          >
            <Camera size={13} aria-hidden="true" className="me-1" />
            {t("session.screenshot")}
          </button>
          <button
            ref={disconnectButton}
            type="button"
            className="inline-flex h-7 items-center rounded-md px-2 text-[12px] font-medium text-muted-foreground hover:bg-accent hover:text-accent-foreground active:scale-[0.98]"
            onClick={() => onDisconnect(id, hasFrame.current)}
          >
            <PlugZap size={13} aria-hidden="true" className="me-1" />
            {size ? t("session.disconnect") : t("session.cancel")}
          </button>
        </div>
      </div>
      <div className="relative grid min-h-0 flex-1 place-items-center overflow-hidden bg-sunken">
        <canvas
          ref={canvas}
          hidden={!size}
          tabIndex={0}
          aria-label={t("session.canvasLabel")}
          className={`max-h-full max-w-full ${ended ? "opacity-40" : ""}`}
          style={{ touchAction: "none" }}
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
          <p className="absolute inset-0 grid place-items-center text-xs text-muted-foreground" role="status">
            {statusLine}
          </p>
        )}
        {ended && (
          <div className="absolute inset-0 grid place-items-center p-4">
            <div className="dialog-card w-full max-w-[430px] p-5 text-center animate-fade-in">
              <AlertTriangle size={22} aria-hidden="true" className="mx-auto text-ended" />
              <h2 className="mt-2 text-sm font-semibold">{t("session.ended")}</h2>
              <p className="mt-1.5 break-words text-xs text-muted-foreground">{statusLine}</p>
              <div className="mt-4 flex items-center justify-center gap-2">
                {tab.profileId !== undefined && (
                  <button
                    type="button"
                    className="inline-flex h-8 items-center gap-1.5 rounded-md border border-primary/60 bg-primary/90 px-3 text-xs font-medium text-primary-foreground hover:bg-primary active:scale-[0.98] dark:bg-primary/80"
                    onClick={() => onReconnect(tab)}
                  >
                    <RefreshCw size={13} aria-hidden="true" />
                    {t("session.reconnectButton")}
                  </button>
                )}
                <button
                  type="button"
                  className="inline-flex h-8 items-center rounded-md border border-border px-3 text-xs font-medium hover:bg-accent active:scale-[0.98]"
                  onClick={() => onDisconnect(id, hasFrame.current)}
                >
                  {t("tabs.close")}
                </button>
              </div>
            </div>
          </div>
        )}
      </div>
      <footer className="flex h-9 shrink-0 items-center justify-between gap-2 border-t border-border/70 bg-sidebar px-3">
        <form
          className="flex w-[380px] max-w-[65%] items-center gap-2"
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
            className="h-7 w-full rounded-md border border-input bg-transparent px-2.5 text-xs placeholder:text-muted-foreground"
          />
          <button
            type="submit"
            className="inline-flex h-7 shrink-0 items-center rounded-md border border-border px-2.5 text-[12px] font-medium hover:bg-accent active:scale-[0.98]"
            disabled={!size || !!ended || !text}
          >
            {t("session.send")}
          </button>
        </form>
        <span className="hidden truncate text-[11px] text-muted-foreground md:block" role="status">
          {capture || t("session.hint")}
        </span>
      </footer>
    </div>
  );
}
