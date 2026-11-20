import { HardDrive, Monitor, Plus, Search } from "lucide-react";
import { useI18n } from "../i18n";
import { targetAddress, type Profile } from "../types";

export function ConnectionPanel({ profiles, selectedId, openProfileIds, query, onQuery, onSelect, onNew, width }: {
  profiles: Profile[];
  selectedId?: number;
  /** Profile ids that currently have an open session tab. */
  openProfileIds: Set<number>;
  query: string;
  onQuery: (value: string) => void;
  onSelect: (profile: Profile) => void;
  onNew: () => void;
  width: number;
}) {
  const { t } = useI18n();
  const needle = query.trim().toLocaleLowerCase();
  const filtered = profiles
    .filter(({ name, host, username }) =>
      [name, host, username].some((value) => value.toLocaleLowerCase().includes(needle)))
    .sort((left, right) => left.name.localeCompare(right.name));

  return (
    <aside
      className="flex flex-col overflow-hidden panel-surface"
      style={{ width, borderRight: "1px solid var(--glass-divider)" }}
      aria-label={t("panel.connections")}
    >
      <div className="panel-header">
        <HardDrive size={13} aria-hidden="true" className="text-muted-foreground" />
        <span className="truncate">{t("panel.connections")}</span>
        <span className="count-pill ms-auto">{profiles.length}</span>
      </div>
      <div className="p-2 pb-1">
        {profiles.length > 0 && (
          <div className="relative">
            <Search size={12} aria-hidden="true" className="pointer-events-none absolute start-2.5 top-1/2 -translate-y-1/2 text-muted-foreground" />
            <input
              type="search"
              aria-label={t("panel.search")}
              placeholder={t("panel.search")}
              value={query}
              onChange={(event) => onQuery(event.target.value)}
              className="field !ps-7"
            />
          </div>
        )}
      </div>
      <div className="min-h-0 flex-1 space-y-0.5 overflow-y-auto px-2 pb-2">
        {filtered.map((profile) => {
          const live = openProfileIds.has(profile.id);
          return (
            <button
              key={profile.id}
              type="button"
              className="conn-row"
              aria-pressed={selectedId === profile.id}
              onClick={() => onSelect(profile)}
            >
              <span className={`icon-tile ${live ? "live" : ""}`} aria-hidden="true">
                <Monitor size={14} />
              </span>
              <span className="min-w-0 flex-1">
                <span className="flex items-center gap-1.5">
                  <span className="truncate text-[13px] font-medium leading-[18px]">{profile.name}</span>
                  {live && <span className="state-dot bg-connected" aria-label={t("status.connections", { count: 1 })} />}
                </span>
                <span className="block truncate text-[11px] leading-4 text-muted-foreground">
                  {targetAddress(profile.host, profile.port)}
                </span>
              </span>
            </button>
          );
        })}
        {filtered.length === 0 && (
          <p className="px-2 py-4 text-xs text-muted-foreground">
            {profiles.length === 0 ? t("panel.empty") : t("panel.noMatch")}
          </p>
        )}
      </div>
      <div className="p-2 pt-1" style={{ borderTop: "1px solid var(--glass-divider)" }}>
        <button type="button" className="nav-row text-muted-foreground hover:!text-foreground" onClick={onNew}>
          <Plus size={13} aria-hidden="true" />
          <span className="truncate">{t("panel.newConnection")}</span>
        </button>
      </div>
    </aside>
  );
}
