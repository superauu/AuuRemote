import { invoke, isTauri } from "@tauri-apps/api/core";
import { ArrowRight, MonitorUp } from "lucide-react";
import { type FormEvent, useCallback, useEffect, useRef, useState } from "react";
import { ConnectionPanel } from "./components/ConnectionPanel";
import { ConfigPanel } from "./components/ConfigPanel";
import { MasterPasswordDialog, type VaultDialogMode } from "./components/MasterPasswordDialog";
import { PanelResizeHandle } from "./components/PanelResizeHandle";
import { RemoteCanvas } from "./components/RemoteCanvas";
import { RenameDialog } from "./components/RenameDialog";
import { StatusBar } from "./components/StatusBar";
import { TabMenu, type TabMenuAction, type TabMenuState } from "./components/TabMenu";
import { Toolbar } from "./components/Toolbar";
import { asApiError, describeError } from "./errors";
import { type TranslationKey, useI18n } from "./i18n";
import { usePanelWidth } from "./usePanelWidth";
import { blankDraft, resolveSize, targetAddress, targetHost, type BusyState, type Draft, type Notice, type Profile, type Tab, type VaultStatus } from "./types";

const desktop = isTauri();

type ConnectTarget = { host: string; port: number; username: string; fingerprint: string; resolution: string };

const CONNECTIONS_PANEL = { key: "auu_remote_panel_connections", fallback: 224, min: 190, max: 420 } as const;
const CONFIG_PANEL = { key: "auu_remote_panel_config", fallback: 320, min: 280, max: 520 } as const;
const MIN_WORKSPACE_WIDTH = 400;

