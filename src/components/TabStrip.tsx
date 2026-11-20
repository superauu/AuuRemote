import { X } from "lucide-react";
import { useI18n } from "../i18n";
import type { Tab } from "../types";

export function TabStrip({ tabs, activeTabId, onSelect, onClose }: {
  tabs: Tab[];
  activeTabId?: number;
  onSelect: (id: number) => void;
  onClose: (id: number) => void;
}) {
  const { t } = useI18n();
  if (tabs.length === 0) return null;
  return (
    <div
      className="flex h-9 shrink-0 items-center gap-1 overflow-x-auto px-2"
      role="tablist"
      style={{ borderBottom: "1px solid var(--glass-divider)" }}
    >
      {tabs.map((tab) => (
        <div
          key={tab.id}
          className="tab-chip"
          role="tab"
          aria-selected={tab.id === activeTabId}
          onClick={() => onSelect(tab.id)}
          onKeyDown={(event) => {
            if (event.key === "Enter" || event.key === " ") {
              event.preventDefault();
              onSelect(tab.id);
            }
          }}
          tabIndex={0}
        >
          <span className="truncate" title={tab.target}>{tab.label}</span>
          <button
            type="button"
            aria-label={`${t("tabs.close")} ${tab.label}`}
            className="tab-close"
            onClick={(event) => {
              event.stopPropagation();
              onClose(tab.id);
            }}
          >
            <X size={11} aria-hidden="true" />
          </button>
        </div>
      ))}
    </div>
  );
}
