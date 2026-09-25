import { invoke } from "@tauri-apps/api/core";

export type Protocol = "sftp" | "ftp" | "ftps" | "ftps-implicit" | "webdav" | "webdavs" | "s3";
export type AuthMethod = "password" | "key" | "agent" | "anonymous";

export const PROTOCOLS: { id: Protocol; label: string; port: number }[] = [
  { id: "sftp", label: "SFTP", port: 22 },
  { id: "ftp", label: "FTP", port: 21 },
  { id: "ftps", label: "FTPS", port: 21 },
  { id: "ftps-implicit", label: "FTPS (implicit)", port: 990 },
  { id: "webdavs", label: "WebDAV (HTTPS)", port: 443 },
  { id: "webdav", label: "WebDAV (HTTP)", port: 80 },
  { id: "s3", label: "S3", port: 443 },
];

export function protocolLabel(p: Protocol): string {
  return PROTOCOLS.find((x) => x.id === p)?.label ?? p;
}

export function defaultPort(p: Protocol): number {
  return PROTOCOLS.find((x) => x.id === p)?.port ?? 22;
}

export interface Site {
  id: string;
  name: string;
  protocol: Protocol;
  host: string;
  port: number | null;
  username: string;
  auth: AuthMethod;
  keyPath: string | null;
  savePassword: boolean;
  remotePath: string;
  localPath: string;
  group: string;
  color: string;
  notes: string;
  favorite: boolean;
  passive: boolean;
  insecureTls: boolean;
  region: string;
  endpoint: string;
  pathStyle: boolean;
  timeout: number;
  createdAt: number;
  lastUsedAt: number | null;
  hasPassword: boolean;
  hasKeyData: boolean;
}

export function emptySite(protocol: Protocol = "sftp"): Site {
  return {
    id: "",
    name: "",
    protocol,
    host: "",
    port: null,
    username: "",
    auth: "password",
    keyPath: null,
    savePassword: true,
    remotePath: "",
    localPath: "",
    group: "",
    color: "",
    notes: "",
    favorite: false,
    passive: true,
    insecureTls: false,
    region: "",
    endpoint: "",
    pathStyle: false,
    timeout: 20,
    createdAt: 0,
    lastUsedAt: null,
    hasPassword: false,
    hasKeyData: false,
  };
}

export interface FileEntry {
  name: string;
  path: string;
  kind: "file" | "dir";
  size: number;
  modified: number | null;
  mode: number | null;
  owner: string | null;
  group: string | null;
  isLink: boolean;
  linkTarget: string | null;
}

export interface Capabilities {
  chmod: boolean;
  renameDirs: boolean;
  symlinks: boolean;
  serverSideCopy: boolean;
}

export interface SessionInfo {
  id: string;
  siteId: string | null;
  title: string;
  protocol: Protocol;
  host: string;
  username: string;
  home: string;
  localPath: string;
  capabilities: Capabilities;
}

export interface HostKey {
  host: string;
  port: number;
  keyType: string;
  fingerprint: string;
  addedAt: number;
}

export type TransferDirection = "upload" | "download";
export type TransferStatus = "queued" | "running" | "done" | "failed" | "cancelled" | "skipped";
export type ConflictPolicy = "overwrite" | "skip" | "newer" | "rename";

export interface TransferInfo {
  id: string;
  sessionId: string;
  direction: TransferDirection;
  name: string;
  localPath: string;
  remotePath: string;
  size: number;
  transferred: number;
  status: TransferStatus;
  error: string | null;
  speed: number;
  createdAt: number;
  startedAt: number | null;
  finishedAt: number | null;
}

export interface TransferProgress {
  id: string;
  transferred: number;
  size: number;
  speed: number;
}

export interface TransferRequest {
  localPath: string;
  remotePath: string;
  isDir: boolean;
  size?: number;
}

export interface LogEntry {
  sessionId: string | null;
  level: "info" | "success" | "warn" | "error";
  message: string;
  time: number;
}

export interface EditedFile {
  id: string;
  sessionId: string;
  remotePath: string;
  localPath: string;
  name: string;
}

export interface Place {
  id: string;
  path: string;
}

export interface PlatformInfo {
  os: string;
  mobile: boolean;
  secretBackend: "system-keychain" | "app-sandbox";
  version: string;
  pathSeparator: string;
}

export type ErrorCode =
  | "connection"
  | "auth_failed"
  | "password_required"
  | "passphrase_required"
  | "host_key_unknown"
  | "host_key_changed"
  | "not_found"
  | "permission_denied"
  | "already_exists"
  | "unsupported"
  | "cancelled"
  | "session_closed"
  | "invalid_input"
  | "protocol"
  | "secret_store"
  | "io";

