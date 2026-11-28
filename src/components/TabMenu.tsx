import { Camera, CopyPlus, Maximize, Minimize, Pencil, PlugZap, RefreshCw, Scan, type LucideIcon } from "lucide-react";
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { useI18n } from "../i18n";
import type { Tab } from "../types";

export type TabMenuState = { tab: Tab; x: number; y: number };
export type TabMenuAction = "fullscreen" | "smartSize" | "screenshot" | "rename" | "duplicate" | "reconnect" | "disconnect";

type Item = {
  action: TabMenuAction;
  icon: LucideIcon;
  label: string;
  checked?: boolean;
  disabled?: boolean;
};

export function TabMenu({ menu, fullscreen, smartSize, smartSizeLocked, onClose, onAction }: {
  menu: TabMenuState;
  /** Current window fullscreen state, for the toggle label. */
  fullscreen: boolean;
  /** Whether the right-clicked session follows the canvas size ("auto"). */
  smartSize: boolean;
  /** Smart Size is on but no fixed resolution exists to fall back to. */
  smartSizeLocked: boolean;
  onClose: () => void;
  onAction: (action: TabMenuAction, tab: Tab) => void;
}) {
  const { t } = useI18n();
  const menuCard = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ x: menu.x, y: menu.y });

  useLayoutEffect(() => {
    const rect = menuCard.current?.getBoundingClientRect();
    if (!rect) return;
    setPos({
      x: Math.max(8, Math.min(menu.x, window.innerWidth - rect.width - 8)),
      y: Math.max(8, Math.min(menu.y, window.innerHeight - rect.height - 8)),
    });
    menuCard.current?.focus();
  }, [menu.x, menu.y]);

  useEffect(() => {
    function onPointerDown(event: PointerEvent) {
      if (!menuCard.current?.contains(event.target as Node)) onClose();
    }
    function onKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") onClose();
    }
    window.addEventListener("pointerdown", onPointerDown, true);
    window.addEventListener("keydown", onKeyDown, true);
    window.addEventListener("resize", onClose);
    return () => {
      window.removeEventListener("pointerdown", onPointerDown, true);
      window.removeEventListener("keydown", onKeyDown, true);
      window.removeEventListener("resize", onClose);
    };
  }, [onClose]);

  const items: Item[] = [
    {
      action: "fullscreen",
      icon: fullscreen ? Minimize : Maximize,
      label: t(fullscreen ? "menu.exitFullscreen" : "menu.fullscreen"),
    },
    {
      action: "smartSize",
      icon: Scan,
      label: t("menu.smartSize"),
      checked: smartSize,
      disabled: smartSize && smartSizeLocked,
    },
    { action: "screenshot", icon: Camera, label: t("session.screenshot") },
    { action: "rename", icon: Pencil, label: t("menu.rename") },
    { action: "duplicate", icon: CopyPlus, label: t("menu.duplicate") },
    { action: "reconnect", icon: RefreshCw, label: t("menu.reconnect") },
    { action: "disconnect", icon: PlugZap, label: t("menu.disconnect") },
  ];

  return (
    <div
      ref={menuCard}
      className="tab-menu animate-fade-in"
      style={{ left: pos.x, top: pos.y }}
      role="menu"
      aria-label={t("tabs.title")}
      tabIndex={-1}
      onBlur={(event) => {
        if (!menuCard.current?.contains(event.relatedTarget as Node)) onClose();
      }}
    >
      {items.map(({ action, icon: Icon, label, checked, disabled }) => (
        <button
          key={action}
          type="button"
          role="menuitemcheckbox"
          aria-checked={checked}
          className="tab-menu-item"
          disabled={disabled}
          onClick={() => {
            onClose();
            onAction(action, menu.tab);
          }}
        >
          <Icon size={13} aria-hidden="true" />
          <span className="flex-1 truncate text-start">{label}</span>
          {checked && <span className="tab-menu-check" aria-hidden="true" />}
        </button>
      ))}
    </div>
  );
}
