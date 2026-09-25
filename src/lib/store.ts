import { create } from "zustand";
import type {
  ConflictPolicy,
  FileEntry,
  LogEntry,
  PlatformInfo,
  SessionInfo,
  Site,
  TransferInfo,
  TransferProgress,
} from "./api";

export interface Settings {
  language: "auto" | "de" | "en";
  theme: "system" | "light" | "dark";
  accent: string;
  showHidden: boolean;
  confirmDelete: boolean;
  useTrash: boolean;
  doubleClick: "transfer" | "open";
  maxConcurrent: number;
  conflict: "ask" | ConflictPolicy;
  preserveMtime: boolean;
  panelOpen: boolean;
  panelHeight: number;
  splitRatio: number;
}

const DEFAULT_SETTINGS: Settings = {
  language: "auto",
  theme: "system",
  accent: "indigo",
  showHidden: false,
  confirmDelete: true,
  useTrash: true,
  doubleClick: "transfer",
  maxConcurrent: 3,
  conflict: "ask",
  preserveMtime: true,
  panelOpen: false,
  panelHeight: 220,
  splitRatio: 0.5,
};

const SETTINGS_KEY = "sftpinguin.settings";

function loadSettings(): Settings {
  try {
    const raw = localStorage.getItem(SETTINGS_KEY);
    if (raw) return { ...DEFAULT_SETTINGS, ...JSON.parse(raw) };
  } catch {
    /* ignore */
  }
  return { ...DEFAULT_SETTINGS };
}

export interface Tab {
  id: string;
  info: SessionInfo;
  connected: boolean;
}

export interface Toast {
  id: number;
  kind: "info" | "success" | "error";
  message: string;
  action?: { label: string; run: () => void };
  secondary?: { label: string; run: () => void };
  sticky?: boolean;
}

export interface DragState {
  source: "local" | "remote";
  sessionId: string;
  entries: FileEntry[];
  x: number;
  y: number;
}

interface State {
  platform: PlatformInfo | null;
  sites: Site[];
  tabs: Tab[];
  activeTab: string | null;
  transfers: Record<string, TransferInfo>;
  transferOrder: string[];
  logs: LogEntry[];
  settings: Settings;
  toasts: Toast[];
  connecting: Record<string, string>;
  drag: DragState | null;
  sidebarOpen: boolean;
  /** Bumped when transfers finished so panes can refresh */
  localVersion: number;
  remoteVersion: Record<string, number>;

  setPlatform: (p: PlatformInfo) => void;
  setSites: (s: Site[]) => void;
  upsertSite: (s: Site) => void;
  removeSite: (id: string) => void;
  addTab: (info: SessionInfo) => void;
  closeTab: (id: string) => void;
  setActiveTab: (id: string | null) => void;
  setTabConnected: (id: string, connected: boolean, info?: SessionInfo) => void;
  replaceTabSession: (oldId: string, info: SessionInfo) => void;
  setTransfers: (list: TransferInfo[]) => void;
  upsertTransfers: (list: TransferInfo[]) => void;
  applyProgress: (list: TransferProgress[]) => void;
  pruneTransfers: (keep: (t: TransferInfo) => boolean) => void;
  addLog: (e: LogEntry) => void;
  clearLogs: () => void;
  updateSettings: (patch: Partial<Settings>) => void;
  toast: (t: Omit<Toast, "id">) => number;
  dismissToast: (id: number) => void;
  setConnecting: (key: string, label: string | null) => void;
  setDrag: (d: DragState | null) => void;
  setSidebarOpen: (v: boolean) => void;
  bumpLocal: () => void;
  bumpRemote: (sessionId: string) => void;
}

let toastId = 1;