export function App() {
  const { t } = useI18n();
  const [profiles, setProfiles] = useState<Profile[]>([]);
  const [selectedId, setSelectedId] = useState<number>();
  const [query, setQuery] = useState("");
  const [draft, setDraft] = useState<Draft>(blankDraft);
  const [password, setPassword] = useState("");
  const [remember, setRemember] = useState(false);
  const [tabs, setTabs] = useState<Tab[]>([]);
  const [activeTabId, setActiveTabId] = useState<number>();
  const [vault, setVault] = useState<VaultStatus>();
  const [vaultDialog, setVaultDialog] = useState<VaultDialogMode>();
  const [vaultPending, setVaultPending] = useState(false);
  const [vaultError, setVaultError] = useState<unknown>();
  const [busy, setBusy] = useState<BusyState>();
  const [notice, setNotice] = useState<Notice>();
  const [configOpen, setConfigOpen] = useState(true);
  const [tabMenu, setTabMenu] = useState<TabMenuState>();
  const [rename, setRename] = useState<{ tabId: number; value: string }>();
  const [fullscreen, setFullscreen] = useState(false);
  const [sessionSizes, setSessionSizes] = useState<Record<number, { x: number; y: number } | undefined>>({});
  const [viewportWidth, setViewportWidth] = useState(window.innerWidth);
  const configToggle = useRef<HTMLButtonElement>(null);
  const hostField = useRef<HTMLInputElement>(null);
  const focusHostOnOpen = useRef(false);
  const pendingSave = useRef<{ profileId: number; password: string } | undefined>(undefined);
  const [connectionsWidth, setConnectionsWidth] = usePanelWidth(
    CONNECTIONS_PANEL.key, CONNECTIONS_PANEL.fallback, CONNECTIONS_PANEL.min, CONNECTIONS_PANEL.max,
  );
  const [configWidth, setConfigWidth] = usePanelWidth(
    CONFIG_PANEL.key, CONFIG_PANEL.fallback, CONFIG_PANEL.min, CONFIG_PANEL.max,
  );

  const selected = profiles.find((profile) => profile.id === selectedId);
  const validDetails = !!draft.host.trim() && !!draft.username.trim() &&
    /^[0-9]+$/.test(draft.port) && Number(draft.port) > 0 && Number(draft.port) <= 65535;
  const openProfileIds = new Set(tabs.map((tab) => tab.profileId).filter((id) => id !== undefined));
  const menuProfile = tabMenu && profiles.find((item) => item.id === tabMenu.tab.profileId);
  const panelBudget = viewportWidth - MIN_WORKSPACE_WIDTH;
  const visibleConfigWidth = Math.min(configWidth, Math.max(CONFIG_PANEL.min, panelBudget - CONNECTIONS_PANEL.min));
  const maxConnectionsWidth = Math.min(CONNECTIONS_PANEL.max, Math.max(
    CONNECTIONS_PANEL.min, panelBudget - (configOpen ? visibleConfigWidth : 0),
  ));
  const visibleConnectionsWidth = Math.min(connectionsWidth, maxConnectionsWidth);
  const maxConfigWidth = Math.min(CONFIG_PANEL.max, Math.max(CONFIG_PANEL.min, panelBudget - visibleConnectionsWidth));

  useEffect(() => {
    const update = () => setViewportWidth(window.innerWidth);
    window.addEventListener("resize", update);
    return () => window.removeEventListener("resize", update);
  }, []);

  // Fullscreen owns F11: the key never reaches the remote desktop, so the
  // toggle works from anywhere in the window.
  useEffect(() => {
    function onKey(event: KeyboardEvent) {
      if (event.key === "F11") {
        event.preventDefault();
        setFullscreen((current) => !current);
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  useEffect(() => {
    if (!desktop) return;
    if (fullscreen) setConfigOpen(false);
    void invoke("set_fullscreen", { fullscreen }).catch(() => {});
  }, [fullscreen]);

  useEffect(() => {
    if (configOpen && focusHostOnOpen.current) {
      hostField.current?.focus();
      focusHostOnOpen.current = false;
    }
  }, [configOpen]);

  useEffect(() => {
    if (notice?.kind === "error") setConfigOpen(true);
  }, [notice]);

  function failNotice(titleKey: TranslationKey, error: unknown): Notice {
    const api = asApiError(error);
    console.debug("backend error", error);
    return { kind: "error", titleKey, message: describeError(error, t), raw: api?.detail, code: api?.code };
  }

  async function refreshVault() {
    try {
      setVault(await invoke<VaultStatus>("vault_status"));
    } catch (error) {
      console.debug("vault status unavailable", error);
    }
  }

  useEffect(() => {
    if (!desktop) return;
    void invoke<Profile[]>("list_profiles").then((items) => setProfiles(items), (error) => {
      console.debug("backend error", error);
      setNotice({ kind: "error", titleKey: "notice.loadFailed", message: describeError(error, t) });
    });
    void refreshVault();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  /** A profile's password is saved when the vault lists it. */
  function hasSavedPassword(profileId: number | undefined): boolean {
    return profileId !== undefined && (vault?.profileIds.includes(profileId) ?? false);
  }

  function selectProfile(profile?: Profile) {
    setConfigOpen(true);
    setSelectedId(profile?.id);
    setDraft(profile ? {
      name: profile.name, host: profile.host, port: String(profile.port),
      username: profile.username, fingerprint: profile.fingerprint,
      resolution: profile.resolution,
    } : blankDraft);
    setNotice(undefined);
    const saved = hasSavedPassword(profile?.id);
    setRemember(saved);
    setPassword("");
    if (profile && saved && vault?.unlocked) {
      void invoke<string | null>("vault_get_password", { profileId: profile.id })
        .then((pw) => { if (pw) setPassword(pw); }, () => {});
    } else if (profile && saved && vault?.setup && !vault.unlocked) {
      // The vault is locked: prompt for the master password so the remembered
      // password can be filled in instead of looking lost.
      setVaultError(undefined);
      setVaultDialog("unlock");
    }
  }

  function focusConfig() {
    if (configOpen) {
      hostField.current?.focus();
    } else {
      focusHostOnOpen.current = true;
      setConfigOpen(true);
    }
  }

  function newConnection() {
    selectProfile();
    focusConfig();
  }

  function closeConfig() {
    setConfigOpen(false);
    configToggle.current?.focus();
  }

  function changeDraft<K extends keyof Draft>(key: K, value: Draft[K]) {
    const endpointChanged = key === "host" || key === "port";
    setDraft((current) => ({
      ...current,
      ...(endpointChanged ? { fingerprint: "" } : {}),
      [key]: value,
    }));
    if (endpointChanged || key === "username") {
      setPassword("");
      setRemember(false);
    }
  }

  function currentDetails() {
    const host = draft.host.trim();
    const port = Number(draft.port);
    return {
      name: draft.name.trim() || targetAddress(host, port).slice(0, 80),
      host, port,
      username: draft.username,
      fingerprint: draft.fingerprint,
      resolution: draft.resolution,
    };
  }

  async function startSession(target: ConnectTarget, password: string, name: string, profileId: number | undefined, replaceTabId?: number): Promise<boolean> {
    try {
      const size = resolveSize(target.resolution);
      const id = await invoke<number>("rdp_connect", {
        connection: {
          host: target.host, port: target.port, username: target.username,
          fingerprint: target.fingerprint, password,
          width: size.width, height: size.height,
        },
      });
      const tab: Tab = { id, profileId, label: name || targetAddress(target.host, target.port), target: targetAddress(target.host, target.port), resolution: target.resolution };
      setTabs((current) => [...current.filter((item) => item.id !== replaceTabId), tab]);
      setActiveTabId(id);
      setNotice(undefined);
      return true;
    } catch (error) {
      setNotice(failNotice("notice.connectFailed", error));
      return false;
    }
  }

  async function savePasswordFor(profileId: number, pw: string) {
    if (!vault?.setup) {
      pendingSave.current = { profileId, password: pw };
      setVaultError(undefined);
      setVaultDialog("create");
      return;
    }
    if (!vault.unlocked) {
      pendingSave.current = { profileId, password: pw };
      setVaultError(undefined);
      setVaultDialog("unlock");
      return;
    }
    try {
      setVault(await invoke<VaultStatus>("vault_set_password", { profileId, password: pw }));
    } catch (error) {
      setNotice(failNotice("notice.vaultSaveFailed", error));
    }
  }

  async function connect(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!desktop || busy || !validDetails || !password) return;
    setBusy("connecting");
    const details = currentDetails();
    const ok = await startSession(details, password, details.name, selectedId);
    if (ok) {
      setConfigOpen(false);
    }
    if (ok && selectedId !== undefined) {
      if (remember) {
        await savePasswordFor(selectedId, password);
      } else if (hasSavedPassword(selectedId)) {
        void invoke<VaultStatus>("vault_remove_password", { profileId: selectedId })
          .then(setVault, () => {});
      }
    }
    setPassword("");
    setBusy(undefined);
  }

  async function save() {
    if (!desktop || busy || !validDetails) return;
    setBusy("saving");
    try {
      const result = await invoke<{ id: number; profiles: Profile[] }>("save_profile", {
        id: selectedId ?? null, details: currentDetails(),
      });
      setProfiles(result.profiles);
      setSelectedId(result.id);
      setConfigOpen(false);
      setNotice({ kind: "info", titleKey: "notice.saved" });
    } catch (error) {
      setNotice(failNotice("notice.saveFailed", error));
    } finally {
      setBusy(undefined);
    }
  }

  async function remove() {
    if (!desktop || busy || selectedId === undefined) return;
    setBusy("removing");
    try {
      setProfiles(await invoke<Profile[]>("remove_profile", { id: selectedId }));
      if (hasSavedPassword(selectedId)) {
        void invoke<VaultStatus>("vault_remove_password", { profileId: selectedId })
          .then(setVault, () => {});
      }
      selectProfile();
      setNotice({ kind: "info", titleKey: "notice.removed" });
    } catch (error) {
      setNotice(failNotice("notice.removeFailed", error));
    } finally {
      setBusy(undefined);
    }
  }

  function closeTab(id: number, hadFrame = false) {
    void invoke("rdp_disconnect", { id }).then(() => {
      if (hadFrame) setNotice({ kind: "info", titleKey: "notice.disconnected" });
    }, () => {});
    const remaining = tabs.filter((tab) => tab.id !== id);
    setTabs(remaining);
    setSessionSizes((current) => {
      const next = { ...current };
      delete next[id];
      return next;
    });
    setActiveTabId((current) => current === id ? remaining.at(-1)?.id : current);
  }

  /** Connects using a profile's saved password when the vault allows it. */
  async function startFromProfile(profile: Profile, replaceTabId?: number): Promise<boolean> {
    if (!vault?.unlocked || !vault.profileIds.includes(profile.id)) return false;
    try {
      const pw = await invoke<string | null>("vault_get_password", { profileId: profile.id });
      if (!pw) return false;
      return await startSession(
        { host: profile.host, port: profile.port, username: profile.username, fingerprint: profile.fingerprint, resolution: profile.resolution },
        pw, profile.name, profile.id, replaceTabId,
      );
    } catch {
      // Fall through to the manual path.
      return false;
    }
  }

  async function reconnect(tab: Tab) {
    void invoke("rdp_disconnect", { id: tab.id }).catch(() => {});
    const profile = profiles.find((item) => item.id === tab.profileId);
    if (profile && (await startFromProfile(profile, tab.id))) return;
    selectProfile(profile);
    setNotice({ kind: "info", titleKey: "notice.reenterPassword" });
  }

  /** Opens a second session for the same connection. */
  async function duplicateTab(tab: Tab) {
    const profile = profiles.find((item) => item.id === tab.profileId);
    if (!profile) return;
    if (await startFromProfile(profile)) return;
    selectProfile(profile);
    setNotice({ kind: "info", titleKey: "notice.reenterPassword" });
  }

  /** Receives live framebuffer sizes from each session for the status bar. */
  const handleSessionSize = useCallback((id: number, size: { x: number; y: number } | undefined) => {
    setSessionSizes((current) => (current[id] === size ? current : { ...current, [id]: size }));
  }, []);

  function toggleSmartSize(tab: Tab) {
    if (tab.resolution === "auto") {
      // Turning Smart Size off needs a fixed resolution to restore.
      const fixed = profiles.find((item) => item.id === tab.profileId)?.resolution;
      if (!fixed || fixed === "auto") return;
      setTabs((current) => current.map((item) => (item.id === tab.id ? { ...item, resolution: fixed } : item)));
    } else {
      setTabs((current) => current.map((item) => (item.id === tab.id ? { ...item, resolution: "auto" } : item)));
    }
  }

  async function screenshotTab(id: number) {
    try {
      if (await invoke<boolean>("rdp_screenshot", { id })) {
        setNotice({ kind: "info", titleKey: "session.screenshotSaved" });
      }
    } catch (error) {
      setNotice(failNotice("notice.screenshotFailed", error));
    }
  }

  function submitRename(value: string) {
    if (!rename) return;
    const label = value.trim().slice(0, 80);
    if (label) {
      setTabs((current) => current.map((item) => (item.id === rename.tabId ? { ...item, label } : item)));
    }
    setRename(undefined);
  }

  function runTabAction(action: TabMenuAction, tab: Tab) {
    switch (action) {
      case "fullscreen": setFullscreen((current) => !current); break;
      case "smartSize": toggleSmartSize(tab); break;
      case "screenshot": void screenshotTab(tab.id); break;
      case "rename": setRename({ tabId: tab.id, value: tab.label }); break;
      case "duplicate": void duplicateTab(tab); break;
      case "reconnect": void reconnect(tab); break;
      case "disconnect": closeTab(tab.id, true); break;
    }
  }

  /** TOFU: remember the presented certificate, then retry the connection. */
  async function trustCertificate(tab: Tab, fingerprint: string) {
    try {
      await invoke("trust_host", { host: targetHost(tab.target), fingerprint });
      await reconnect(tab);
    } catch (error) {
      setNotice(failNotice("notice.trustFailed", error));
    }
  }

  function openVault() {
    setVaultError(undefined);
    setVaultDialog(!vault?.setup ? "create" : vault.unlocked ? "change" : "unlock");
  }

  function closeVaultDialog() {
    setVaultDialog(undefined);
    pendingSave.current = undefined;
  }

  async function submitVault({ oldPassword, newPassword }: { oldPassword?: string; newPassword: string }) {
    setVaultPending(true);
    setVaultError(undefined);
    const mode = vaultDialog;
    try {
      const status = mode === "create"
        ? await invoke<VaultStatus>("vault_create", { password: newPassword })
        : mode === "unlock"
          ? await invoke<VaultStatus>("vault_unlock", { password: newPassword })
          : await invoke<VaultStatus>("vault_change_password", { oldPassword, newPassword });
      setVault(status);
      setVaultDialog(undefined);
      const saved = pendingSave.current;
      if (saved && mode !== "change") {
        pendingSave.current = undefined;
        try {
          setVault(await invoke<VaultStatus>("vault_set_password", { profileId: saved.profileId, password: saved.password }));
        } catch (error) {
          setNotice(failNotice("notice.vaultSaveFailed", error));
        }
      }
      // Unlocking fills the selected connection's remembered password, so a
      // locked vault never looks like a lost password.
      if (mode === "unlock" && hasSavedPassword(selectedId)) {
        try {
          const pw = await invoke<string | null>("vault_get_password", { profileId: selectedId });
          if (pw) setPassword(pw);
        } catch {
          // The user can still type the password manually.
        }
      }
    } catch (error) {
      setVaultError(error);
    } finally {
      setVaultPending(false);
    }
  }

  return (
    <div className="flex h-dvh flex-col overflow-hidden">
      <Toolbar
        tabs={tabs}
        activeTabId={activeTabId}
        onSelect={setActiveTabId}
        onCloseTab={closeTab}
        onTabContextMenu={(tab, x, y) => setTabMenu({ tab, x, y })}
        onNew={newConnection}
        vault={vault}
        onOpenVault={openVault}
        configOpen={configOpen}
        configToggle={configToggle}
        onToggleConfig={() => setConfigOpen((open) => !open)}
      />
      <div className="flex min-h-0 flex-1">
        {!fullscreen && (
          <>
            <ConnectionPanel
              profiles={profiles}
              selectedId={selectedId}
              openProfileIds={openProfileIds}
              query={query}
              onQuery={setQuery}
              onSelect={selectProfile}
              onNew={newConnection}
              width={visibleConnectionsWidth}
            />
            <PanelResizeHandle
              label={t("panel.resize.connections")}
              value={visibleConnectionsWidth}
              min={CONNECTIONS_PANEL.min}
              max={maxConnectionsWidth}
              defaultValue={CONNECTIONS_PANEL.fallback}
              side="left"
              onChange={setConnectionsWidth}
            />
          </>
        )}
        <main className="workspace flex min-w-0 flex-1 flex-col">
          <div className="relative min-h-0 flex-1">
            {tabs.map((tab) => (
              <div
                key={tab.id}
                id={`session-panel-${tab.id}`}
                role="tabpanel"
                aria-labelledby={`session-tab-${tab.id}`}
                hidden={tab.id !== activeTabId}
                className="absolute inset-0"
              >
                <RemoteCanvas tab={tab} active={tab.id === activeTabId} autoResize={tab.resolution === "auto"} onSessionSize={handleSessionSize} onDisconnect={closeTab} onReconnect={reconnect} onTrustCertificate={(tab, fingerprint) => void trustCertificate(tab, fingerprint)} />
              </div>
            ))}
            {tabs.length === 0 && (
              <div className="absolute inset-0 grid place-items-center overflow-auto p-6">
                <div className="empty-workspace">
                  <div className="empty-illustration" aria-hidden="true">
                    <div className="empty-window">
                      <span className="empty-window-bar"><i /><i /><i /></span>
                      <div className="empty-window-content"><MonitorUp size={30} strokeWidth={1.5} /></div>
                    </div>
                  </div>
                  <h1 className="text-xl font-semibold tracking-[-0.025em]">{t("empty.title")}</h1>
                  <p className="mt-2 max-w-[300px] text-[13px] leading-5 text-muted-foreground">{t("empty.hint")}</p>
                  <button type="button" className="btn-primary mt-5" onClick={focusConfig}>
                    {t("config.focus")}
                    <ArrowRight size={14} aria-hidden="true" />
                  </button>
                </div>
              </div>
            )}
          </div>
        </main>
        {configOpen && (
          <PanelResizeHandle
            label={t("panel.resize.config")}
            value={visibleConfigWidth}
            min={CONFIG_PANEL.min}
            max={maxConfigWidth}
            defaultValue={CONFIG_PANEL.fallback}
            side="right"
            onChange={setConfigWidth}
          />
        )}
        <ConfigPanel
          open={configOpen}
          onClose={closeConfig}
          hostField={hostField}
          draft={draft}
          password={password}
          remember={remember}
          selected={selected}
          busy={busy}
          desktop={desktop}
          notice={notice}
          validDetails={validDetails}
          onDraftChange={changeDraft}
          onPasswordChange={setPassword}
          onRememberChange={setRemember}
          onConnect={(event) => void connect(event)}
          onSave={() => void save()}
          onRemove={() => void remove()}
          width={visibleConfigWidth}
        />
      </div>
      <StatusBar
        activeTab={tabs.find((tab) => tab.id === activeTabId)}
        sessionSize={activeTabId !== undefined ? sessionSizes[activeTabId] : undefined}
        sessionCount={tabs.length}
        vault={vault}
      />
      {tabMenu && (
        <TabMenu
          menu={tabMenu}
          fullscreen={fullscreen}
          smartSize={tabMenu.tab.resolution === "auto"}
          smartSizeLocked={!menuProfile || menuProfile.resolution === "auto"}
          onClose={() => setTabMenu(undefined)}
          onAction={runTabAction}
        />
      )}
      {rename && (
        <RenameDialog
          initial={rename.value}
          onSubmit={submitRename}
          onClose={() => setRename(undefined)}
        />
      )}
      {vaultDialog && (
        <MasterPasswordDialog
          mode={vaultDialog}
          pending={vaultPending}
          error={vaultError}
          onSubmit={(values) => void submitVault(values)}
          onClose={closeVaultDialog}
        />
      )}
    </div>
  );
}
