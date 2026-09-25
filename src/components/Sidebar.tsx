import { useMemo, useRef, useState } from "react";
import {
  ChevronDown,
  ChevronRight,
  Copy,
  Folder,
  FolderInput,
  FolderOpen,
  FolderPlus,
  Loader2,
  LockOpen,
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
import { api, emptySite, isUnencrypted, protocolLabel, Site } from "../lib/api";
import { useT } from "../lib/i18n";
import { useStore } from "../lib/store";
import { confirmDialog, promptDialog } from "../lib/dialogs";
import { openSession, reloadSites, showError } from "../lib/actions";
import { MenuItem, openContextMenu } from "./ContextMenu";
import { duplicateSite, openSiteEditor } from "./SiteEditor";
import { openSettings } from "./Settings";
import { createFolder, folderLabel, pickFolder } from "./FolderPicker";

export async function newSite(folder = "") {
  const r = await openSiteEditor(folder ? { ...emptySite(), group: folder } : undefined);
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

async function moveSites(ids: string[], folder: string) {
  try {
    await api.moveSites(ids, folder);
    await reloadSites();
  } catch (e) {
    showError(e);
  }
}

async function renameFolder(path: string, t: ReturnType<typeof useT>) {
  const name = await promptDialog({ title: t("folders.rename"), label: t("folders.name"), initial: folderLabel(path) });
  if (!name || name === folderLabel(path)) return;
  if (name.includes("/")) {
    useStore.getState().toast({ kind: "error", message: t("folders.invalid") });
    return;
  }
  const parent = path.includes("/") ? path.slice(0, path.lastIndexOf("/")) : "";
  try {
    await api.renameFolder(path, parent ? `${parent}/${name}` : name);
    await reloadSites();
  } catch (e) {
    showError(e);
  }
}

async function deleteFolder(path: string, t: ReturnType<typeof useT>) {
  const ok = await confirmDialog({
    title: t("folders.delete"),
    message: t("folders.deleteConfirm", { name: folderLabel(path) }),
    confirmLabel: t("common.delete"),
    danger: true,
  });
  if (!ok) return;
  try {
    await api.deleteFolder(path);
    await reloadSites();
  } catch (e) {
    showError(e);
  }
}

export function Logo({ size = 28 }: { size?: number }) {
  return <img src="/logo.svg" width={size} height={size} alt="" draggable={false} />;
}

interface FolderNode {
  path: string;
  name: string;
  children: FolderNode[];
  sites: Site[];
  total: number;
}

function buildTree(folders: string[], sites: Site[]): FolderNode {
  const root: FolderNode = { path: "", name: "", children: [], sites: [], total: 0 };
  const byPath = new Map<string, FolderNode>([["", root]]);
  const ensure = (path: string): FolderNode => {
    const existing = byPath.get(path);
    if (existing) return existing;
    const idx = path.lastIndexOf("/");
    const parent = ensure(idx >= 0 ? path.slice(0, idx) : "");
    const node: FolderNode = { path, name: path.slice(idx + 1), children: [], sites: [], total: 0 };
    parent.children.push(node);
    byPath.set(path, node);
    return node;
  };
  for (const f of folders) ensure(f);
  for (const s of sites) ensure(s.group).sites.push(s);
  const sortRec = (n: FolderNode): number => {
    n.children.sort((a, b) => a.name.localeCompare(b.name));
    n.sites.sort((a, b) => (a.name || a.host).localeCompare(b.name || b.host));
    n.total = n.sites.length + n.children.reduce((sum, c) => sum + sortRec(c), 0);
    return n.total;
  };
  sortRec(root);
  return root;
}

/** Pointer based drag & drop of servers onto folders (works on every platform). */
function useSiteDrag() {
  const [dragging, setDragging] = useState<{ site: Site; x: number; y: number } | null>(null);
  const [target, setTarget] = useState<string | null>(null);
  const start = useRef<{ x: number; y: number; site: Site } | null>(null);
  const suppressClick = useRef(false);

  const onPointerDown = (e: React.PointerEvent, site: Site) => {
    if (e.button !== 0 || e.pointerType === "touch") return;
    start.current = { x: e.clientX, y: e.clientY, site };
    let active = false;
    const folderAt = (x: number, y: number) => {
      const el = document.elementFromPoint(x, y) as HTMLElement | null;
      const f = el?.closest<HTMLElement>("[data-folder]");
      return f ? (f.dataset.folder ?? "") : null;
    };
    const move = (ev: PointerEvent) => {
      const s = start.current;
      if (!s) return;
      if (!active && Math.hypot(ev.clientX - s.x, ev.clientY - s.y) < 6) return;
      active = true;
      document.body.classList.add("dragging");
      setDragging({ site: s.site, x: ev.clientX, y: ev.clientY });
      setTarget(folderAt(ev.clientX, ev.clientY));
    };
    const up = (ev: PointerEvent) => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      document.body.classList.remove("dragging");
      const s = start.current;
      start.current = null;
      if (active && s) {
        suppressClick.current = true;
        setTimeout(() => (suppressClick.current = false), 0);
        const folder = folderAt(ev.clientX, ev.clientY);
        if (folder !== null && folder !== s.site.group) moveSites([s.site.id], folder);
      }
      setDragging(null);
      setTarget(null);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  };

  return { dragging, target, onPointerDown, suppressClick };
}

export function Sidebar({ onQuickConnect }: { onQuickConnect: () => void }) {
  const t = useT();
  const sites = useStore((s) => s.sites);
  const folders = useStore((s) => s.folders);
  const tabs = useStore((s) => s.tabs);
  const connecting = useStore((s) => s.connecting);
  const sidebarOpen = useStore((s) => s.sidebarOpen);
  const setSidebarOpen = useStore((s) => s.setSidebarOpen);
  const collapsedList = useStore((s) => s.settings.collapsedFolders);
  const updateSettings = useStore((s) => s.updateSettings);
  const [query, setQuery] = useState("");
  const drag = useSiteDrag();

  const collapsed = useMemo(() => new Set(collapsedList), [collapsedList]);
  const toggle = (path: string) => {
    const next = new Set(collapsed);
    if (next.has(path)) next.delete(path);
    else next.add(path);
    updateSettings({ collapsedFolders: [...next] });
  };

  const connectedSites = useMemo(() => new Set(tabs.filter((t) => t.connected).map((t) => t.info.siteId)), [tabs]);

  const q = query.trim().toLowerCase();
  const { favorites, tree } = useMemo(() => {
    const matches = (s: Site) =>
      !q ||
      s.name.toLowerCase().includes(q) ||
      s.host.toLowerCase().includes(q) ||
      s.username.toLowerCase().includes(q) ||
      s.group.toLowerCase().includes(q);
    const filtered = sites.filter(matches);
    return {
      favorites: filtered
        .filter((s) => s.favorite)
        .sort((a, b) => (a.name || a.host).localeCompare(b.name || b.host)),
      // favorites are listed once, in their own section; while searching only
      // folders containing hits are shown
      tree: buildTree(q ? [] : folders, filtered.filter((s) => !s.favorite)),
    };
  }, [folders, sites, q]);

  const openSite = (site: Site) => {
    if (drag.suppressClick.current) return;
    const tab = useStore.getState().tabs.find((x) => x.info.siteId === site.id);
    if (tab) useStore.getState().setActiveTab(tab.id);
    else openSession({ siteId: site.id });
  };

  const siteMenu = (e: React.MouseEvent, site: Site) => {
    e.preventDefault();
    e.stopPropagation();
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
        label: t("folders.moveTo"),
        icon: <FolderInput size={14} />,
        onClick: async () => {
          const folder = await pickFolder(site.group);
          if (folder !== null && folder !== site.group) moveSites([site.id], folder);
        },
      },
      {
        label: site.favorite ? t("site.favoriteRemove") : t("site.favoriteAdd"),
        icon: <Star size={14} />,
        onClick: async () => {
          const res = await api.saveSite({ ...site, favorite: !site.favorite }, {}).catch(showError);
          if (res) useStore.getState().upsertSite(res.site);
        },
      },
      { separator: true },
      { label: t("common.delete"), icon: <Trash2 size={14} />, danger: true, onClick: () => removeSite(site, t) },
    ]);
  };

  const folderMenu = (e: React.MouseEvent, path: string) => {
    e.preventDefault();
    e.stopPropagation();
    const items: MenuItem[] = [
      { label: t("folders.newServerHere"), icon: <Plus size={14} />, onClick: () => newSite(path) },
      { label: t("folders.newSub"), icon: <FolderPlus size={14} />, onClick: () => createFolder(path) },
      { separator: true },
      { label: t("folders.rename"), icon: <Pencil size={14} />, onClick: () => renameFolder(path, t) },
      { label: t("folders.delete"), icon: <Trash2 size={14} />, danger: true, onClick: () => deleteFolder(path, t) },
    ];
    openContextMenu(e.clientX, e.clientY, items);
  };

  const listMenu = (e: React.MouseEvent) => {
    e.preventDefault();
    openContextMenu(e.clientX, e.clientY, [
      { label: t("sidebar.newSite"), icon: <Plus size={14} />, onClick: () => newSite() },
      { label: t("folders.new"), icon: <FolderPlus size={14} />, onClick: () => createFolder() },
    ]);
  };

  const renderSite = (site: Site, depth: number, keyPrefix: string) => {
    const busy = !!connecting[site.id];
    const connected = connectedSites.has(site.id);
    return (
      <button
        key={`${keyPrefix}-${site.id}`}
        className={`site-item ${connected ? "connected" : ""} ${drag.dragging?.site.id === site.id ? "drag-source" : ""}`}
        style={{ paddingLeft: 8 + depth * 14 }}
        onPointerDown={(e) => drag.onPointerDown(e, site)}
        onClick={() => openSite(site)}
        onContextMenu={(e) => siteMenu(e, site)}
        title={`${protocolLabel(site.protocol)} · ${site.username ? site.username + "@" : ""}${site.host}`}
      >
        <span className="site-dot" style={site.color ? { background: site.color } : undefined}>
          {protocolLabel(site.protocol).slice(0, 1)}
        </span>
        <span className="site-text">
          <span className="site-name">{site.name || site.host}</span>
          <span className="site-sub">
            {isUnencrypted(site) && <LockOpen size={10} className="text-danger insecure-mark" />}
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
            onPointerDown={(e) => e.stopPropagation()}
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
  };

  const renderFolder = (node: FolderNode, depth: number): React.ReactNode => {
    const isCollapsed = collapsed.has(node.path) && !q;
    const Icon = isCollapsed ? Folder : FolderOpen;
    return (
      <div key={node.path} className={`folder ${drag.target === node.path ? "drop-target" : ""}`} data-folder={node.path}>
        <button
          className="folder-header"
          style={{ paddingLeft: 4 + depth * 14 }}
          onClick={() => toggle(node.path)}
          onContextMenu={(e) => folderMenu(e, node.path)}
        >
          {isCollapsed ? <ChevronRight size={12} /> : <ChevronDown size={12} />}
          <Icon size={14} className="folder-icon" />
          <span className="folder-name">{node.name}</span>
          <span className="count">{node.total}</span>
        </button>
        {!isCollapsed && (
          <>
            {node.children.map((c) => renderFolder(c, depth + 1))}
            {node.sites.map((s) => renderSite(s, depth + 1, node.path))}
          </>
        )}
      </div>
    );
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
          <button className="btn btn-primary grow" onClick={() => newSite()}>
            <Plus size={15} /> {t("sidebar.newSite")}
          </button>
          <button className="btn icon-only" title={t("folders.new")} onClick={() => createFolder()}>
            <FolderPlus size={15} />
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

        <nav className="site-list" onContextMenu={listMenu}>
          {sites.length === 0 && folders.length === 0 && <p className="sidebar-empty">{t("sidebar.noSites")}</p>}

          {favorites.length > 0 && (
            <div className="site-group">
              <div className="group-header">
                <Star size={11} />
                <span>{t("sidebar.favorites")}</span>
                <span className="count">{favorites.length}</span>
              </div>
              {favorites.map((s) => renderSite(s, 0, "fav"))}
            </div>
          )}

          <div className={`site-group root-drop ${drag.target === "" ? "drop-target" : ""}`} data-folder="">
            {favorites.length > 0 && tree.total > 0 && (
              <div className="group-header">
                <span>{t("sidebar.sites")}</span>
                <span className="count">{tree.total}</span>
              </div>
            )}
            {tree.children.map((c) => renderFolder(c, 0))}
            {tree.sites.map((s) => renderSite(s, 0, "root"))}
          </div>
        </nav>

        <div className="sidebar-footer">
          <button className="btn btn-ghost grow" onClick={() => openSettings()}>
            <SettingsIcon size={15} /> {t("sidebar.settings")}
          </button>
        </div>
      </aside>
      {drag.dragging && (
        <div className="drag-ghost" style={{ left: drag.dragging.x + 14, top: drag.dragging.y + 10 }}>
          <FolderInput size={14} /> {drag.dragging.site.name || drag.dragging.site.host}
        </div>
      )}
    </>
  );
}
