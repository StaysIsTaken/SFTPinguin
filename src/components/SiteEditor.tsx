import { useState } from "react";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
import { ChevronDown, ChevronRight, FolderOpen, LockOpen, Star, Trash2 } from "lucide-react";
import { api, AuthMethod, defaultPort, emptySite, isUnencrypted, Protocol, Site } from "../lib/api";
import { FolderSelect } from "./FolderPicker";
import { Modal, openDialog } from "../lib/dialogs";
import { t, useT } from "../lib/i18n";
import { useStore } from "../lib/store";
import { showError } from "../lib/actions";

export const SITE_COLORS = ["", "#ef4444", "#f97316", "#eab308", "#22c55e", "#14b8a6", "#3b82f6", "#8b5cf6", "#ec4899"];

type Family = "sftp" | "ftp" | "ftps" | "webdav" | "s3";

const FAMILIES: { id: Family; label: string }[] = [
  { id: "sftp", label: "SFTP" },
  { id: "ftp", label: "FTP" },
  { id: "ftps", label: "FTPS" },
  { id: "webdav", label: "WebDAV" },
  { id: "s3", label: "S3" },
];

function familyOf(p: Protocol): Family {
  if (p === "ftps-implicit") return "ftps";
  if (p === "webdavs") return "webdav";
  return p as Family;
}

function defaultProtocol(f: Family): Protocol {
  if (f === "webdav") return "webdavs";
  return f as Protocol;
}

export type SiteEditorResult = { site: Site; connect: boolean } | null;

export function openSiteEditor(initial?: Site, opts?: { password?: string }): Promise<SiteEditorResult> {
  return openDialog<SiteEditorResult>((done) => (
    <SiteEditor initial={initial} initialPassword={opts?.password} done={done} />
  ));
}

