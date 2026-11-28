import { LayoutPanelTop, Monitor, X } from "lucide-react";
import { type KeyboardEvent, type MouseEvent, useLayoutEffect, useRef } from "react";
import { useI18n } from "../i18n";
import type { Tab } from "../types";

export function TabStrip({ tabs, activeTabId, onSelect, onClose, onTabContextMenu }: {
  tabs: Tab[];
  activeTabId?: number;
  onSelect: (id: number) => void;
  onClose: (id: number) => void;
  onTabContextMenu?: (tab: Tab, x: number, y: number) => void;
}) {
  const { t } = useI18n();
  const focusAfterClose = useRef(false);
  const heading = useRef<HTMLHeadingElement>(null);
  const tabList = useRef<HTMLDivElement>(null);

  function focusTab(id: number | undefined) {
    tabList.current?.querySelector<HTMLButtonElement>(`#session-tab-${id}`)?.focus();
  }

  useLayoutEffect(() => {
    tabList.current?.querySelector(`#session-tab-${activeTabId}`)?.scrollIntoView({ block: "nearest", inline: "nearest" });
    if (!focusAfterClose.current) return;
    focusAfterClose.current = false;
    if (tabs.length) focusTab(activeTabId);
    else heading.current?.focus();
  }, [activeTabId, tabs]);

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

  function tabContextMenu(tab: Tab, event: MouseEvent<HTMLDivElement>) {
    if (!onTabContextMenu) return;
    event.preventDefault();
    onTabContextMenu(tab, event.clientX, event.clientY);
  }

  return (
    <div className="panel-header workspace-header">
      <LayoutPanelTop size={14} aria-hidden="true" className="shrink-0 text-muted-foreground" />
      <h2 ref={heading} id="session-heading" tabIndex={-1} className="shrink-0">{t("tabs.title")}</h2>
      {tabs.length > 0 && (
        <div className="session-tabs" ref={tabList} role="tablist" aria-labelledby="session-heading">
          {tabs.map((tab, index) => (
            <div
              key={tab.id}
              className="tab-chip"
              data-active={tab.id === activeTabId}
              onContextMenu={(event) => tabContextMenu(tab, event)}
            >
              <button
                type="button"
                id={`session-tab-${tab.id}`}
                role="tab"
                aria-selected={tab.id === activeTabId}
                aria-controls={`session-panel-${tab.id}`}
                tabIndex={tab.id === activeTabId ? 0 : -1}
                className="tab-select"
                title={tab.target}
                onClick={() => onSelect(tab.id)}
                onKeyDown={(event) => navigate(event, index)}
              >
                <Monitor size={13} aria-hidden="true" className="shrink-0" />
                <span className="truncate">{tab.label}</span>
              </button>
              <button
                type="button"
                aria-label={`${t("tabs.close")} ${tab.label}`}
                tabIndex={tab.id === activeTabId ? 0 : -1}
                className="tab-close"
                onClick={() => {
                  focusAfterClose.current = true;
                  onClose(tab.id);
                }}
              >
                <X size={13} aria-hidden="true" />
              </button>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
