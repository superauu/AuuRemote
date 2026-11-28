import { getCurrentWindow } from "@tauri-apps/api/window";
import { isTauri } from "@tauri-apps/api/core";
import { Copy, KeyRound, Languages, LockKeyhole, LockOpen, Minus, Monitor, Moon, PanelRight, Plus, Square, Sun, X } from "lucide-react";
import { type KeyboardEvent, type MouseEvent, useEffect, useLayoutEffect, useRef, useState, type Ref } from "react";
import appLogo from "../assets/auu-remote.png";
import { setLanguage, useI18n } from "../i18n";
import { setTheme, type ThemeMode, useTheme } from "../theme";
import type { Tab, VaultStatus } from "../types";

const THEME_ORDER: ThemeMode[] = ["dark", "light", "system"];

/** Mousedown targets that must never start a window drag. */
const INTERACTIVE = 'button, a, input, textarea, select, [role="button"], [data-no-window-drag]';

function ThemeIcon({ mode }: { mode: ThemeMode }) {
  if (mode === "light") return <Sun size={14} aria-hidden="true" />;
  if (mode === "system") return <Monitor size={14} aria-hidden="true" />;
  return <Moon size={14} aria-hidden="true" />;
}

function WindowButton({ label, onClick, close, children }: {
  label: string;
  onClick: () => void;
  close?: boolean;
  children: React.ReactNode;
}) {
  return (
    <button type="button" className={`window-btn${close ? " close" : ""}`} title={label} aria-label={label} onClick={onClick}>
      {children}
    </button>
  );
}

