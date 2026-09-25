import { ReactNode, useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  AlertCircle,
  ArrowDown,
  ArrowUp,
  ChevronRight,
  Copy,
  Eye,
  EyeOff,
  FilePen,
  FilePlus2,
  FolderInput,
  FolderPlus,
  Home,
  Loader2,
  MapPin,
  Pencil,
  RefreshCw,
  Search,
  ShieldCheck,
  SquareArrowOutUpRight,
  Trash2,
  X,
  CornerLeftUp,
  PlugZap,
} from "lucide-react";
import type { FileEntry, Place } from "../lib/api";
import { resolveLang, useT, TKey } from "../lib/i18n";
import { DragState, useStore } from "../lib/store";
import { confirmDialog, promptDialog } from "../lib/dialogs";
import { errorText, showError } from "../lib/actions";
import { formatDate, formatSize, isHidden, modeString } from "../lib/format";
import { FileIcon } from "./FileIcon";
import { MenuItem, openContextMenu } from "./ContextMenu";
import { chmodDialog } from "./ChmodDialog";

export interface PaneAdapter {
  side: "local" | "remote";
  list: (path: string) => Promise<FileEntry[]>;
  mkdir: (path: string) => Promise<void>;
  createFile: (path: string) => Promise<void>;
  rename: (from: string, to: string) => Promise<void>;
  remove: (entries: FileEntry[]) => Promise<void>;
  chmod?: (entries: FileEntry[], mode: number, recursive: boolean) => Promise<void>;
  join: (dir: string, name: string) => string;
  parent: (path: string) => string;
  crumbs: (path: string) => { label: string; path: string }[];
  home: string;
  places?: Place[];
  trashHint?: boolean;
}

export interface DropDetail {
  drag: DragState;
  dir: string;
}

interface Props {
  adapter: PaneAdapter;
  sessionId: string;
  path: string;
  onNavigate: (path: string) => void;
  version: number;
  connected: boolean;
  onReconnect?: () => void;
  onTransfer: (entries: FileEntry[]) => void;
  onOpen?: (entry: FileEntry) => void;
  onDrop: (drag: DragState, dir: string) => void;
  onOsDrop?: (paths: string[], dir: string) => void;
  onEntries?: (entries: FileEntry[]) => void;
  title: string;
  titleIcon: ReactNode;
  transferLabel: string;
  transferIcon: ReactNode;
  headerExtra?: ReactNode;
  className?: string;
}

type SortKey = "name" | "size" | "modified";

const collator = new Intl.Collator(undefined, { numeric: true, sensitivity: "base" });

