import { useMemo, useState } from "react";
import {
  ChevronDown,
  ChevronRight,
  Copy,
  Loader2,
  Pencil,
  Plug,
  Plus,
  Search,
  Settings as SettingsIcon,
  Star,
  Trash2,
  X,
  Zap,
} from "lucide-react";
import { api, protocolLabel, Site } from "../lib/api";
import { useT } from "../lib/i18n";
import { useStore } from "../lib/store";
import { confirmDialog } from "../lib/dialogs";
import { openSession, showError } from "../lib/actions";
import { openContextMenu } from "./ContextMenu";
import { duplicateSite, openSiteEditor } from "./SiteEditor";
import { openSettings } from "./Settings";

export async function newSite() {
  const r = await openSiteEditor();
  if (r?.connect) openSession({ siteId: r.site.id });
}

export async function editSite(site: Site) {
  const r = await openSiteEditor(site);
  if (r?.connect) openSession({ siteId: r.site.id });
}

export async function removeSite(site: Site, t: ReturnType<typeof useT>) {
  const ok = await confirmDialog({
    title: t("common.delete"),
    message: t("site.deleteConfirm", { name: site.name || site.host }),
    confirmLabel: t("common.delete"),
    danger: true,
  });
  if (!ok) return;
  try {
    await api.deleteSite(site.id);
    useStore.getState().removeSite(site.id);
  } catch (e) {
    showError(e);
  }
}

export function Logo({ size = 28 }: { size?: number }) {
  return <img src="/logo.svg" width={size} height={size} alt="" draggable={false} />;
}

