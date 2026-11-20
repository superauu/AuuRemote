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
  if (tabs.length === 0) {
    return (
      <div className="flex h-9 shrink-0 items-center border-b border-border/70 px-3">
        <span className="caption">{t("tabs.noTabs")}</span>
      </div>
    );
  }
  return (
    <div className="flex h-9 shrink-0 items-center gap-1.5 overflow-x-auto border-b border-border/70 px-2" role="tablist">
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
            className="-me-1 inline-flex h-5 w-5 shrink-0 items-center justify-center rounded-sm hover:bg-foreground/10"
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