function SiteEditor({
  initial,
  initialPassword,
  done,
}: {
  initial?: Site;
  initialPassword?: string;
  done: (r: SiteEditorResult) => void;
}) {
  const t = useT();
  const platform = useStore((s) => s.platform);
  const [site, setSite] = useState<Site>(() => ({ ...(initial ?? emptySite()) }));
  const [password, setPassword] = useState(initialPassword ?? "");
  const [keyData, setKeyData] = useState("");
  const [clearKey, setClearKey] = useState(false);
  const [advanced, setAdvanced] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const isNew = !initial?.id;
  const family = familyOf(site.protocol);
  const unencrypted = isUnencrypted(site);
  const set = <K extends keyof Site>(key: K, value: Site[K]) => setSite((s) => ({ ...s, [key]: value }));

  const authOptions: AuthMethod[] =
    family === "sftp"
      ? platform?.mobile
        ? ["password", "key"]
        : ["password", "key", "agent"]
      : family === "s3"
        ? ["password"]
        : ["password", "anonymous"];

  const setFamily = (f: Family) => {
    setSite((s) => {
      const protocol = defaultProtocol(f);
      const auth = f === "sftp" || s.auth === "password" || s.auth === "anonymous" ? s.auth : "password";
      return {
        ...s,
        protocol,
        auth: f !== "sftp" && (auth === "key" || auth === "agent") ? "password" : auth,
      };
    });
  };

  const chooseKey = async () => {
    const file = await openFileDialog({ multiple: false, directory: false });
    if (typeof file === "string") set("keyPath", file);
  };

  const chooseLocalDir = async () => {
    const dir = await openFileDialog({ multiple: false, directory: true });
    if (typeof dir === "string") set("localPath", dir);
  };

  const save = async (connect: boolean) => {
    if (!site.host.trim() && site.protocol !== "s3") {
      setError(t("site.hostRequired"));
      return;
    }
    setSaving(true);
    try {
      const { site: saved, warning } = await api.saveSite(
        { ...site, host: site.host.trim(), username: site.username.trim() },
        {
          password: password ? password : null,
          keyData: keyData.trim() ? keyData : null,
          clearKeyData: clearKey,
        },
      );
      useStore.getState().upsertSite(saved);
      if (warning) useStore.getState().toast({ kind: "info", message: t("site.passwordNotSaved", { reason: warning }) });
      done({ site: saved, connect });
    } catch (e) {
      showError(e);
    } finally {
      setSaving(false);
    }
  };

  const secretLabel = family === "s3" ? t("site.secretKey") : site.auth === "key" ? t("site.passphrase") : t("site.password");
  const showSecret = site.auth === "password" || site.auth === "key";
  const secretStored = site.hasPassword;

  return (
    <Modal
      title={isNew ? t("site.title.new") : t("site.title.edit")}
      onClose={() => done(null)}
      width={560}
      footer={
        <>
          <button className={`icon-btn fav-toggle ${site.favorite ? "on" : ""}`} onClick={() => set("favorite", !site.favorite)} title={t("site.favorite")}>
            <Star size={16} fill={site.favorite ? "currentColor" : "none"} />
          </button>
          <div className="spacer" />
          <button className="btn" onClick={() => done(null)}>
            {t("common.cancel")}
          </button>
          <button className="btn" disabled={saving} onClick={() => save(false)}>
            {t("common.save")}
          </button>
          <button className="btn btn-primary" disabled={saving} onClick={() => save(true)}>
            {t("site.saveAndConnect")}
          </button>
        </>
      }
    >
      <form
        className="form"
        onSubmit={(e) => {
          e.preventDefault();
          save(true);
        }}
      >
        <div className="segmented" role="tablist">
          {FAMILIES.map((f) => (
            <button
              type="button"
              key={f.id}
              className={family === f.id ? "active" : ""}
              onClick={() => setFamily(f.id)}
            >
              {f.label}
            </button>
          ))}
        </div>

        {family === "ftps" && (
          <div className="radio-row">
            <label className="check">
              <input type="radio" checked={site.protocol === "ftps"} onChange={() => set("protocol", "ftps")} />
              <span>{t("site.ftpsExplicit")}</span>
            </label>
            <label className="check">
              <input type="radio" checked={site.protocol === "ftps-implicit"} onChange={() => set("protocol", "ftps-implicit")} />
              <span>{t("site.ftpsImplicit")}</span>
            </label>
          </div>
        )}
        {family === "webdav" && (
          <div className="radio-row">
            <label className="check">
              <input type="radio" checked={site.protocol === "webdavs"} onChange={() => set("protocol", "webdavs")} />
              <span>HTTPS</span>
            </label>
            <label className="check">
              <input type="radio" checked={site.protocol === "webdav"} onChange={() => set("protocol", "webdav")} />
              <span>HTTP</span>
            </label>
          </div>
        )}

        {unencrypted && (
          <div className="warn-box">
            <LockOpen size={16} />
            <span>{site.protocol === "ftp" ? t("site.unencryptedWarning") : t("site.httpWarning")}</span>
          </div>
        )}

        <div className="frow">
          <label className="field grow">
            <span>{t("site.name")}</span>
            <input className="input" value={site.name} placeholder={t("site.namePlaceholder")} onChange={(e) => set("name", e.target.value)} autoFocus />
          </label>
          <label className="field" style={{ width: 190 }}>
            <span>{t("site.group")}</span>
            <FolderSelect value={site.group} onChange={(v) => set("group", v)} />
          </label>
        </div>

        {family === "s3" ? (
          <div className="frow">
            <label className="field grow">
              <span>{t("site.endpoint")}</span>
              <input
                className="input"
                value={site.endpoint}
                placeholder={t("site.endpointPlaceholder")}
                autoCapitalize="off"
                spellCheck={false}
                onChange={(e) => set("endpoint", e.target.value)}
              />
            </label>
            <label className="field" style={{ width: 150 }}>
              <span>{t("site.region")}</span>
              <input className="input" value={site.region} placeholder="eu-central-1" onChange={(e) => set("region", e.target.value)} />
            </label>
          </div>
        ) : (
          <div className="frow">
            <label className="field grow">
              <span>{t("site.host")}</span>
              <input
                className="input"
                value={site.host}
                placeholder={family === "webdav" ? "cloud.example.com/remote.php/dav/files/me" : t("site.hostPlaceholder")}
                autoCapitalize="off"
                autoCorrect="off"
                spellCheck={false}
                onChange={(e) => set("host", e.target.value)}
              />
            </label>
            <label className="field" style={{ width: 96 }}>
              <span>{t("site.port")}</span>
              <input
                className="input"
                inputMode="numeric"
                value={site.port ?? ""}
                placeholder={String(defaultPort(site.protocol))}
                onChange={(e) => {
                  const v = parseInt(e.target.value.replace(/\D/g, ""), 10);
                  set("port", Number.isFinite(v) ? Math.min(v, 65535) : null);
                }}
              />
            </label>
          </div>
        )}

        {site.auth !== "anonymous" && (
          <label className="field">
            <span>{family === "s3" ? t("site.accessKey") : t("site.user")}</span>
            <input
              className="input"
              value={site.username}
              autoCapitalize="off"
              autoCorrect="off"
              spellCheck={false}
              onChange={(e) => set("username", e.target.value)}
            />
          </label>
        )}

        {authOptions.length > 1 && (
          <label className="field">
            <span>{t("site.auth")}</span>
            <select className="input" value={site.auth} onChange={(e) => set("auth", e.target.value as AuthMethod)}>
              {authOptions.map((a) => (
                <option key={a} value={a}>
                  {t(`site.auth.${a}` as any)}
                </option>
              ))}
            </select>
          </label>
        )}

        {site.auth === "key" && (
          <>
            <label className="field">
              <span>{t("site.keyFile")}</span>
              <div className="input-group">
                <input
                  className="input"
                  value={site.keyPath ?? ""}
                  placeholder="~/.ssh/id_ed25519"
                  spellCheck={false}
                  onChange={(e) => set("keyPath", e.target.value || null)}
                />
                {!platform?.mobile && (
                  <button type="button" className="btn" onClick={chooseKey}>
                    {t("site.keyFileChoose")}
                  </button>
                )}
              </div>
            </label>
            {site.hasKeyData && !clearKey ? (
              <div className="stored-note">
                <span>{t("site.keySaved")}</span>
                <button type="button" className="btn btn-sm" onClick={() => setClearKey(true)}>
                  <Trash2 size={13} /> {t("site.keyRemove")}
                </button>
              </div>
            ) : (
              <label className="field">
                <span>{t("site.keyPaste")}</span>
                <textarea
                  className="input mono"
                  rows={3}
                  value={keyData}
                  placeholder={t("site.keyPastePlaceholder")}
                  spellCheck={false}
                  onChange={(e) => setKeyData(e.target.value)}
                />
              </label>
            )}
          </>
        )}

        {showSecret && (
          <label className="field">
            <span>{secretLabel}</span>
            <input
              className="input"
              type="password"
              value={password}
              autoComplete="new-password"
              placeholder={secretStored ? t("site.passwordSaved") : ""}
              onChange={(e) => setPassword(e.target.value)}
            />
          </label>
        )}

        {showSecret && (
          <label className="check">
            <input type="checkbox" checked={site.savePassword} onChange={(e) => set("savePassword", e.target.checked)} />
            <span>
              {t("site.savePassword")}
              <small className="muted">
                {" "}
                – {platform?.mobile ? t("site.savePasswordHintMobile") : t("site.savePasswordHintDesktop")}
              </small>
            </span>
          </label>
        )}

        <button type="button" className="disclosure" onClick={() => setAdvanced(!advanced)}>
          {advanced ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
          {t("site.advanced")}
        </button>

        {advanced && (
          <div className="form advanced">
            <label className="field">
              <span>{t("site.remotePath")}</span>
              <input
                className="input"
                value={site.remotePath}
                placeholder={family === "s3" ? t("site.bucketHint") : "/"}
                spellCheck={false}
                onChange={(e) => set("remotePath", e.target.value)}
              />
            </label>
            <label className="field">
              <span>{t("site.localPath")}</span>
              <div className="input-group">
                <input className="input" value={site.localPath} spellCheck={false} onChange={(e) => set("localPath", e.target.value)} />
                {!platform?.mobile && (
                  <button type="button" className="btn icon-only" onClick={chooseLocalDir}>
                    <FolderOpen size={15} />
                  </button>
                )}
              </div>
            </label>
            <div className="frow">
              <label className="field" style={{ width: 120 }}>
                <span>{t("site.timeout")}</span>
                <input
                  className="input"
                  inputMode="numeric"
                  value={site.timeout}
                  onChange={(e) => set("timeout", Math.max(3, parseInt(e.target.value, 10) || 20))}
                />
              </label>
            </div>
            <div className="field">
              <span>{t("site.color")}</span>
              <div className="swatches">
                {SITE_COLORS.map((c) => (
                  <button
                    type="button"
                    key={c || "none"}
                    className={`swatch ${site.color === c ? "active" : ""} ${c ? "" : "none"}`}
                    style={c ? { background: c } : undefined}
                    onClick={() => set("color", c)}
                    aria-label={c || "none"}
                  />
                ))}
              </div>
            </div>
            {(family === "ftp" || family === "ftps") && (
              <label className="check">
                <input type="checkbox" checked={site.passive} onChange={(e) => set("passive", e.target.checked)} />
                <span>{t("site.passive")}</span>
              </label>
            )}
            {family === "s3" && (
              <label className="check">
                <input type="checkbox" checked={site.pathStyle} onChange={(e) => set("pathStyle", e.target.checked)} />
                <span>{t("site.pathStyle")}</span>
              </label>
            )}
            {unencrypted && (
              <label className="check">
                <input type="checkbox" checked={site.allowInsecure} onChange={(e) => set("allowInsecure", e.target.checked)} />
                <span>{t("site.insecureAllowed")}</span>
              </label>
            )}
            <label className="field">
              <span>{t("site.notes")}</span>
              <textarea className="input" rows={2} value={site.notes} onChange={(e) => set("notes", e.target.value)} />
            </label>
          </div>
        )}
        {error && <div className="form-error">{error}</div>}
        <button type="submit" hidden />
      </form>
    </Modal>
  );
}

export function duplicateSite(site: Site): Site {
  return {
    ...site,
    id: "",
    name: `${site.name || site.host} (${t("common.duplicate").toLowerCase()})`,
    hasPassword: false,
    hasKeyData: false,
    createdAt: 0,
    lastUsedAt: null,
  };
}