export function FilePane(props: Props) {
  const {
    adapter,
    sessionId,
    path,
    onNavigate,
    version,
    connected,
    onTransfer,
    onOpen,
    onDrop,
    onOsDrop,
    onEntries,
  } = props;
  const t = useT();
  const settings = useStore((s) => s.settings);
  const updateSettings = useStore((s) => s.updateSettings);
  const mobile = useStore((s) => !!s.platform?.mobile);
  const lang = resolveLang(settings.language);

  const [entries, setEntries] = useState<FileEntry[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [cursor, setCursor] = useState<number>(-1);
  const [anchor, setAnchor] = useState<number>(-1);
  const [sort, setSort] = useState<{ key: SortKey; asc: boolean }>({ key: "name", asc: true });
  const [filter, setFilter] = useState("");
  const [filterOpen, setFilterOpen] = useState(false);
  const [editingPath, setEditingPath] = useState(false);
  const [pathInput, setPathInput] = useState(path);
  const [osDropHover, setOsDropHover] = useState(false);

  const rootRef = useRef<HTMLElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const loadSeq = useRef(0);

  // ---------------------------------------------------------------- loading
  const load = useCallback(
    async (opts?: { keepSelection?: boolean }) => {
      if (!connected) return;
      const seq = ++loadSeq.current;
      setLoading(true);
      try {
        const list = await adapter.list(path);
        if (seq !== loadSeq.current) return;
        setEntries(list);
        setError(null);
        onEntries?.(list);
        if (!opts?.keepSelection) {
          setSelected(new Set());
          setCursor(-1);
          setAnchor(-1);
        } else {
          const paths = new Set(list.map((e) => e.path));
          setSelected((sel) => new Set([...sel].filter((p) => paths.has(p))));
        }
      } catch (e) {
        if (seq !== loadSeq.current) return;
        setError(errorText(e));
        setEntries([]);
        onEntries?.([]);
      } finally {
        if (seq === loadSeq.current) setLoading(false);
      }
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [adapter, path, connected],
  );

  useEffect(() => {
    load();
    setPathInput(path);
    setFilter("");
  }, [path, adapter, connected]); // eslint-disable-line react-hooks/exhaustive-deps

  const firstVersion = useRef(version);
  useEffect(() => {
    if (version !== firstVersion.current) load({ keepSelection: true });
  }, [version]); // eslint-disable-line react-hooks/exhaustive-deps

  // ---------------------------------------------------------------- view
  const visible = useMemo(() => {
    const q = filter.trim().toLowerCase();
    const list = entries.filter(
      (e) => (settings.showHidden || !isHidden(e.name)) && (!q || e.name.toLowerCase().includes(q)),
    );
    const dir = sort.asc ? 1 : -1;
    list.sort((a, b) => {
      if (a.kind !== b.kind) return a.kind === "dir" ? -1 : 1;
      let r = 0;
      if (sort.key === "size") r = a.size - b.size;
      else if (sort.key === "modified") r = (a.modified ?? 0) - (b.modified ?? 0);
      if (r === 0) r = collator.compare(a.name, b.name);
      return r * dir;
    });
    return list;
  }, [entries, filter, sort, settings.showHidden]);

  const selectedEntries = useMemo(() => visible.filter((e) => selected.has(e.path)), [visible, selected]);
  const selectedSize = selectedEntries.reduce((s, e) => s + (e.kind === "file" ? e.size : 0), 0);
  const isRoot = adapter.parent(path) === path || path === "/" || path === "";

  // ---------------------------------------------------------------- actions
  const navigate = (p: string) => {
    if (p !== path) onNavigate(p);
  };

  const goUp = () => {
    if (!isRoot) navigate(adapter.parent(path));
  };

  const openEntry = async (e: FileEntry) => {
    if (e.kind === "dir") return navigate(e.path);
    if (e.isLink) {
      // symlink with unknown target type (FTP): try to enter it
      try {
        await adapter.list(e.path);
        return navigate(e.path);
      } catch {
        /* not a directory */
      }
    }
    if (settings.doubleClick === "open" && onOpen) onOpen(e);
    else onTransfer([e]);
  };

  const run = async (fn: () => Promise<void>, refresh = true) => {
    try {
      await fn();
    } catch (e) {
      showError(e);
    }
    if (refresh) load({ keepSelection: true });
  };

  const newFolder = async () => {
    const name = await promptDialog({ title: t("pane.newFolder"), label: t("pane.folderName") });
    if (name) run(() => adapter.mkdir(adapter.join(path, name)));
  };

  const newFile = async () => {
    const name = await promptDialog({ title: t("pane.newFile"), label: t("pane.fileName") });
    if (name) run(() => adapter.createFile(adapter.join(path, name)));
  };

  const rename = async (e: FileEntry) => {
    const name = await promptDialog({
      title: t("common.rename"),
      label: t("pane.newName"),
      initial: e.name,
      selectStem: e.kind === "file",
    });
    if (name && name !== e.name) run(() => adapter.rename(e.path, adapter.join(adapter.parent(e.path), name)));
  };

  const remove = async (list: FileEntry[]) => {
    if (!list.length) return;
    if (settings.confirmDelete) {
      const ok = await confirmDialog({
        title: t("common.delete"),
        message: (
          <>
            <p>{list.length === 1 ? t("pane.deleteConfirmOne", { name: list[0].name }) : t("pane.deleteConfirm", { count: list.length })}</p>
            <p className="muted small">{adapter.trashHint ? t("pane.deleteTrashHint") : t("pane.deleteRemoteHint")}</p>
          </>
        ),
        confirmLabel: t("common.delete"),
        danger: true,
      });
      if (!ok) return;
    }
    run(() => adapter.remove(list));
  };

  const chmod = async (list: FileEntry[]) => {
    if (!adapter.chmod || !list.length) return;
    const initial = list[0].mode ?? (list[0].kind === "dir" ? 0o755 : 0o644);
    const res = await chmodDialog(initial, list.some((e) => e.kind === "dir"));
    if (res) run(() => adapter.chmod!(list, res.mode, res.recursive));
  };

  const copyPath = (p: string) => {
    navigator.clipboard?.writeText(p).then(
      () => useStore.getState().toast({ kind: "success", message: t("common.copied") }),
      () => {},
    );
  };

  const moveInto = (list: FileEntry[], dir: string) => {
    const movable = list.filter((e) => e.path !== dir && adapter.parent(e.path) !== dir);
    if (!movable.length) return;
    run(async () => {
      for (const e of movable) await adapter.rename(e.path, adapter.join(dir, e.name));
    });
  };

  // ---------------------------------------------------------------- selection
  const selectIndex = (idx: number, e?: { shiftKey?: boolean; ctrlKey?: boolean; metaKey?: boolean }) => {
    const entry = visible[idx];
    if (!entry) return;
    if (e?.shiftKey && anchor >= 0) {
      const [a, b] = anchor < idx ? [anchor, idx] : [idx, anchor];
      const next = new Set(e.ctrlKey || e.metaKey ? selected : []);
      for (let i = a; i <= b; i++) next.add(visible[i].path);
      setSelected(next);
    } else if (e?.ctrlKey || e?.metaKey || (mobile && selected.size > 0)) {
      const next = new Set(selected);
      if (next.has(entry.path)) next.delete(entry.path);
      else next.add(entry.path);
      setSelected(next);
      setAnchor(idx);
    } else {
      setSelected(new Set([entry.path]));
      setAnchor(idx);
    }
    setCursor(idx);
  };

  useEffect(() => {
    if (cursor < 0 || !listRef.current) return;
    const el = listRef.current.querySelector<HTMLElement>(`[data-index="${cursor}"]`);
    el?.scrollIntoView({ block: "nearest" });
  }, [cursor]);

  const onKeyDown = (e: React.KeyboardEvent) => {
    if ((e.target as HTMLElement).tagName === "INPUT") return;
    const mod = e.ctrlKey || e.metaKey;
    switch (e.key) {
      case "ArrowDown":
      case "ArrowUp": {
        e.preventDefault();
        if (e.key === "ArrowUp" && e.altKey) return goUp();
        const next = Math.max(0, Math.min(visible.length - 1, cursor + (e.key === "ArrowDown" ? 1 : -1)));
        selectIndex(next, { shiftKey: e.shiftKey });
        if (!e.shiftKey) setAnchor(next);
        break;
      }
      case "Home":
        e.preventDefault();
        selectIndex(0);
        break;
      case "End":
        e.preventDefault();
        selectIndex(visible.length - 1);
        break;
      case "Enter":
        if (cursor >= 0 && visible[cursor]) {
          e.preventDefault();
          if (selectedEntries.length > 1) onTransfer(selectedEntries);
          else openEntry(visible[cursor]);
        }
        break;
      case "Backspace":
        e.preventDefault();
        goUp();
        break;
      case "Delete":
        e.preventDefault();
        remove(selectedEntries);
        break;
      case "F2":
        if (selectedEntries.length === 1) {
          e.preventDefault();
          rename(selectedEntries[0]);
        }
        break;
      case "F5":
        e.preventDefault();
        load({ keepSelection: true });
        break;
      case "Escape":
        setSelected(new Set());
        setFilterOpen(false);
        setFilter("");
        break;
      default:
        if (mod && e.key.toLowerCase() === "a") {
          e.preventDefault();
          setSelected(new Set(visible.map((x) => x.path)));
        } else if (mod && e.key.toLowerCase() === "r") {
          e.preventDefault();
          load({ keepSelection: true });
        } else if (mod && e.key.toLowerCase() === "f") {
          e.preventDefault();
          setFilterOpen(true);
        } else if (e.key.length === 1 && !mod && !e.altKey && e.key !== " ") {
          // type-ahead: jump to the first entry starting with the typed character
          const ch = e.key.toLowerCase();
          const start = cursor + 1;
          const order = [...visible.slice(start), ...visible.slice(0, start)];
          const hit = order.find((x) => x.name.toLowerCase().startsWith(ch));
          if (hit) selectIndex(visible.indexOf(hit));
        }
    }
  };

  // ---------------------------------------------------------------- context menus
  const rowMenu = (ev: React.MouseEvent, entry: FileEntry, idx: number) => {
    ev.preventDefault();
    ev.stopPropagation();
    let list = selectedEntries;
    if (!selected.has(entry.path)) {
      setSelected(new Set([entry.path]));
      setAnchor(idx);
      setCursor(idx);
      list = [entry];
    }
    const single = list.length === 1 ? list[0] : null;
    const items: MenuItem[] = [];
    if (single?.kind === "dir") {
      items.push({ label: t("pane.open"), icon: <FolderInput size={14} />, onClick: () => navigate(single.path) });
    }
    items.push({ label: props.transferLabel, icon: props.transferIcon, onClick: () => onTransfer(list) });
    if (single?.kind === "file" && onOpen) {
      items.push({
        label: adapter.side === "remote" && !mobile ? t("pane.editFile") : t("pane.open"),
        icon: adapter.side === "remote" && !mobile ? <FilePen size={14} /> : <SquareArrowOutUpRight size={14} />,
        onClick: () => onOpen(single),
      });
    }
    items.push({ separator: true });
    if (single) items.push({ label: t("common.rename"), icon: <Pencil size={14} />, shortcut: "F2", onClick: () => rename(single) });
    if (adapter.chmod) items.push({ label: t("pane.chmod"), icon: <ShieldCheck size={14} />, onClick: () => chmod(list) });
    if (single) items.push({ label: t("common.copyPath"), icon: <Copy size={14} />, onClick: () => copyPath(single.path) });
    items.push({ separator: true });
    items.push({
      label: t("common.delete"),
      icon: <Trash2 size={14} />,
      shortcut: "Del",
      danger: true,
      onClick: () => remove(list),
    });
    openContextMenu(ev.clientX, ev.clientY, items);
  };

  const backgroundMenu = (ev: React.MouseEvent) => {
    ev.preventDefault();
    setSelected(new Set());
    openContextMenu(ev.clientX, ev.clientY, [
      { label: t("pane.newFolder"), icon: <FolderPlus size={14} />, onClick: newFolder },
      { label: t("pane.newFile"), icon: <FilePlus2 size={14} />, onClick: newFile },
      { separator: true },
      { label: t("common.refresh"), icon: <RefreshCw size={14} />, shortcut: "F5", onClick: () => load({ keepSelection: true }) },
      {
        label: t("pane.showHidden"),
        icon: settings.showHidden ? <EyeOff size={14} /> : <Eye size={14} />,
        onClick: () => updateSettings({ showHidden: !settings.showHidden }),
      },
      { label: t("common.copyPath"), icon: <Copy size={14} />, onClick: () => copyPath(path) },
    ]);
  };

  const placesMenu = (ev: React.MouseEvent) => {
    const rect = (ev.currentTarget as HTMLElement).getBoundingClientRect();
    openContextMenu(
      rect.left,
      rect.bottom + 4,
      (adapter.places ?? []).map((p) => ({
        label: t(`places.${p.id}` as TKey),
        icon: <MapPin size={14} />,
        onClick: () => navigate(p.path),
      })),
    );
  };

  // ---------------------------------------------------------------- drag & drop (pointer based)
  const dragStart = useRef<{ x: number; y: number; idx: number; entry: FileEntry } | null>(null);

  const onRowPointerDown = (e: React.PointerEvent, entry: FileEntry, idx: number) => {
    if (e.button !== 0 || mobile || e.pointerType === "touch") return;
    dragStart.current = { x: e.clientX, y: e.clientY, idx, entry };
    const onMove = (ev: PointerEvent) => {
      const start = dragStart.current;
      if (!start) return;
      const store = useStore.getState();
      if (!store.drag) {
        if (Math.hypot(ev.clientX - start.x, ev.clientY - start.y) < 6) return;
        const list = selected.has(start.entry.path) ? selectedEntries : [start.entry];
        if (!selected.has(start.entry.path)) {
          setSelected(new Set([start.entry.path]));
          setAnchor(start.idx);
          setCursor(start.idx);
        }
        document.body.classList.add("dragging");
        store.setDrag({ source: adapter.side, sessionId, entries: list, x: ev.clientX, y: ev.clientY });
      } else {
        store.setDrag({ ...store.drag, x: ev.clientX, y: ev.clientY });
      }
    };
    const onUp = (ev: PointerEvent) => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
      const store = useStore.getState();
      const drag = store.drag;
      dragStart.current = null;
      document.body.classList.remove("dragging");
      if (!drag) return;
      store.setDrag(null);
      const el = document.elementFromPoint(ev.clientX, ev.clientY) as HTMLElement | null;
      const paneEl = el?.closest<HTMLElement>("[data-drop-side]");
      if (!paneEl) return;
      const dirEl = el?.closest<HTMLElement>("[data-drop-dir]");
      const dir = dirEl?.dataset.dropDir ?? paneEl.dataset.dropPath ?? "";
      paneEl.dispatchEvent(new CustomEvent<DropDetail>("sftpinguin-drop", { detail: { drag, dir } }));
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
  };

  // receive drops (from the other pane, from this pane or from the OS)
  useEffect(() => {
    const el = rootRef.current;
    if (!el) return;
    const onInternal = (ev: Event) => {
      const { drag, dir } = (ev as CustomEvent<DropDetail>).detail;
      if (drag.source === adapter.side) {
        if (drag.sessionId === sessionId || adapter.side === "local") {
          if (dir !== path || drag.entries.some((x) => adapter.parent(x.path) !== dir)) moveInto(drag.entries, dir);
        }
        return;
      }
      if (drag.sessionId !== sessionId && adapter.side === "remote") return;
      onDrop(drag, dir || path);
    };
    const onOs = (ev: Event) => {
      const { paths, dir } = (ev as CustomEvent<{ paths: string[]; dir: string }>).detail;
      onOsDrop?.(paths, dir || path);
    };
    const onOsHover = (ev: Event) => setOsDropHover((ev as CustomEvent<boolean>).detail);
    el.addEventListener("sftpinguin-drop", onInternal);
    el.addEventListener("sftpinguin-osdrop", onOs);
    el.addEventListener("sftpinguin-oshover", onOsHover);
    return () => {
      el.removeEventListener("sftpinguin-drop", onInternal);
      el.removeEventListener("sftpinguin-osdrop", onOs);
      el.removeEventListener("sftpinguin-oshover", onOsHover);
    };
  });

  // ---------------------------------------------------------------- render
  const sortHeader = (key: SortKey, label: string, cls: string) => (
    <button
      className={`th ${cls} ${sort.key === key ? "sorted" : ""}`}
      onClick={() => setSort((s) => ({ key, asc: s.key === key ? !s.asc : true }))}
    >
      {label}
      {sort.key === key && (sort.asc ? <ArrowUp size={11} /> : <ArrowDown size={11} />)}
    </button>
  );

  const showPerms = adapter.side === "remote" ? !!adapter.chmod : !mobile && entries.some((e) => e.mode !== null);

  return (
    <section
      ref={rootRef}
      className={`pane ${props.className ?? ""} ${osDropHover ? "os-drop" : ""}`}
      data-drop-side={adapter.side}
      data-drop-path={path}
      tabIndex={0}
      onKeyDown={onKeyDown}
    >
      <header className="pane-header">
        <div className="pane-title">
          {props.titleIcon}
          <span>{props.title}</span>
        </div>
        <div className="pane-tools">
          {props.headerExtra}
          <button className={`icon-btn ${filterOpen ? "on" : ""}`} title={t("pane.filter")} onClick={() => setFilterOpen(!filterOpen)}>
            <Search size={15} />
          </button>
          <button className="icon-btn" title={t("pane.newFolder")} onClick={newFolder} disabled={!connected}>
            <FolderPlus size={15} />
          </button>
          <button className="icon-btn" title={`${t("common.refresh")} (F5)`} onClick={() => load({ keepSelection: true })} disabled={!connected}>
            <RefreshCw size={15} className={loading ? "spin" : ""} />
          </button>
        </div>
      </header>

      <div className="pathbar">
        <button className="icon-btn" title={t("pane.up")} onClick={goUp} disabled={isRoot}>
          <CornerLeftUp size={15} />
        </button>
        <button className="icon-btn" title={t("pane.home")} onClick={() => navigate(adapter.home)}>
          <Home size={15} />
        </button>
        {adapter.places && adapter.places.length > 0 && (
          <button className="icon-btn" title={t("pane.places")} onClick={placesMenu}>
            <MapPin size={15} />
          </button>
        )}
        {editingPath ? (
          <input
            className="input path-input"
            autoFocus
            value={pathInput}
            spellCheck={false}
            autoCapitalize="off"
            onChange={(e) => setPathInput(e.target.value)}
            onBlur={() => setEditingPath(false)}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                setEditingPath(false);
                if (pathInput.trim()) navigate(pathInput.trim());
              } else if (e.key === "Escape") {
                setEditingPath(false);
                setPathInput(path);
              }
            }}
          />
        ) : (
          <div className="crumbs" onClick={(e) => e.target === e.currentTarget && setEditingPath(true)} title={t("pane.goTo")}>
            {adapter.crumbs(path).map((c, i, all) => (
              <span key={c.path} className="crumb-wrap" data-drop-dir={c.path}>
                <button className={`crumb ${i === all.length - 1 ? "current" : ""}`} onClick={() => navigate(c.path)}>
                  {c.label}
                </button>
                {i < all.length - 1 && c.label !== "/" && <ChevronRight size={12} className="crumb-sep" />}
              </span>
            ))}
          </div>
        )}
      </div>

      {filterOpen && (
        <div className="filterbar">
          <Search size={13} />
          <input
            autoFocus
            value={filter}
            placeholder={t("pane.filter")}
            onChange={(e) => setFilter(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Escape") {
                setFilter("");
                setFilterOpen(false);
                rootRef.current?.focus();
              }
            }}
          />
          <button
            className="icon-btn"
            onClick={() => {
              setFilter("");
              setFilterOpen(false);
            }}
          >
            <X size={13} />
          </button>
        </div>
      )}

      <div className={`thead ${showPerms ? "with-perms" : ""}`}>
        {sortHeader("name", t("common.name"), "c-name")}
        {sortHeader("size", t("common.size"), "c-size")}
        {sortHeader("modified", t("common.modified"), "c-date")}
        {showPerms && <span className="th c-perm">{t("common.permissions")}</span>}
      </div>

      <div className="rows" ref={listRef} onContextMenu={backgroundMenu} onClick={(e) => e.target === e.currentTarget && setSelected(new Set())}>
        {!connected ? (
          <div className="pane-empty">
            <PlugZap size={28} />
            <p>{t("pane.notConnected")}</p>
            {props.onReconnect && (
              <button className="btn btn-primary" onClick={props.onReconnect}>
                {t("pane.reconnect")}
              </button>
            )}
          </div>
        ) : error ? (
          <div className="pane-empty error">
            <AlertCircle size={26} />
            <p>{error}</p>
            <div className="row-actions">
              {!isRoot && (
                <button className="btn" onClick={goUp}>
                  {t("pane.up")}
                </button>
              )}
              <button className="btn" onClick={() => load()}>
                {t("common.refresh")}
              </button>
            </div>
          </div>
        ) : loading && entries.length === 0 ? (
          <div className="pane-empty">
            <Loader2 size={22} className="spin" />
          </div>
        ) : visible.length === 0 ? (
          <div className="pane-empty muted" onContextMenu={backgroundMenu}>
            <p>{filter ? "—" : t("common.empty")}</p>
          </div>
        ) : (
          visible.map((e, idx) => (
            <div
              key={e.path}
              data-index={idx}
              data-drop-dir={e.kind === "dir" ? e.path : undefined}
              className={`row ${selected.has(e.path) ? "selected" : ""} ${cursor === idx ? "cursor" : ""} ${e.kind} ${isHidden(e.name) ? "hidden-file" : ""} ${showPerms ? "with-perms" : ""}`}
              onPointerDown={(ev) => onRowPointerDown(ev, e, idx)}
              onClick={(ev) => {
                if (mobile && e.kind === "dir" && selected.size === 0) return navigate(e.path);
                selectIndex(idx, ev);
              }}
              onDoubleClick={() => openEntry(e)}
              onContextMenu={(ev) => rowMenu(ev, e, idx)}
            >
              <span className="c-name">
                <FileIcon entry={e} />
                <span className="fname">{e.name}</span>
                {e.isLink && e.linkTarget && <span className="link-target">→ {e.linkTarget}</span>}
              </span>
              <span className="c-size">{e.kind === "file" ? formatSize(e.size, lang) : ""}</span>
              <span className="c-date">{formatDate(e.modified, lang)}</span>
              {showPerms && <span className="c-perm mono">{modeString(e.mode)}</span>}
            </div>
          ))
        )}
      </div>

      <footer className="pane-status">
        <span className="muted">
          {selectedEntries.length > 0
            ? `${t("pane.selected", { count: selectedEntries.length })}${selectedSize ? ` · ${formatSize(selectedSize, lang)}` : ""}`
            : visible.length === 1
              ? t("pane.itemsOne")
              : t("pane.items", { count: visible.length })}
        </span>
        <div className="spacer" />
        {selectedEntries.length > 0 && (
          <>
            {mobile && (
              <button className="btn btn-sm" onClick={() => remove(selectedEntries)}>
                <Trash2 size={13} />
              </button>
            )}
            <button className="btn btn-sm btn-primary" onClick={() => onTransfer(selectedEntries)}>
              {props.transferIcon} {props.transferLabel}
            </button>
          </>
        )}
        {mobile && selectedEntries.length === 0 && (
          <button className="btn btn-sm" onClick={newFile}>
            <FilePlus2 size={13} />
          </button>
        )}
      </footer>
      {loading && entries.length > 0 && <div className="pane-progress" />}
    </section>
  );
}