export function Toolbar({ tabs, activeTabId, onSelect, onCloseTab, onTabContextMenu, onNew, vault, onOpenVault, configOpen, configToggle, onToggleConfig }: {
  tabs: Tab[];
  activeTabId?: number;
  onSelect: (id: number) => void;
  onCloseTab: (id: number) => void;
  onTabContextMenu: (tab: Tab, x: number, y: number) => void;
  onNew: () => void;
  vault?: VaultStatus;
  onOpenVault: () => void;
  configOpen: boolean;
  configToggle: Ref<HTMLButtonElement>;
  onToggleConfig: () => void;
}) {
  const { lang, t } = useI18n();
  const { theme } = useTheme();
  const nextTheme = THEME_ORDER[(THEME_ORDER.indexOf(theme) + 1) % THEME_ORDER.length];
  const VaultIcon = !vault?.setup ? KeyRound : vault.unlocked ? LockOpen : LockKeyhole;
  const vaultLabel = !vault?.setup ? t("toolbar.vault.setup")
    : vault.unlocked ? t("toolbar.vault.unlocked") : t("toolbar.vault.locked");
  // Frameless window: this header doubles as the title bar (drag + double-click
  // maximize, auugit style), so it hosts the window controls too.
  const [maximized, setMaximized] = useState(false);
  const tabList = useRef<HTMLDivElement>(null);
  const focusAfterClose = useRef(false);

  useEffect(() => {
    if (!isTauri()) return;
    const win = getCurrentWindow();
    let unlisten: (() => void) | undefined;
    void win.onResized(() => {
      void win.isMaximized().then(setMaximized, () => {});
    }).then((stop) => { unlisten = stop; }, () => {});
    return () => unlisten?.();
  }, []);

  useLayoutEffect(() => {
    if (!focusAfterClose.current) return;
    focusAfterClose.current = false;
    tabList.current?.querySelector<HTMLButtonElement>(`#session-tab-${activeTabId}`)?.focus();
  }, [activeTabId, tabs]);

  function focusTab(id: number | undefined) {
    tabList.current?.querySelector<HTMLButtonElement>(`#session-tab-${id}`)?.focus();
  }

  function navigate(event: KeyboardEvent<HTMLButtonElement>, index: number) {
    let next: number;
    switch (event.key) {
      case "ArrowLeft": next = (index + tabs.length - 1) % tabs.length; break;
      case "ArrowRight": next = (index + 1) % tabs.length; break;
      case "Home": next = 0; break;
      case "End": next = tabs.length - 1; break;
      default: return;
    }
    event.preventDefault();
    onSelect(tabs[next].id);
    focusTab(tabs[next].id);
  }

  function dragTitlebar(event: MouseEvent<HTMLElement>) {
    if (!isTauri() || event.button !== 0 || (event.target as Element).closest(INTERACTIVE)) return;
    if (event.detail === 2) {
      void getCurrentWindow().toggleMaximize();
      return;
    }
    void getCurrentWindow().startDragging().catch(() => {});
  }

  function closeTab(event: MouseEvent<HTMLButtonElement>, id: number) {
    event.stopPropagation();
    focusAfterClose.current = true;
    onCloseTab(id);
  }

  return (
    <header onMouseDown={dragTitlebar} className="app-toolbar flex h-10 shrink-0 items-center gap-1 ps-2 select-none">
      <div className="flex h-full min-w-0 flex-1 items-center gap-1 overflow-hidden">
        <img src={appLogo} alt="" aria-hidden="true" draggable={false} className="h-5 w-5 shrink-0" />
        <span className="brand-mark shrink-0">AuuRemote</span>
        <span className="toolbar-divider mx-1.5 shrink-0" aria-hidden="true" />
        {tabs.length > 0 && (
          <div className="titlebar-tabs" ref={tabList} role="tablist" aria-label={t("tabs.title")}>
            {tabs.map((tab, index) => (
              <div
                key={tab.id}
                className="titlebar-tab group"
                data-active={tab.id === activeTabId}
                aria-current={tab.id === activeTabId ? "page" : undefined}
                onContextMenu={(event) => onTabContextMenu(tab, event.clientX, event.clientY)}
              >
                <button
                  type="button"
                  id={`session-tab-${tab.id}`}
                  role="tab"
                  aria-selected={tab.id === activeTabId}
                  aria-controls={`session-panel-${tab.id}`}
                  tabIndex={tab.id === activeTabId ? 0 : -1}
                  className="flex min-w-0 items-center gap-2"
                  title={tab.target}
                  onClick={() => onSelect(tab.id)}
                  onKeyDown={(event) => navigate(event, index)}
                >
                  <Monitor size={14} aria-hidden="true" className="shrink-0" />
                  <span className="max-w-[160px] truncate">{tab.label}</span>
                  <span className="titlebar-tab-branch font-mono">{tab.target}</span>
                </button>
                <button
                  type="button"
                  className="tab-close"
                  aria-label={`${t("tabs.close")} ${tab.label}`}
                  tabIndex={tab.id === activeTabId ? 0 : -1}
                  onClick={(event) => closeTab(event, tab.id)}
                >
                  <X size={13} aria-hidden="true" />
                </button>
              </div>
            ))}
          </div>
        )}
        <button
          type="button"
          className="tab-new ms-0.5"
          title={t("toolbar.newConnection")}
          aria-label={t("toolbar.newConnection")}
          onClick={onNew}
        >
          <Plus size={15} aria-hidden="true" />
        </button>
      </div>
      <div className="flex shrink-0 items-center gap-1 pe-2">
        <button
          type="button"
          className="icon-btn"
          title={vaultLabel}
          aria-label={vaultLabel}
          onClick={onOpenVault}
        >
          <VaultIcon size={14} aria-hidden="true" />
        </button>
        <button
          type="button"
          className="icon-btn !w-auto gap-1 px-1.5 text-xs font-semibold"
          title={t("toolbar.language")}
          aria-label={t("toolbar.language")}
          onClick={() => setLanguage(lang === "zh" ? "en" : "zh")}
        >
          <Languages size={14} aria-hidden="true" />
          {lang === "zh" ? "EN" : "中"}
        </button>
        <button
          type="button"
          className="icon-btn"
          title={t(`toolbar.theme.${nextTheme}`)}
          aria-label={t(`toolbar.theme.${nextTheme}`)}
          onClick={() => setTheme(nextTheme)}
        >
          <ThemeIcon mode={theme} />
        </button>
        <button
          ref={configToggle}
          type="button"
          className="icon-btn"
          title={t(configOpen ? "config.hide" : "config.show")}
          aria-label={t(configOpen ? "config.hide" : "config.show")}
          aria-expanded={configOpen}
          aria-controls="connection-config"
          onClick={onToggleConfig}
        >
          <PanelRight size={16} aria-hidden="true" />
        </button>
      </div>
      {isTauri() && (
        <div className="window-controls" role="group" aria-label={t("toolbar.windowControls")}>
          <WindowButton label={t("toolbar.minimize")} onClick={() => void getCurrentWindow().minimize()}>
            <Minus size={14} aria-hidden="true" />
          </WindowButton>
          <WindowButton label={t(maximized ? "toolbar.restore" : "toolbar.maximize")} onClick={() => void getCurrentWindow().toggleMaximize()}>
            {maximized ? <Copy size={12} aria-hidden="true" /> : <Square size={12} aria-hidden="true" />}
          </WindowButton>
          <WindowButton label={t("toolbar.close")} close onClick={() => void getCurrentWindow().close()}>
            <X size={15} aria-hidden="true" />
          </WindowButton>
        </div>
      )}
    </header>
  );
}
