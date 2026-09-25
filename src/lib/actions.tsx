import { listen } from "@tauri-apps/api/event";
import { CalendarClock, CopyPlus, Replace, ShieldAlert, ShieldQuestion, SkipForward } from "lucide-react";
import {
  api,
  asAppError,
  AppError,
  ConflictPolicy,
  EditedFile,
  FileEntry,
  HostKey,
  LogEntry,
  SessionInfo,
  Site,
  TransferInfo,
  TransferProgress,
  TransferRequest,
} from "./api";
import { choiceDialog, Modal, openDialog, passwordDialog } from "./dialogs";
import { t, TKey } from "./i18n";
import { useStore } from "./store";
import { lname, ljoin, rjoin } from "./format";

export function errorText(e: unknown): string {
  const err = asAppError(e);
  const key = `error.${err.code}` as TKey;
  const prefix = t(key);
  if (prefix !== key && !err.message.toLowerCase().includes(prefix.toLowerCase())) {
    return `${prefix}: ${err.message}`;
  }
  return err.message;
}

export function showError(e: unknown) {
  const err = asAppError(e);
  if (err.code === "cancelled") return;
  useStore.getState().toast({ kind: "error", message: errorText(e) });
}

export async function reloadSites() {
  const sites = await api.listSites();
  useStore.getState().setSites(sites);
}

export function syncTransferOptions() {
  const s = useStore.getState().settings;
  api.setTransferOptions(s.maxConcurrent, s.preserveMtime).catch(() => {});
}

// ---------------------------------------------------------------------------
// Connecting
// ---------------------------------------------------------------------------

function hostKeyDialog(kind: "unknown" | "changed", presented: HostKey, previous?: HostKey): Promise<boolean> {
  const hostLabel = presented.port === 22 ? presented.host : `${presented.host}:${presented.port}`;
  return openDialog<boolean>((done) => (
    <Modal
      title={kind === "unknown" ? t("hostkey.unknownTitle") : t("hostkey.changedTitle")}
      onClose={() => done(false)}
      danger={kind === "changed"}
      width={520}
      icon={
        kind === "unknown" ? (
          <ShieldQuestion size={20} className="text-accent" />
        ) : (
          <ShieldAlert size={20} className="text-danger" />
        )
      }
      footer={
        <>
          <button className="btn" onClick={() => done(false)} autoFocus={kind === "changed"}>
            {t("common.cancel")}
          </button>
          <button
            className={`btn ${kind === "changed" ? "btn-danger" : "btn-primary"}`}
            onClick={() => done(true)}
            autoFocus={kind === "unknown"}
          >
            {kind === "unknown" ? t("hostkey.trust") : t("hostkey.replace")}
          </button>
        </>
      }
    >
      <p className="dialog-message">
        {t(kind === "unknown" ? "hostkey.unknownText" : "hostkey.changedText", { host: hostLabel })}
      </p>
      <dl className="kv">
        <dt>{t("hostkey.type")}</dt>
        <dd>{presented.keyType}</dd>
        <dt>{t("hostkey.fingerprint")}</dt>
        <dd className="mono selectable">{presented.fingerprint}</dd>
        {previous && (
          <>
            <dt>{t("hostkey.previous")}</dt>
            <dd className="mono selectable muted">{previous.fingerprint}</dd>
          </>
        )}
      </dl>
    </Modal>
  ));
}

/**
 * Connects to a saved site (siteId) or an ad-hoc site. Handles host key
 * confirmation and password prompts. Returns null if the user aborted.
 */
