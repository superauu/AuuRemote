import { invoke } from "@tauri-apps/api/core";
import { type FormEvent, type KeyboardEvent, type PointerEvent, useCallback, useEffect, useRef, useState } from "react";
import { scanCode } from "./keyboard";

type Position = { x: number; y: number };
type SessionStatus =
  | { state: "connecting" | "waitingForFrame" | "ready" }
  | { state: "ended"; message: string };
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

export function RemoteDesktop({ id, target, close }: {
  id: number;
  target: string;
  close: (id: number, hadFrame: boolean, error?: string) => void;
}) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const disconnect = useRef<HTMLButtonElement>(null);
  const pending = useRef<Input[]>([]);
  const sending = useRef(false);
  const scheduled = useRef<number | undefined>(undefined);
  const live = useRef(false);
  const hasFrame = useRef(false);
  const [waitingForFrame, setWaitingForFrame] = useState(false);
  const [size, setSize] = useState<Position>();
  const [text, setText] = useState("");
  const [capture, setCapture] = useState("");
  const [saving, setSaving] = useState(false);

  const end = useCallback((error?: string) => {
    if (!live.current) return;
    live.current = false;
    close(id, hasFrame.current, error);
  }, [close, id]);

  const send = useCallback(async () => {
    scheduled.current = undefined;
    if (sending.current || !live.current) return;
    sending.current = true;
    try {
      while (live.current && pending.current.length > 0) {
        await invoke("rdp_input", { id, events: pending.current.splice(0, 64) });
      }
    } catch (error) {
      end(String(error));
    } finally {
      sending.current = false;
    }
  }, [end, id]);

  const queue = useCallback((event: Input) => {
    if (!live.current || !size) return;
    const events = pending.current;
    if (event.type === "move" && events.at(-1)?.type === "move") {
      events[events.length - 1] = event;
    } else {
      if (events.length >= 256) {
        end("远端输入队列已满，会话已断开。");
        return;
      }
      events.push(event);
    }
    if (!sending.current && scheduled.current === undefined) {
      scheduled.current = window.setTimeout(() => { void send(); }, 8);
    }
  }, [end, send, size]);

  useEffect(() => {
    const element = canvas.current;
    const context = element?.getContext("2d", { alpha: false });
    if (!element || !context) {
      close(id, false, "无法初始化远端画布。");
      return;
    }
    const surface = element;
    const renderer = context;
    let active = true;
    let timer: number | undefined;
    let after = 0;
    live.current = true;

    async function nextFrame() {
      if (!live.current) return;
      try {
        if (after === 0) {
          const status = await invoke<SessionStatus>("rdp_status", { id });
          if (!active || !live.current) return;
          if (status.state === "ended") {
            end(status.message);
            return;
          }
          if (status.state !== "ready") {
            setWaitingForFrame(status.state === "waitingForFrame");
            timer = window.setTimeout(() => { void nextFrame(); }, 250);
            return;
          }
        }
        const buffer = await invoke<ArrayBuffer>("rdp_frame", { id, after });
        if (!active || !live.current) return;
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
        timer = window.setTimeout(() => { void nextFrame(); }, 33);
      } catch (error) {
        if (active) end(String(error));
      }
    }

    void nextFrame();
    return () => {
      active = false;
      live.current = false;
      pending.current = [];
      window.clearTimeout(timer);
      window.clearTimeout(scheduled.current);
    };
  }, [close, end, id]);

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
    if (!size || event.button < 0 || event.button > 4) return;
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
      disconnect.current?.focus();
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
      if (await invoke<boolean>("rdp_screenshot", { id })) setCapture("已保存远端截图。");
    } catch (error) {
      setCapture(String(error));
    } finally {
      setSaving(false);
    }
  }

  const progress = waitingForFrame ? "会话已建立，正在接收首帧…" : "正在连接远端主机…";
  return (
    <main className="session">
      <div className="session-bar">
        <div className="session-name">
          <strong title={target}>{target}</strong>
          {size && <span>已连接 · {size.x} × {size.y}</span>}
        </div>
        <div className="session-actions">
          <button type="button" className="secondary" disabled={!size} onClick={() => {
            queue({ type: "remoteRun" });
            canvas.current?.focus();
          }}>远端 Win+R</button>
          <button type="button" className="secondary" disabled={!size || saving} onClick={() => void screenshot()}>保存截图</button>
          <button ref={disconnect} type="button" className="secondary" onClick={() => end()}>{size ? "断开" : "取消连接"}</button>
        </div>
      </div>
      <div className="viewport">
        {!size && <p className="waiting" role="status">{progress}</p>}
        <canvas
          ref={canvas}
          hidden={!size}
          tabIndex={0}
          aria-label="远端桌面。按 Ctrl+Alt+Esc 返回工具栏。"
          onContextMenu={(event) => event.preventDefault()}
          onPointerMove={(event) => queue({ type: "move", ...toRemote(event.currentTarget, event.clientX, event.clientY) })}
          onPointerDown={(event) => button(event, true)}
          onPointerUp={(event) => button(event, false)}
          onPointerCancel={() => queue({ type: "releaseAll" })}
          onKeyDown={(event) => keyboard(event, true)}
          onKeyUp={(event) => keyboard(event, false)}
          onBlur={() => queue({ type: "releaseAll" })}
        />
      </div>
      <footer className="session-footer">
        <form className="send-text" onSubmit={sendText}>
          <input aria-label="发送到远端的文字" placeholder="发送文字到远端当前输入框" value={text}
            onChange={(event) => setText(event.target.value)} maxLength={64} disabled={!size} autoComplete="off" />
          <button type="submit" className="secondary" disabled={!size || !text}>发送</button>
        </form>
        <span role="status">{capture || "Ctrl+Alt+Esc 返回工具栏"}</span>
      </footer>
    </main>
  );
}
