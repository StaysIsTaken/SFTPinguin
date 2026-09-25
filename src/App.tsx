import { useEffect, useState } from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { Files, Loader2, Menu, Plus, X } from "lucide-react";
import { protocolLabel } from "./lib/api";
import { useT } from "./lib/i18n";
import { useStore } from "./lib/store";
import { closeTab, initBackend, showError } from "./lib/actions";
import { DialogHost } from "./lib/dialogs";
import { Sidebar, newSite } from "./components/Sidebar";
import { Welcome } from "./components/Welcome";
import { SessionView } from "./components/SessionView";
import { TransferPanel } from "./components/TransferPanel";
import { Toasts } from "./components/Toasts";
import { ContextMenuHost } from "./components/ContextMenu";
import { openSettings } from "./components/Settings";

function useTheme() {
  const theme = useStore((s) => s.settings.theme);
  const accent = useStore((s) => s.settings.accent);
  useEffect(() => {
    const root = document.documentElement;
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const apply = () => {
      root.dataset.theme = theme === "system" ? (mq.matches ? "dark" : "light") : theme;
    };
    apply();
    root.dataset.accent = accent;
    mq.addEventListener("change", apply);
    return () => mq.removeEventListener("change", apply);
  }, [theme, accent]);
}

/** Files dragged in from the operating system (desktop only). */
function useOsDrop() {
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let hovered: HTMLElement | null = null;
    const setHover = (el: HTMLElement | null) => {
      if (hovered === el) return;
      hovered?.dispatchEvent(new CustomEvent("sftpinguin-oshover", { detail: false }));
      hovered = el;
      hovered?.dispatchEvent(new CustomEvent("sftpinguin-oshover", { detail: true }));
    };
    const paneAt = (x: number, y: number) => {
      const ratio = window.devicePixelRatio || 1;
      const el = document.elementFromPoint(x / ratio, y / ratio) as HTMLElement | null;
      const pane = el?.closest<HTMLElement>('[data-drop-side="remote"]') ?? null;
      const dir = el?.closest<HTMLElement>("[data-drop-dir]")?.dataset.dropDir;
      return { pane, dir };
    };
    getCurrentWebview()
      .onDragDropEvent((event) => {
        const p = event.payload;
        if (p.type === "over") {
          setHover(paneAt(p.position.x, p.position.y).pane);
        } else if (p.type === "drop") {
          const { pane, dir } = paneAt(p.position.x, p.position.y);
          setHover(null);
          if (pane && p.paths.length) {
            pane.dispatchEvent(
              new CustomEvent("sftpinguin-osdrop", { detail: { paths: p.paths, dir: dir ?? pane.dataset.dropPath } }),
            );
          }
        } else {
          setHover(null);
        }
      })
      .then((fn) => (unlisten = fn))
      .catch(() => {});
    return () => unlisten?.();
  }, []);
}

function DragGhost() {
  const drag = useStore((s) => s.drag);
  const t = useT();
  if (!drag) return null;
  const label = drag.entries.length === 1 ? drag.entries[0].name : `${drag.entries.length} ${t("pane.items", { count: "" }).trim()}`;
  return (
    <div className="drag-ghost" style={{ left: drag.x + 14, top: drag.y + 10 }}>
      <Files size={14} /> {label}
    </div>
  );
}

function TabBar() {
  const t = useT();
  const tabs = useStore((s) => s.tabs);
  const active = useStore((s) => s.activeTab);
  const setActive = useStore((s) => s.setActiveTab);
  const sites = useStore((s) => s.sites);
  const connecting = useStore((s) => s.connecting);
  const setSidebarOpen = useStore((s) => s.setSidebarOpen);
  const activeTab = tabs.find((x) => x.id === active);

  return (
    <div className="tabbar">
      <button className="icon-btn mobile-only" onClick={() => setSidebarOpen(true)}>
        <Menu size={18} />
      </button>
      <div className="tabs">
        {tabs.map((tab) => {
          const site = sites.find((s) => s.id === tab.info.siteId);
          return (
            <div
              key={tab.id}
              className={`tab ${tab.id === active ? "active" : ""} ${tab.connected ? "" : "disconnected"}`}
              onClick={() => setActive(tab.id)}
              onMouseDown={(e) => {
                if (e.button === 1) {
                  e.preventDefault();
                  closeTab(tab.id);
                }
              }}
              title={`${protocolLabel(tab.info.protocol)} · ${tab.info.username ? tab.info.username + "@" : ""}${tab.info.host}`}
            >
              <span className="tab-dot" style={site?.color ? { background: site.color } : undefined} />
              <span className="tab-title">{tab.info.title}</span>
              <button
                className="tab-close"
                onClick={(e) => {
                  e.stopPropagation();
                  closeTab(tab.id);
                }}
                title={t("common.disconnect")}
              >
                <X size={12} />
              </button>
            </div>
          );
        })}
        {Object.entries(connecting).map(([key, label]) => (
          <div key={`c-${key}`} className="tab connecting">
            <Loader2 size={12} className="spin" />
            <span className="tab-title">{label}</span>
          </div>
        ))}
        <button className={`tab-new ${active === null ? "active" : ""}`} onClick={() => setActive(null)} title={t("sidebar.quickConnect")}>
          <Plus size={15} />
        </button>
      </div>
      <span className="mobile-only mobile-title">{activeTab?.info.title ?? "SFTPinguin"}</span>
    </div>
  );
}

export default function App() {
  const tabs = useStore((s) => s.tabs);
  const active = useStore((s) => s.activeTab);
  const [ready, setReady] = useState(false);
  useTheme();
  useOsDrop();

  useEffect(() => {
    initBackend()
      .catch(showError)
      .finally(() => setReady(true));
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const mod = e.ctrlKey || e.metaKey;
      if (!mod) return;
      const s = useStore.getState();
      if (e.key === ",") {
        e.preventDefault();
        openSettings();
      } else if (e.key.toLowerCase() === "n" && !e.shiftKey) {
        e.preventDefault();
        newSite();
      } else if (e.key.toLowerCase() === "t") {
        e.preventDefault();
        s.setActiveTab(null);
      } else if (e.key.toLowerCase() === "w" && s.activeTab) {
        e.preventDefault();
        closeTab(s.activeTab);
      } else if (e.key === "Tab" && s.tabs.length > 1) {
        e.preventDefault();
        const idx = s.tabs.findIndex((x) => x.id === s.activeTab);
        const next = (idx + (e.shiftKey ? -1 : 1) + s.tabs.length) % s.tabs.length;
        s.setActiveTab(s.tabs[next].id);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // Disable the native browser context menu outside of inputs
  useEffect(() => {
    const onCtx = (e: MouseEvent) => {
      const el = e.target as HTMLElement;
      if (!el.closest("input, textarea, .selectable")) e.preventDefault();
    };
    window.addEventListener("contextmenu", onCtx);
    return () => window.removeEventListener("contextmenu", onCtx);
  }, []);

  return (
    <div className="app">
      <Sidebar onQuickConnect={() => useStore.getState().setActiveTab(null)} />
      <main className="main">
        <TabBar />
        <div className="content">
          {!ready ? null : active === null ? <Welcome /> : null}
          {tabs.map((tab) => (
            <SessionView key={tab.id} tab={tab} visible={tab.id === active} />
          ))}
        </div>
        <TransferPanel />
      </main>
      <DragGhost />
      <ContextMenuHost />
      <DialogHost />
      <Toasts />
    </div>
  );
}