export interface AppError {
  code: ErrorCode;
  message: string;
  details?: any;
}

export function asAppError(e: unknown): AppError {
  if (e && typeof e === "object" && "code" in e && "message" in e) return e as AppError;
  return { code: "io", message: String(e) };
}

export interface PathItem {
  path: string;
  isDir: boolean;
}

export const api = {
  platformInfo: () => invoke<PlatformInfo>("platform_info"),

  listSites: () => invoke<Site[]>("list_sites"),
  saveSite: (site: Site, secrets: { password?: string | null; keyData?: string | null; clearKeyData?: boolean }) =>
    invoke<Site>("save_site", { site, secrets }),
  deleteSite: (id: string) => invoke<void>("delete_site", { id }),
  filezillaDefaultPath: () => invoke<string | null>("filezilla_default_path"),
  importFilezilla: (path: string) => invoke<{ imported: number; withPassword: number }>("import_filezilla", { path }),

  trustHostKey: (key: HostKey) => invoke<void>("trust_host_key", { key }),
  listHostKeys: () => invoke<HostKey[]>("list_host_keys"),
  removeHostKey: (host: string, port: number) => invoke<void>("remove_host_key", { host, port }),

  connect: (req: { siteId?: string | null; site?: Site | null; password?: string | null; remember?: boolean }) =>
    invoke<SessionInfo>("connect", { req }),
  disconnect: (sessionId: string) => invoke<void>("disconnect", { sessionId }),
  listRemote: (sessionId: string, path: string) => invoke<FileEntry[]>("list_remote", { sessionId, path }),
  remoteStat: (sessionId: string, path: string) => invoke<FileEntry | null>("remote_stat", { sessionId, path }),
  remoteMkdir: (sessionId: string, path: string) => invoke<void>("remote_mkdir", { sessionId, path }),
  remoteCreateFile: (sessionId: string, path: string) => invoke<void>("remote_create_file", { sessionId, path }),
  remoteRename: (sessionId: string, from: string, to: string) => invoke<void>("remote_rename", { sessionId, from, to }),
  remoteDelete: (sessionId: string, items: PathItem[]) => invoke<void>("remote_delete", { sessionId, items }),
  remoteChmod: (sessionId: string, items: PathItem[], mode: number, recursive: boolean) =>
    invoke<void>("remote_chmod", { sessionId, items, mode, recursive }),

  listLocal: (path: string) => invoke<FileEntry[]>("list_local", { path }),
  localPlaces: () => invoke<Place[]>("local_places"),
  localDefaultDir: () => invoke<string>("local_default_dir"),
  localStat: (path: string) => invoke<FileEntry | null>("local_stat", { path }),
  localMkdir: (path: string) => invoke<void>("local_mkdir", { path }),
  localCreateFile: (path: string) => invoke<void>("local_create_file", { path }),
  localRename: (from: string, to: string) => invoke<void>("local_rename", { from, to }),
  localDelete: (paths: string[], trash: boolean) => invoke<void>("local_delete", { paths, trash }),
  localChmod: (paths: string[], mode: number) => invoke<void>("local_chmod", { paths, mode }),

  enqueueTransfers: (sessionId: string, direction: TransferDirection, items: TransferRequest[], policy: ConflictPolicy) =>
    invoke<void>("enqueue_transfers", { sessionId, direction, items, policy }),
  listTransfers: () => invoke<TransferInfo[]>("list_transfers"),
  cancelTransfer: (id: string) => invoke<void>("cancel_transfer", { id }),
  cancelAllTransfers: () => invoke<void>("cancel_all_transfers"),
  retryTransfer: (id: string) => invoke<void>("retry_transfer", { id }),
  clearTransfers: (all: boolean) => invoke<void>("clear_transfers", { all }),
  setTransferOptions: (maxConcurrent: number, preserveMtime: boolean) =>
    invoke<void>("set_transfer_options", { maxConcurrent, preserveMtime }),

  openRemoteFile: (sessionId: string, path: string, watch: boolean) =>
    invoke<EditedFile>("open_remote_file", { sessionId, path, watch }),
  uploadEdited: (id: string) => invoke<void>("upload_edited", { id }),
  stopEditing: (id: string) => invoke<void>("stop_editing", { id }),
  openLocalPath: (path: string) => invoke<void>("open_local_path", { path }),
};