export async function connectTo(target: {
  siteId?: string;
  site?: Site;
  password?: string;
}): Promise<SessionInfo | null> {
  const store = useStore.getState();
  const site = target.siteId ? store.sites.find((s) => s.id === target.siteId) : target.site;
  if (!site) return null;
  const key = target.siteId ?? "quick";
  const name = site.name || site.host;
  if (store.connecting[key]) return null;
  store.setConnecting(key, name);

  let password: string | null = target.password || null;
  let remember = site.savePassword;
  let lastError: AppError | null = null;
  try {
    for (let attempt = 0; attempt < 6; attempt++) {
      try {
        const info = await api.connect({
          siteId: target.siteId ?? null,
          site: target.siteId ? null : site,
          password,
          remember: remember && !!target.siteId,
        });
        if (target.siteId) reloadSites().catch(() => {});
        return info;
      } catch (e) {
        const err = asAppError(e);
        lastError = err;
        if (err.code === "host_key_unknown" || err.code === "host_key_changed") {
          const presented = err.details?.presented as HostKey;
          const ok = await hostKeyDialog(
            err.code === "host_key_unknown" ? "unknown" : "changed",
            presented,
            err.details?.previous,
          );
          if (!ok) return null;
          await api.trustHostKey(presented);
          continue;
        }
        const needsSecret =
          err.code === "password_required" ||
          err.code === "passphrase_required" ||
          (err.code === "auth_failed" && (site.auth === "password" || site.auth === "key"));
        if (needsSecret) {
          const isKey = site.auth === "key";
          const title =
            site.protocol === "s3"
              ? t("connect.secretTitle", { name })
              : isKey
                ? t("connect.passphraseTitle")
                : t("connect.passwordTitle", { name });
          const label = site.protocol === "s3" ? t("site.secretKey") : isKey ? t("site.passphrase") : t("site.password");
          const res = await passwordDialog({
            title,
            label,
            hint:
              err.code === "auth_failed" || (attempt > 0 && err.code === "passphrase_required") ? (
                <span className="text-danger">{attempt > 0 || err.code === "auth_failed" ? t("connect.authFailed") : ""}</span>
              ) : undefined,
            remember: target.siteId && site.savePassword ? remember : undefined,
          });
          if (!res) return null;
          password = res.password;
          remember = res.remember;
          continue;
        }
        throw err;
      }
    }
    if (lastError) throw lastError;
    return null;
  } catch (e) {
    useStore.getState().toast({ kind: "error", message: `${t("connect.failed")}: ${errorText(e)}` });
    return null;
  } finally {
    useStore.getState().setConnecting(key, null);
  }
}

export async function openSession(target: { siteId?: string; site?: Site; password?: string }) {
  const info = await connectTo(target);
  if (info) {
    useStore.getState().addTab(info);
    if (!target.siteId && target.site) quickSites.set(info.id, { site: target.site, password: target.password });
  }
  return info;
}

export async function reconnectTab(tabId: string) {
  const tab = useStore.getState().tabs.find((t) => t.id === tabId);
  if (!tab) return;
  const quick = quickSites.get(tabId);
  const info = await connectTo(tab.info.siteId ? { siteId: tab.info.siteId } : { site: quick?.site, password: quick?.password });
  if (!info) return;
  // keep the tab (and its panes) but point it to the new backend session
  useStore.setState((s) => ({
    tabs: s.tabs.map((x) => (x.id === tabId ? { ...x, info: { ...info }, connected: true } : x)),
  }));
}

/** Ad-hoc (quick connect) sites by tab id, so the tab can reconnect. Kept in memory only. */
const quickSites = new Map<string, { site: Site; password?: string }>();

export async function closeTab(tabId: string) {
  const tab = useStore.getState().tabs.find((t) => t.id === tabId);
  if (tab) {
    await api.disconnect(tab.info.id).catch(() => {});
  }
  quickSites.delete(tabId);
  useStore.getState().closeTab(tabId);
}

// ---------------------------------------------------------------------------
// Transfers
// ---------------------------------------------------------------------------

async function resolvePolicy(conflicts: number): Promise<ConflictPolicy | null> {
  const store = useStore.getState();
  const setting = store.settings.conflict;
  if (setting !== "ask") return setting;
  if (conflicts === 0) return "overwrite";
  const res = await choiceDialog<ConflictPolicy>({
    title: t("conflict.title"),
    message: t("conflict.text", { count: conflicts }),
    choices: [
      { value: "overwrite", label: t("conflict.overwrite"), hint: t("conflict.overwriteHint"), icon: <Replace size={16} />, primary: true },
      { value: "newer", label: t("conflict.newer"), hint: t("conflict.newerHint"), icon: <CalendarClock size={16} /> },
      { value: "skip", label: t("conflict.skip"), hint: t("conflict.skipHint"), icon: <SkipForward size={16} /> },
      { value: "rename", label: t("conflict.rename"), hint: t("conflict.renameHint"), icon: <CopyPlus size={16} /> },
    ],
  });
  return res ? res.value : null;
}

