import { ListFilter, Monitor, Plus, Search } from "lucide-react";
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
      className="panel-surface flex shrink-0 flex-col overflow-hidden"
      style={{ width }}
      aria-label={t("panel.connections")}
    >
      <div className="panel-header">
        <h2 className="truncate">{t("panel.connections")}</h2>
        <span className="count-pill" aria-label={t("panel.count", { count: profiles.length })}>{profiles.length}</span>
        <button
          type="button"
          className="icon-btn ms-auto"
          title={t("toolbar.newConnection")}
          aria-label={t("toolbar.newConnection")}
          onClick={onNew}
        >
          <Plus size={15} aria-hidden="true" />
        </button>
      </div>
      <div className="relative mx-3 my-3">
        <Search size={14} aria-hidden="true" className="pointer-events-none absolute start-2.5 top-1/2 -translate-y-1/2 text-muted-foreground" />
        <input
          type="search"
          aria-label={t("panel.search")}
          placeholder={t("panel.search")}
          value={query}
          onChange={(event) => onQuery(event.target.value)}
          className="field ps-8"
          disabled={profiles.length === 0}
        />
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
                <span className="block truncate text-[13px] font-medium leading-[18px]" title={profile.name}>{profile.name}</span>
                <span className="block truncate font-mono text-[11px] leading-4 text-muted-foreground" title={targetAddress(profile.host, profile.port)}>
                  {targetAddress(profile.host, profile.port)}
                </span>
              </span>
              {live && <span className="state-dot bg-connected" title={t("status.connections", { count: 1 })} />}
            </button>
          );
        })}
        {filtered.length === 0 && (
          <div className="sidebar-empty">
            {profiles.length === 0 ? <Monitor size={18} aria-hidden="true" /> : <ListFilter size={18} aria-hidden="true" />}
            <p>{profiles.length === 0 ? t("panel.empty") : t("panel.noMatch")}</p>
          </div>
        )}
      </div>
    </aside>
  );
}
