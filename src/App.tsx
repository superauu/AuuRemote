import { invoke, isTauri } from "@tauri-apps/api/core";
import { type FormEvent, useEffect, useRef, useState } from "react";
import { ConnectionPanel } from "./components/ConnectionPanel";
import { ConfigPanel } from "./components/ConfigPanel";
import { MasterPasswordDialog, type VaultDialogMode } from "./components/MasterPasswordDialog";
import { RemoteCanvas } from "./components/RemoteCanvas";
import { StatusBar } from "./components/StatusBar";
import { TabStrip } from "./components/TabStrip";
import { Toolbar } from "./components/Toolbar";
import { asApiError, describeError } from "./errors";
import { type TranslationKey, useI18n } from "./i18n";
import { blankDraft, targetAddress, type BusyState, type Draft, type Notice, type Profile, type Tab, type VaultStatus } from "./types";

const desktop = isTauri();

type ConnectTarget = { host: string; port: number; username: string; fingerprint: string };

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
  const pendingSave = useRef<{ profileId: number; password: string } | undefined>(undefined);

  const selected = profiles.find((profile) => profile.id === selectedId);
  const validDetails = !!draft.host.trim() && !!draft.username.trim() &&
    /^[0-9]+$/.test(draft.port) && Number(draft.port) > 0 && Number(draft.port) <= 65535;
  const openProfileIds = new Set(tabs.map((tab) => tab.profileId).filter((id) => id !== undefined));

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
    setSelectedId(profile?.id);
    setDraft(profile ? {
      name: profile.name, host: profile.host, port: String(profile.port),
      username: profile.username, fingerprint: profile.fingerprint,
    } : blankDraft);
    setNotice(undefined);
    const saved = hasSavedPassword(profile?.id);
    setRemember(saved);
    setPassword("");
    if (profile && saved && vault?.unlocked) {
      void invoke<string | null>("vault_get_password", { profileId: profile.id })
        .then((pw) => { if (pw) setPassword(pw); }, () => {});
    }
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
    };
  }

  async function startSession(target: ConnectTarget, password: string, name: string, profileId: number | undefined, replaceTabId?: number): Promise<boolean> {
    try {
      const id = await invoke<number>("rdp_connect", {
        connection: { host: target.host, port: target.port, username: target.username, fingerprint: target.fingerprint, password },
      });
      const tab: Tab = { id, profileId, label: name || targetAddress(target.host, target.port), target: targetAddress(target.host, target.port) };
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
    setActiveTabId((current) => current === id ? remaining.at(-1)?.id : current);
  }

  async function reconnect(tab: Tab) {
    const profile = profiles.find((item) => item.id === tab.profileId);
    void invoke("rdp_disconnect", { id: tab.id }).catch(() => {});
    if (profile && vault?.unlocked && vault.profileIds.includes(profile.id)) {
      try {
        const pw = await invoke<string | null>("vault_get_password", { profileId: profile.id });
        if (pw) {
          const ok = await startSession(
            { host: profile.host, port: profile.port, username: profile.username, fingerprint: profile.fingerprint },
            pw, profile.name, profile.id, tab.id,
          );
          if (ok) return;
        }
      } catch {
        // Fall through to the manual path.
      }
    }
    selectProfile(profile);
    setNotice({ kind: "info", titleKey: "notice.reenterPassword" });
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
    } catch (error) {
      setVaultError(error);
    } finally {
      setVaultPending(false);
    }
  }

  return (
    <div className="flex h-dvh flex-col overflow-hidden">
      <Toolbar vault={vault} onNewConnection={() => selectProfile()} onOpenVault={openVault} />
      <div className="flex min-h-0 flex-1">
        <ConnectionPanel
          profiles={profiles}
          selectedId={selectedId}
          openProfileIds={openProfileIds}
          query={query}
          onQuery={setQuery}
          onSelect={selectProfile}
          onNew={() => selectProfile()}
        />
        <div className="flex min-w-0 flex-1 flex-col">
          <TabStrip tabs={tabs} activeTabId={activeTabId} onSelect={setActiveTabId} onClose={closeTab} />
          <div className="relative min-h-0 flex-1">
            {tabs.map((tab) => (
              <div key={tab.id} className={`absolute inset-0 ${tab.id === activeTabId ? "" : "hidden"}`}>
                <RemoteCanvas tab={tab} active={tab.id === activeTabId} onDisconnect={closeTab} onReconnect={reconnect} />
              </div>
            ))}
            {tabs.length === 0 && (
              <div className="absolute inset-0 grid place-items-center">
                <p className="max-w-[300px] text-center text-xs text-muted-foreground">{t("tabs.noTabs")}</p>
              </div>
            )}
          </div>
        </div>
        <ConfigPanel
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
        />
      </div>
      <StatusBar sessionCount={tabs.length} vault={vault} />
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