/** Uploads local entries into the remote directory `remoteDir`. */
export async function uploadEntries(
  sessionId: string,
  entries: FileEntry[],
  remoteDir: string,
  existingRemote: FileEntry[] | null,
) {
  if (!entries.length) return;
  const existing = new Set((existingRemote ?? []).map((e) => e.name));
  const conflicts = entries.filter((e) => e.kind === "file" && existing.has(e.name)).length;
  const policy = await resolvePolicy(conflicts);
  if (!policy) return;
  const items: TransferRequest[] = entries.map((e) => ({
    localPath: e.path,
    remotePath: rjoin(remoteDir, e.name),
    isDir: e.kind === "dir",
    size: e.size,
  }));
  await api.enqueueTransfers(sessionId, "upload", items, policy).catch(showError);
  openPanel();
}

/** Downloads remote entries into the local directory `localDir`. */
export async function downloadEntries(
  sessionId: string,
  entries: FileEntry[],
  localDir: string,
  existingLocal: FileEntry[] | null,
) {
  if (!entries.length) return;
  const existing = new Set((existingLocal ?? []).map((e) => e.name));
  const conflicts = entries.filter((e) => e.kind === "file" && existing.has(e.name)).length;
  const policy = await resolvePolicy(conflicts);
  if (!policy) return;
  const items: TransferRequest[] = entries.map((e) => ({
    localPath: ljoin(localDir, e.name),
    remotePath: e.path,
    isDir: e.kind === "dir",
    size: e.size,
  }));
  await api.enqueueTransfers(sessionId, "download", items, policy).catch(showError);
  openPanel();
}

/** Uploads files dropped from the operating system. */
export async function uploadPaths(sessionId: string, paths: string[], remoteDir: string, existingRemote: FileEntry[] | null) {
  const entries: FileEntry[] = [];
  for (const p of paths) {
    const e = await api.localStat(p).catch(() => null);
    if (e) entries.push(e);
    else
      entries.push({
        name: lname(p),
        path: p,
        kind: "file",
        size: 0,
        modified: null,
        mode: null,
        owner: null,
        group: null,
        isLink: false,
        linkTarget: null,
      });
  }
  await uploadEntries(sessionId, entries, remoteDir, existingRemote);
}

function openPanel() {
  const s = useStore.getState();
  if (!s.settings.panelOpen) s.updateSettings({ panelOpen: true });
}

export async function openRemote(sessionId: string, entry: FileEntry) {
  const store = useStore.getState();
  try {
    const watch = !store.platform?.mobile;
    const file = await api.openRemoteFile(sessionId, entry.path, watch);
    if (watch) store.toast({ kind: "info", message: t("edit.opened", { name: file.name }) });
  } catch (e) {
    showError(e);
  }
}

// ---------------------------------------------------------------------------
// Backend events
// ---------------------------------------------------------------------------

let finishedTimer: number | undefined;
const pendingRefresh = { local: false, remote: new Set<string>() };

function scheduleRefresh(t: TransferInfo) {
  if (t.direction === "download") pendingRefresh.local = true;
  else pendingRefresh.remote.add(t.sessionId);
  if (finishedTimer) return;
  finishedTimer = window.setTimeout(() => {
    finishedTimer = undefined;
    const s = useStore.getState();
    if (pendingRefresh.local) s.bumpLocal();
    for (const id of pendingRefresh.remote) s.bumpRemote(id);
    pendingRefresh.local = false;
    pendingRefresh.remote.clear();
  }, 600);
}

let initialized = false;

export async function initBackend() {
  if (initialized) return;
  initialized = true;
  const store = useStore.getState();
  const platform = await api.platformInfo();
  store.setPlatform(platform);
  await reloadSites();
  store.setTransfers(await api.listTransfers());
  syncTransferOptions();

  await listen<LogEntry>("log", (e) => useStore.getState().addLog(e.payload));
  await listen<TransferInfo>("transfer-update", (e) => {
    const t = e.payload;
    useStore.getState().upsertTransfers([t]);
    if (t.status === "done") scheduleRefresh(t);
  });
  await listen<TransferInfo[]>("transfers-added", (e) => useStore.getState().upsertTransfers(e.payload));
  await listen<TransferProgress[]>("transfer-progress", (e) => useStore.getState().applyProgress(e.payload));
  await listen<EditedFile>("edit-changed", (e) => {
    const file = e.payload;
    useStore.getState().toast({
      kind: "info",
      sticky: true,
      message: t("edit.changed", { name: file.name }),
      action: {
        label: t("edit.upload"),
        run: () => api.uploadEdited(file.id).catch(showError),
      },
      secondary: {
        label: t("edit.stop"),
        run: () => api.stopEditing(file.id).catch(() => {}),
      },
    });
  });
}
