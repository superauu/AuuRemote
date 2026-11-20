import { invoke, isTauri } from "@tauri-apps/api/core";
import { type FormEvent, useCallback, useEffect, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { RemoteDesktop } from "./RemoteDesktop";
import "./styles.css";

type Details = {
  name: string;
  host: string;
  port: number;
  username: string;
  fingerprint: string;
};
type Profile = Details & { id: number };
type Draft = Omit<Details, "port"> & { port: string };
type Notice = { title: string; message?: string; error?: boolean };
type Session = { id: number; target: string };

const blank: Draft = { name: "", host: "", port: "3389", username: "", fingerprint: "" };
const desktop = isTauri();

function targetAddress(host: string, port: string | number) {
  return `${host.includes(":") && !host.startsWith("[") ? `[${host}]` : host}:${port}`;
}

function App() {
  const [profiles, setProfiles] = useState<Profile[]>([]);
  const [selected, setSelected] = useState<number>();
  const [query, setQuery] = useState("");
  const [draft, setDraft] = useState<Draft>(blank);
  const [password, setPassword] = useState("");
  const [session, setSession] = useState<Session>();
  const [busy, setBusy] = useState<"loading" | "connecting" | "saving" | "removing">();
  const [notice, setNotice] = useState<Notice>();
  const [reconnect, setReconnect] = useState(false);
  const passwordField = useRef<HTMLInputElement>(null);
  const validDetails = !!draft.host.trim() && !!draft.username.trim() &&
    /^[0-9]+$/.test(draft.port) && Number(draft.port) > 0 && Number(draft.port) <= 65535;

  useEffect(() => {
    if (!desktop) return;
    let current = true;
    setBusy("loading");
    void invoke<Profile[]>("list_profiles").then((items) => {
      if (current) setProfiles(items);
    }, (error) => {
      if (current) setNotice({ title: "无法读取已保存的连接", message: String(error), error: true });
    }).finally(() => {
      if (current) setBusy(undefined);
    });
    return () => { current = false; };
  }, []);

  useEffect(() => {
    if (reconnect && !session) passwordField.current?.focus();
  }, [reconnect, session]);

  const close = useCallback((id: number, hadFrame: boolean, reason?: string) => {
    void invoke("rdp_disconnect", { id }).then(() => {
      setSession((current) => current?.id === id ? undefined : current);
      setReconnect(hadFrame);
      setNotice({
        title: hadFrame ? "连接已断开" : reason ? "连接失败" : "已取消连接",
        message: reason,
        error: !!reason,
      });
    }, (failure) => {
      setSession((current) => current?.id === id ? undefined : current);
      setReconnect(hadFrame);
      setNotice({ title: "无法结束会话", message: String(failure), error: true });
    });
  }, []);

  function selectProfile(profile?: Profile, clearPassword = true) {
    setSelected(profile?.id);
    setDraft(profile ? {
      name: profile.name, host: profile.host, port: String(profile.port),
      username: profile.username, fingerprint: profile.fingerprint,
    } : blank);
    if (clearPassword) setPassword("");
    setNotice(undefined);
    setReconnect(false);
  }

  function change<K extends keyof Draft>(key: K, value: Draft[K]) {
    const endpointChanged = key === "host" || key === "port";
    setDraft((current) => ({
      ...current,
      ...(endpointChanged ? { fingerprint: "" } : {}),
      [key]: value,
    }));
    if (endpointChanged || key === "username") {
      setPassword("");
      setReconnect(false);
    }
  }

  function currentDetails(): Details {
    const host = draft.host.trim();
    const port = Number(draft.port);
    return { ...draft, name: draft.name.trim() || targetAddress(host, port).slice(0, 80), host, port };
  }

  async function connect(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!desktop || busy || !validDetails || !password) return;
    setBusy("connecting");
    setNotice(undefined);
    const { host, port, username, fingerprint } = currentDetails();
    try {
      const id = await invoke<number>("rdp_connect", { connection: { host, port, username, fingerprint, password } });
      setSession({ id, target: targetAddress(host, port) });
      setReconnect(false);
    } catch (error) {
      setNotice({ title: "无法连接", message: String(error), error: true });
    } finally {
      setPassword("");
      setBusy(undefined);
    }
  }

  async function save() {
    if (!desktop || busy || !validDetails) return;
    setBusy("saving");
    try {
      const result = await invoke<{ id: number; profiles: Profile[] }>("save_profile", {
        id: selected ?? null, details: currentDetails(),
      });
      setProfiles(result.profiles);
      selectProfile(result.profiles.find((item) => item.id === result.id), false);
      setNotice({ title: "连接已保存" });
    } catch (error) {
      setNotice({ title: "无法保存连接", message: String(error), error: true });
    } finally {
      setBusy(undefined);
    }
  }

  async function remove() {
    if (!desktop || busy || selected === undefined) return;
    setBusy("removing");
    try {
      setProfiles(await invoke<Profile[]>("remove_profile", { id: selected }));
      selectProfile();
      setNotice({ title: "已移除保存的连接" });
    } catch (error) {
      setNotice({ title: "无法移除连接", message: String(error), error: true });
    } finally {
      setBusy(undefined);
    }
  }

  const needle = query.trim().toLocaleLowerCase();
  const filtered = profiles.filter(({ name, host, username }) =>
    [name, host, username].some((value) => value.toLocaleLowerCase().includes(needle))
  ).sort((left, right) => left.name.localeCompare(right.name));

  return (
    <>
      <header className="titlebar">
        <span className="app-icon" aria-hidden="true"><span /></span>
        <span>AuuRemote</span>
      </header>
      {session ? <RemoteDesktop key={session.id} {...session} close={close} /> :
        <main className="connection-workspace">
          <aside className="profiles" aria-labelledby="profiles-title">
            <div className="profiles-header">
              <h2 id="profiles-title">已保存的连接</h2>
              <button className="secondary" type="button" onClick={() => selectProfile()} disabled={!!busy}>新建</button>
            </div>
            {profiles.length > 0 && <input type="search" aria-label="搜索连接" placeholder="搜索连接"
              value={query} onChange={(event) => setQuery(event.target.value)} />}
            <ul>
              {filtered.map((profile) => <li key={profile.id}>
                <button type="button" className="profile" aria-pressed={selected === profile.id} disabled={!!busy}
                  onClick={() => selectProfile(profile)}>
                  <strong>{profile.name}</strong>
                  <span>{targetAddress(profile.host, profile.port)}</span>
                </button>
              </li>)}
            </ul>
            {filtered.length === 0 && <p className="empty-profiles">{busy === "loading" ? "正在读取连接…" :
              profiles.length ? "没有匹配的连接。" : "保存常用主机后，它会出现在这里。"}</p>}
          </aside>

          <section className="connection" aria-labelledby="connection-title">
            <h1 id="connection-title">{selected === undefined ? "连接远程桌面" : "连接设置"}</h1>
            <form onSubmit={connect}>
              <fieldset disabled={!!busy}>
                <label>连接名称
                  <input value={draft.name} onChange={(event) => change("name", event.target.value)}
                    placeholder="留空时使用主机地址" maxLength={80} autoComplete="off" />
                </label>
                <div className="address">
                  <label>主机名或 IP
                    <input value={draft.host} onChange={(event) => change("host", event.target.value)}
                      placeholder="workstation.example.com" autoComplete="off" spellCheck={false} required />
                  </label>
                  <label>端口
                    <input type="number" min={1} max={65535} value={draft.port}
                      onChange={(event) => change("port", event.target.value)} required />
                  </label>
                </div>
                <label>用户名
                  <input value={draft.username} onChange={(event) => change("username", event.target.value)}
                    placeholder="例如 administrator 或 DOMAIN\user" autoComplete="off" spellCheck={false} required />
                </label>
                <label>密码
                  <input ref={passwordField} type="password" value={password} onChange={(event) => setPassword(event.target.value)}
                    placeholder={reconnect ? "重新输入密码以继续连接" : undefined} autoComplete="off" required />
                </label>
                <details open={notice?.message?.includes("SHA-256") || undefined}>
                  <summary>证书验证</summary>
                  <label>SHA-256 证书指纹（可选）
                    <input value={draft.fingerprint} onChange={(event) => change("fingerprint", event.target.value)}
                      placeholder="填写已核对的自签名证书指纹" autoComplete="off" spellCheck={false} />
                  </label>
                </details>
                <div className="connection-actions">
                  <button type="submit" disabled={!desktop || !validDetails || !password}>
                    {busy === "connecting" ? "正在连接…" : reconnect ? "重新连接" : "连接"}
                  </button>
                  <button type="button" className="secondary" onClick={() => void save()} disabled={!desktop || !validDetails}>
                    {busy === "saving" ? "正在保存…" : "保存连接"}
                  </button>
                  {selected !== undefined && <button type="button" className="secondary remove-profile"
                    onClick={() => void remove()}>移除连接</button>}
                </div>
              </fieldset>
            </form>
            {!desktop && <p className="notice">连接和保存需要 AuuRemote 桌面应用。</p>}
            <div className={notice?.error ? "notice error" : "notice"} role="status">
              {notice && <><strong>{notice.title}</strong>{notice.message && <p>{notice.message}</p>}</>}
            </div>
          </section>
        </main>}
    </>
  );
}

createRoot(document.getElementById("root")!).render(<App />);