export function Sidebar({ onQuickConnect }: { onQuickConnect: () => void }) {
  const t = useT();
  const sites = useStore((s) => s.sites);
  const tabs = useStore((s) => s.tabs);
  const connecting = useStore((s) => s.connecting);
  const sidebarOpen = useStore((s) => s.sidebarOpen);
  const setSidebarOpen = useStore((s) => s.setSidebarOpen);
  const [query, setQuery] = useState("");
  const [collapsed, setCollapsed] = useState<Record<string, boolean>>({});

  const connectedSites = useMemo(() => new Set(tabs.filter((t) => t.connected).map((t) => t.info.siteId)), [tabs]);

  const groups = useMemo(() => {
    const q = query.trim().toLowerCase();
    const filtered = sites
      .filter(
        (s) =>
          !q ||
          s.name.toLowerCase().includes(q) ||
          s.host.toLowerCase().includes(q) ||
          s.username.toLowerCase().includes(q) ||
          s.group.toLowerCase().includes(q),
      )
      .sort((a, b) => (a.name || a.host).localeCompare(b.name || b.host));
    const map = new Map<string, Site[]>();
    const favs = filtered.filter((s) => s.favorite);
    if (favs.length) map.set("\u0000fav", favs);
    for (const s of filtered) {
      const g = s.group || "";
      if (!map.has(g)) map.set(g, []);
      map.get(g)!.push(s);
    }
    return [...map.entries()].sort(([a], [b]) => {
      if (a === "\u0000fav") return -1;
      if (b === "\u0000fav") return 1;
      if (a === "") return 1;
      if (b === "") return -1;
      return a.localeCompare(b);
    });
  }, [sites, query]);

  const siteMenu = (e: React.MouseEvent, site: Site) => {
    e.preventDefault();
    openContextMenu(e.clientX, e.clientY, [
      { label: t("common.connect"), icon: <Plug size={14} />, onClick: () => openSession({ siteId: site.id }) },
      { separator: true },
      { label: t("common.edit"), icon: <Pencil size={14} />, onClick: () => editSite(site) },
      {
        label: t("common.duplicate"),
        icon: <Copy size={14} />,
        onClick: async () => {
          const r = await openSiteEditor(duplicateSite(site));
          if (r?.connect) openSession({ siteId: r.site.id });
        },
      },
      {
        label: site.favorite ? "★" : "☆",
        icon: <Star size={14} />,
        onClick: async () => {
          const saved = await api.saveSite({ ...site, favorite: !site.favorite }, {}).catch(showError);
          if (saved) useStore.getState().upsertSite(saved);
        },
      },
      { separator: true },
      { label: t("common.delete"), icon: <Trash2 size={14} />, danger: true, onClick: () => removeSite(site, t) },
    ]);
  };

  return (
    <>
      <div className={`sidebar-scrim ${sidebarOpen ? "open" : ""}`} onClick={() => setSidebarOpen(false)} />
      <aside className={`sidebar ${sidebarOpen ? "open" : ""}`}>
        <div className="sidebar-brand">
          <Logo />
          <span className="brand-name">SFTPinguin</span>
          <button className="icon-btn mobile-only" onClick={() => setSidebarOpen(false)}>
            <X size={18} />
          </button>
        </div>

        <div className="sidebar-actions">
          <button className="btn btn-primary grow" onClick={newSite}>
            <Plus size={15} /> {t("sidebar.newSite")}
          </button>
          <button
            className="btn icon-only"
            title={t("sidebar.quickConnect")}
            onClick={() => {
              setSidebarOpen(false);
              onQuickConnect();
            }}
          >
            <Zap size={15} />
          </button>
        </div>

        {sites.length > 4 && (
          <div className="search">
            <Search size={14} />
            <input value={query} onChange={(e) => setQuery(e.target.value)} placeholder={t("common.search")} />
            {query && (
              <button className="icon-btn" onClick={() => setQuery("")}>
                <X size={12} />
              </button>
            )}
          </div>
        )}

        <nav className="site-list">
          {sites.length === 0 && <p className="sidebar-empty">{t("sidebar.noSites")}</p>}
          {groups.map(([group, list]) => {
            const label = group === "\u0000fav" ? t("sidebar.favorites") : group || t("sidebar.ungrouped");
            const isCollapsed = collapsed[group];
            return (
              <div key={group} className="site-group">
                <button className="group-header" onClick={() => setCollapsed({ ...collapsed, [group]: !isCollapsed })}>
                  {isCollapsed ? <ChevronRight size={12} /> : <ChevronDown size={12} />}
                  <span>{label}</span>
                  <span className="count">{list.length}</span>
                </button>
                {!isCollapsed &&
                  list.map((site) => {
                    const busy = !!connecting[site.id];
                    const connected = connectedSites.has(site.id);
                    return (
                      <button
                        key={`${group}-${site.id}`}
                        className={`site-item ${connected ? "connected" : ""}`}
                        onClick={() => {
                          const tab = useStore.getState().tabs.find((x) => x.info.siteId === site.id);
                          if (tab) useStore.getState().setActiveTab(tab.id);
                          else openSession({ siteId: site.id });
                        }}
                        onDoubleClick={() => openSession({ siteId: site.id })}
                        onContextMenu={(e) => siteMenu(e, site)}
                        title={`${protocolLabel(site.protocol)} · ${site.username ? site.username + "@" : ""}${site.host}`}
                      >
                        <span className="site-dot" style={site.color ? { background: site.color } : undefined}>
                          {protocolLabel(site.protocol).slice(0, 1)}
                        </span>
                        <span className="site-text">
                          <span className="site-name">{site.name || site.host}</span>
                          <span className="site-sub">
                            {protocolLabel(site.protocol)} · {site.host || site.endpoint || "s3"}
                          </span>
                        </span>
                        {busy ? (
                          <Loader2 size={14} className="spin muted" />
                        ) : connected ? (
                          <span className="live-dot" title={t("sidebar.connected")} />
                        ) : (
                          <span
                            className="icon-btn site-edit"
                            role="button"
                            onClick={(e) => {
                              e.stopPropagation();
                              editSite(site);
                            }}
                          >
                            <Pencil size={13} />
                          </span>
                        )}
                      </button>
                    );
                  })}
              </div>
            );
          })}
        </nav>

        <div className="sidebar-footer">
          <button className="btn btn-ghost grow" onClick={() => openSettings()}>
            <SettingsIcon size={15} /> {t("sidebar.settings")}
          </button>
        </div>
      </aside>
    </>
  );
}
