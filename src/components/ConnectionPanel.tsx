import { Plus, MonitorDot } from "lucide-react";
import { useI18n } from "../i18n";
import { targetAddress, type Profile } from "../types";

export function ConnectionPanel({ profiles, selectedId, openProfileIds, query, onQuery, onSelect, onNew }: {
  profiles: Profile[];
  selectedId?: number;
  /** Profile ids that currently have an open session tab. */
  openProfileIds: Set<number>;
  query: string;
  onQuery: (value: string) => void;
  onSelect: (profile: Profile) => void;
  onNew: () => void;
}) {
  const { t } = useI18n();
  const needle = query.trim().toLocaleLowerCase();
  const filtered = profiles
    .filter(({ name, host, username }) =>
      [name, host, username].some((value) => value.toLocaleLowerCase().includes(needle)))
    .sort((left, right) => left.name.localeCompare(right.name));

  return (
    <aside className="flex w-[228px] shrink-0 flex-col border-r border-border/70" aria-label={t("panel.connections")}>
      <div className="panel-header">
        <span>{t("panel.connections")}</span>
        <span className="caption">{profiles.length}</span>
      </div>
      <div className="p-2">
        {profiles.length > 0 && (
          <input
            type="search"
            aria-label={t("panel.search")}
            placeholder={t("panel.search")}
            value={query}
            onChange={(event) => onQuery(event.target.value)}
            className="h-7 w-full rounded-md border border-input bg-transparent px-2 text-xs placeholder:text-muted-foreground"
          />
        )}
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto px-2 pb-2">
        {filtered.map((profile) => (
          <button
            key={profile.id}
            type="button"
            className="nav-row"
            aria-pressed={selectedId === profile.id}
            onClick={() => onSelect(profile)}
          >
            <MonitorDot
              size={13}
              aria-hidden="true"
              className={openProfileIds.has(profile.id) ? "shrink-0 text-connected" : "shrink-0 text-muted-foreground"}
            />
            <span className="min-w-0 flex-1">
              <span className="block truncate leading-4">{profile.name}</span>
              <span className="block truncate text-[11px] font-normal leading-4 text-muted-foreground">
                {targetAddress(profile.host, profile.port)}
              </span>
            </span>
          </button>
        ))}
        {filtered.length === 0 && (
          <p className="px-2 py-3 text-xs text-muted-foreground">
            {profiles.length === 0 ? t("panel.empty") : t("panel.noMatch")}
          </p>
        )}
      </div>
      <div className="border-t border-border/70 p-2">
        <button type="button" className="nav-row" onClick={onNew}>
          <Plus size={13} aria-hidden="true" className="shrink-0 text-muted-foreground" />
          <span className="truncate">{t("panel.newConnection")}</span>
        </button>
      </div>
    </aside>
  );
}