export const useStore = create<State>((set, get) => ({
  platform: null,
  sites: [],
  tabs: [],
  activeTab: null,
  transfers: {},
  transferOrder: [],
  logs: [],
  settings: loadSettings(),
  toasts: [],
  connecting: {},
  drag: null,
  sidebarOpen: false,
  localVersion: 0,
  remoteVersion: {},

  setPlatform: (platform) => set({ platform }),
  setSites: (sites) => set({ sites }),
  upsertSite: (site) =>
    set((s) => {
      const idx = s.sites.findIndex((x) => x.id === site.id);
      const sites = [...s.sites];
      if (idx >= 0) sites[idx] = site;
      else sites.push(site);
      return { sites };
    }),
  removeSite: (id) => set((s) => ({ sites: s.sites.filter((x) => x.id !== id) })),

  addTab: (info) =>
    set((s) => ({
      tabs: [...s.tabs, { id: info.id, info, connected: true }],
      activeTab: info.id,
    })),
  closeTab: (id) =>
    set((s) => {
      const idx = s.tabs.findIndex((t) => t.id === id);
      const tabs = s.tabs.filter((t) => t.id !== id);
      let activeTab = s.activeTab;
      if (activeTab === id) {
        activeTab = tabs.length ? tabs[Math.max(0, idx - 1)].id : null;
      }
      return { tabs, activeTab };
    }),
  setActiveTab: (activeTab) => set({ activeTab, sidebarOpen: false }),
  setTabConnected: (id, connected, info) =>
    set((s) => ({
      tabs: s.tabs.map((t) => (t.id === id ? { ...t, connected, info: info ?? t.info } : t)),
    })),
  replaceTabSession: (oldId, info) =>
    set((s) => ({
      tabs: s.tabs.map((t) => (t.id === oldId ? { id: t.id, info, connected: true } : t)),
    })),

  setTransfers: (list) =>
    set({
      transfers: Object.fromEntries(list.map((t) => [t.id, t])),
      transferOrder: list.map((t) => t.id),
    }),
  upsertTransfers: (list) =>
    set((s) => {
      const transfers = { ...s.transfers };
      const order = [...s.transferOrder];
      for (const t of list) {
        if (!transfers[t.id]) order.push(t.id);
        transfers[t.id] = t;
      }
      return { transfers, transferOrder: order };
    }),
  applyProgress: (list) =>
    set((s) => {
      const transfers = { ...s.transfers };
      for (const p of list) {
        const t = transfers[p.id];
        if (t) transfers[p.id] = { ...t, transferred: p.transferred, size: p.size || t.size, speed: p.speed };
      }
      return { transfers };
    }),
  pruneTransfers: (keep) =>
    set((s) => {
      const transfers: Record<string, TransferInfo> = {};
      const order = s.transferOrder.filter((id) => s.transfers[id] && keep(s.transfers[id]));
      for (const id of order) transfers[id] = s.transfers[id];
      return { transfers, transferOrder: order };
    }),

  addLog: (e) => set((s) => ({ logs: [...s.logs.slice(-499), e] })),
  clearLogs: () => set({ logs: [] }),

  updateSettings: (patch) => {
    const settings = { ...get().settings, ...patch };
    try {
      localStorage.setItem(SETTINGS_KEY, JSON.stringify(settings));
    } catch {
      /* ignore */
    }
    set({ settings });
  },

  toast: (t) => {
    const id = toastId++;
    set((s) => ({ toasts: [...s.toasts.slice(-4), { ...t, id }] }));
    if (!t.sticky) {
      setTimeout(() => get().dismissToast(id), t.kind === "error" ? 7000 : 4000);
    }
    return id;
  },
  dismissToast: (id) => set((s) => ({ toasts: s.toasts.filter((x) => x.id !== id) })),

  setConnecting: (key, label) =>
    set((s) => {
      const connecting = { ...s.connecting };
      if (label === null) delete connecting[key];
      else connecting[key] = label;
      return { connecting };
    }),
  setDrag: (drag) => set({ drag }),
  setSidebarOpen: (sidebarOpen) => set({ sidebarOpen }),
  bumpLocal: () => set((s) => ({ localVersion: s.localVersion + 1 })),
  bumpRemote: (sessionId) =>
    set((s) => ({ remoteVersion: { ...s.remoteVersion, [sessionId]: (s.remoteVersion[sessionId] ?? 0) + 1 } })),
}));
