import { useEffect, useMemo, useRef, useState } from "react";
import { ArrowLeft, ArrowRight, HardDrive, Server } from "lucide-react";
import { api, FileEntry, Place } from "../lib/api";
import { useT } from "../lib/i18n";
import { DragState, Tab, useStore } from "../lib/store";
import { downloadEntries, openRemote, reconnectTab, showError, uploadEntries, uploadPaths } from "../lib/actions";
import { lcrumbs, ljoin, lparent, rcrumbs, rjoin, rparent } from "../lib/format";
import { FilePane, PaneAdapter } from "./FilePane";

export function SessionView({ tab, visible }: { tab: Tab; visible: boolean }) {
  const t = useT();
  const sessionId = tab.info.id;
  const platform = useStore((s) => s.platform);
  const useTrash = useStore((s) => s.settings.useTrash);
  const splitRatio = useStore((s) => s.settings.splitRatio);
  const updateSettings = useStore((s) => s.updateSettings);
  const localVersion = useStore((s) => s.localVersion);
  const remoteVersion = useStore((s) => s.remoteVersion[sessionId] ?? 0);
  const mobile = !!platform?.mobile;
  const narrow = useNarrow();
  const single = mobile || narrow;

  const [local, setLocal] = useState<{ path: string; home: string; places: Place[] } | null>(null);
  const [remotePath, setRemotePath] = useState(tab.info.home);
  const [mobilePane, setMobilePane] = useState<"remote" | "local">("remote");
  const localEntries = useRef<FileEntry[]>([]);
  const remoteEntries = useRef<FileEntry[]>([]);
  const splitRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    (async () => {
      const [home, places] = await Promise.all([api.localDefaultDir(), api.localPlaces()]);
      let start = tab.info.localPath || home;
      if (tab.info.localPath) {
        const exists = await api.localStat(tab.info.localPath).catch(() => null);
        if (!exists) start = home;
      }
      setLocal({ path: start, home, places });
    })().catch(showError);
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  const isWindows = platform?.os === "windows";

  const localAdapter = useMemo<PaneAdapter | null>(() => {
    if (!local) return null;
    const trash = useTrash && !mobile;
    return {
      side: "local",
      list: api.listLocal,
      mkdir: api.localMkdir,
      createFile: api.localCreateFile,
      rename: api.localRename,
      remove: (entries) => api.localDelete(entries.map((e) => e.path), trash),
      chmod: isWindows || mobile ? undefined : (entries, mode) => api.localChmod(entries.map((e) => e.path), mode),
      join: ljoin,
      parent: lparent,
      crumbs: (p) => lcrumbs(p, isWindows),
      home: local.home,
      places: local.places,
      trashHint: trash,
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [local?.home, local?.places, useTrash, mobile, isWindows]);

  const remoteAdapter = useMemo<PaneAdapter>(
    () => ({
      side: "remote",
      list: (p) => api.listRemote(sessionId, p),
      mkdir: (p) => api.remoteMkdir(sessionId, p),
      createFile: (p) => api.remoteCreateFile(sessionId, p),
      rename: (a, b) => api.remoteRename(sessionId, a, b),
      remove: (entries) => api.remoteDelete(sessionId, entries.map((e) => ({ path: e.path, isDir: e.kind === "dir" }))),
      chmod: tab.info.capabilities.chmod
        ? (entries, mode, recursive) =>
            api.remoteChmod(sessionId, entries.map((e) => ({ path: e.path, isDir: e.kind === "dir" })), mode, recursive)
        : undefined,
      join: rjoin,
      parent: rparent,
      crumbs: rcrumbs,
      home: tab.info.home,
    }),
    [sessionId, tab.info.home, tab.info.capabilities.chmod],
  );

  const upload = async (entries: FileEntry[], dir: string) => {
    const existing = dir === remotePath ? remoteEntries.current : await api.listRemote(sessionId, dir).catch(() => []);
    uploadEntries(sessionId, entries, dir, existing);
  };

  const download = async (entries: FileEntry[], dir: string) => {
    if (!local) return;
    const existing = dir === local.path ? localEntries.current : await api.listLocal(dir).catch(() => []);
    downloadEntries(sessionId, entries, dir, existing);
  };

  const startSplitDrag = (e: React.PointerEvent) => {
    e.preventDefault();
    const box = splitRef.current?.getBoundingClientRect();
    if (!box) return;
    const move = (ev: PointerEvent) => {
      const ratio = Math.min(0.8, Math.max(0.2, (ev.clientX - box.left) / box.width));
      updateSettings({ splitRatio: ratio });
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      document.body.classList.remove("resizing");
    };
    document.body.classList.add("resizing");
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  };

  const localPane = local && localAdapter && (
    <FilePane
      key="local"
      adapter={localAdapter}
      sessionId={sessionId}
      path={local.path}
      onNavigate={(p) => setLocal((l) => (l ? { ...l, path: p } : l))}
      version={localVersion}
      connected
      onTransfer={(entries) => upload(entries, remotePath)}
      onOpen={(e) => api.openLocalPath(e.path).catch(showError)}
      onDrop={(drag: DragState, dir) => download(drag.entries, dir)}
      onEntries={(e) => (localEntries.current = e)}
      title={t("pane.local")}
      titleIcon={<HardDrive size={15} />}
      transferLabel={t("pane.upload")}
      transferIcon={<ArrowRight size={14} />}
      className={single && mobilePane !== "local" ? "hidden" : ""}
    />
  );

  const remotePane = (
    <FilePane
      key="remote"
      adapter={remoteAdapter}
      sessionId={sessionId}
      path={remotePath}
      onNavigate={setRemotePath}
      version={remoteVersion}
      connected={tab.connected}
      onReconnect={() => reconnectTab(tab.id)}
      onTransfer={(entries) => local && download(entries, local.path)}
      onOpen={(e) => openRemote(sessionId, e)}
      onDrop={(drag: DragState, dir) => upload(drag.entries, dir)}
      onOsDrop={(paths, dir) => uploadPaths(sessionId, paths, dir, dir === remotePath ? remoteEntries.current : null)}
      onEntries={(e) => (remoteEntries.current = e)}
      title={tab.info.title}
      titleIcon={<Server size={15} />}
      transferLabel={t("pane.download")}
      transferIcon={<ArrowLeft size={14} />}
      className={single && mobilePane !== "remote" ? "hidden" : ""}
    />
  );

  return (
    <div className={`session ${visible ? "" : "hidden"} ${single ? "single" : ""}`}>
      {single && (
        <div className="segmented pane-switch">
          <button className={mobilePane === "remote" ? "active" : ""} onClick={() => setMobilePane("remote")}>
            <Server size={14} /> {t("pane.remote")}
          </button>
          <button className={mobilePane === "local" ? "active" : ""} onClick={() => setMobilePane("local")}>
            <HardDrive size={14} /> {t("pane.local")}
          </button>
        </div>
      )}
      <div
        className="split"
        ref={splitRef}
        style={single ? undefined : { gridTemplateColumns: `${splitRatio}fr 6px ${1 - splitRatio}fr` }}
      >
        {localPane ?? <div className={`pane ${single && mobilePane !== "local" ? "hidden" : ""}`} />}
        {!single && <div className="splitter" onPointerDown={startSplitDrag} onDoubleClick={() => updateSettings({ splitRatio: 0.5 })} />}
        {remotePane}
      </div>
    </div>
  );
}

function useNarrow() {
  const query = "(max-width: 760px)";
  const [narrow, setNarrow] = useState(() => window.matchMedia(query).matches);
  useEffect(() => {
    const mq = window.matchMedia(query);
    const on = () => setNarrow(mq.matches);
    mq.addEventListener("change", on);
    return () => mq.removeEventListener("change", on);
  }, []);
  return narrow;
}

