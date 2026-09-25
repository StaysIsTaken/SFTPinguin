import { ReactNode, useEffect, useState } from "react";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
import { Download, Info, KeyRound, Monitor, Moon, Settings2, ShieldCheck, Sun, Trash2, ArrowLeftRight } from "lucide-react";
import { api, CertInfo, HostKey } from "../lib/api";
import { Modal, openDialog } from "../lib/dialogs";
import { t, useT } from "../lib/i18n";
import { Settings as SettingsType, useStore } from "../lib/store";
import { reloadSites, showError, syncTransferOptions } from "../lib/actions";
import { formatDate } from "../lib/format";

export const ACCENTS: { id: string; color: string }[] = [
  { id: "indigo", color: "#5b5bd6" },
  { id: "sky", color: "#0ea5e9" },
  { id: "emerald", color: "#10b981" },
  { id: "violet", color: "#8b5cf6" },
  { id: "rose", color: "#f43f5e" },
  { id: "amber", color: "#f59e0b" },
];

export async function importFromFilezilla() {
  const store = useStore.getState();
  try {
    let path = store.platform?.mobile ? null : await api.filezillaDefaultPath();
    if (!path) {
      store.toast({ kind: "info", message: t("settings.importNone") });
      const chosen = await openFileDialog({
        multiple: false,
        filters: [{ name: "FileZilla", extensions: ["xml"] }],
      });
      if (typeof chosen !== "string") return;
      path = chosen;
    }
    const res = await api.importFilezilla(path);
    await reloadSites();
    store.toast({ kind: "success", message: t("settings.importDone", { count: res.imported, pw: res.withPassword }) });
  } catch (e) {
    showError(e);
  }
}

export function openSettings() {
  return openDialog<void>((done) => <SettingsDialog done={done} />);
}

type Section = "general" | "files" | "transfers" | "security" | "about";

function SettingsDialog({ done }: { done: () => void }) {
  const t = useT();
  const settings = useStore((s) => s.settings);
  const platform = useStore((s) => s.platform);
  const update = useStore((s) => s.updateSettings);
  const [section, setSection] = useState<Section>("general");
  const [hostKeys, setHostKeys] = useState<HostKey[]>([]);
  const [certs, setCerts] = useState<CertInfo[]>([]);

  useEffect(() => {
    if (section === "security") {
      api.listHostKeys().then(setHostKeys).catch(showError);
      api.listCertificates().then(setCerts).catch(showError);
    }
  }, [section]);

  const set = (patch: Partial<SettingsType>) => {
    update(patch);
    if ("maxConcurrent" in patch || "preserveMtime" in patch) syncTransferOptions();
  };

  const sections: { id: Section; label: string; icon: ReactNode }[] = [
    { id: "general", label: t("settings.general"), icon: <Settings2 size={15} /> },
    { id: "files", label: t("settings.files"), icon: <Monitor size={15} /> },
    { id: "transfers", label: t("settings.transfers"), icon: <ArrowLeftRight size={15} /> },
    { id: "security", label: t("settings.security"), icon: <KeyRound size={15} /> },
    { id: "about", label: t("settings.about"), icon: <Info size={15} /> },
  ];

  return (
    <Modal title={t("settings.title")} onClose={done} width={680}>
      <div className="settings">
        <nav className="settings-nav">
          {sections.map((s) => (
            <button key={s.id} className={section === s.id ? "active" : ""} onClick={() => setSection(s.id)}>
              {s.icon}
              <span>{s.label}</span>
            </button>
          ))}
        </nav>
        <div className="settings-body">
          {section === "general" && (
            <>
              <div className="setting">
                <span>{t("settings.language")}</span>
                <select className="input" value={settings.language} onChange={(e) => set({ language: e.target.value as any })}>
                  <option value="auto">{t("settings.language.auto")}</option>
                  <option value="de">Deutsch</option>
                  <option value="en">English</option>
                </select>
              </div>
              <div className="setting">
                <span>{t("settings.theme")}</span>
                <div className="segmented small">
                  {(
                    [
                      ["system", <Monitor size={14} key="m" />],
                      ["light", <Sun size={14} key="s" />],
                      ["dark", <Moon size={14} key="d" />],
                    ] as const
                  ).map(([id, icon]) => (
                    <button key={id} className={settings.theme === id ? "active" : ""} onClick={() => set({ theme: id })}>
                      {icon} {t(`settings.theme.${id}`)}
                    </button>
                  ))}
                </div>
              </div>
              <div className="setting">
                <span>{t("settings.accent")}</span>
                <div className="swatches">
                  {ACCENTS.map((a) => (
                    <button
                      key={a.id}
                      className={`swatch ${settings.accent === a.id ? "active" : ""}`}
                      style={{ background: a.color }}
                      onClick={() => set({ accent: a.id })}
                      aria-label={a.id}
                    />
                  ))}
                </div>
              </div>
              <div className="setting">
                <span>{t("settings.import")}</span>
                <button className="btn" onClick={() => importFromFilezilla()}>
                  <Download size={14} /> {t("settings.importFilezilla")}
                </button>
              </div>
            </>
          )}

          {section === "files" && (
            <>
              <Toggle label={t("settings.showHidden")} value={settings.showHidden} onChange={(v) => set({ showHidden: v })} />
              <Toggle label={t("settings.confirmDelete")} value={settings.confirmDelete} onChange={(v) => set({ confirmDelete: v })} />
              {!platform?.mobile && (
                <Toggle label={t("settings.useTrash")} value={settings.useTrash} onChange={(v) => set({ useTrash: v })} />
              )}
              <div className="setting">
                <span>{t("settings.doubleClick")}</span>
                <select className="input" value={settings.doubleClick} onChange={(e) => set({ doubleClick: e.target.value as any })}>
                  <option value="transfer">{t("settings.doubleClick.transfer")}</option>
                  <option value="open">{t("settings.doubleClick.open")}</option>
                </select>
              </div>
            </>
          )}

          {section === "transfers" && (
            <>
              <div className="setting">
                <span>{t("settings.concurrent")}</span>
                <div className="range">
                  <input
                    type="range"
                    min={1}
                    max={10}
                    value={settings.maxConcurrent}
                    onChange={(e) => set({ maxConcurrent: parseInt(e.target.value, 10) })}
                  />
                  <b>{settings.maxConcurrent}</b>
                </div>
              </div>
              <div className="setting">
                <span>{t("settings.conflict")}</span>
                <select className="input" value={settings.conflict} onChange={(e) => set({ conflict: e.target.value as any })}>
                  <option value="ask">{t("conflict.ask")}</option>
                  <option value="overwrite">{t("conflict.overwrite")}</option>
                  <option value="skip">{t("conflict.skip")}</option>
                  <option value="newer">{t("conflict.newer")}</option>
                  <option value="rename">{t("conflict.rename")}</option>
                </select>
              </div>
              <Toggle label={t("settings.preserveMtime")} value={settings.preserveMtime} onChange={(v) => set({ preserveMtime: v })} />
            </>
          )}

          {section === "security" && (
            <>
              <div className="info-box">
                <ShieldCheck size={16} />
                <span>{t("settings.securityIntro")}</span>
              </div>
              <div className="setting">
                <span>{t("settings.insecurePolicy")}</span>
                <select
                  className="input"
                  value={settings.insecurePolicy}
                  onChange={(e) => set({ insecurePolicy: e.target.value as "warn" | "block" })}
                >
                  <option value="warn">{t("settings.insecurePolicy.warn")}</option>
                  <option value="block">{t("settings.insecurePolicy.block")}</option>
                </select>
              </div>
              <p className="muted small">
                {t("settings.secretStore", {
                  store: t(`settings.secretStore.${platform?.secretBackend ?? "system-keychain"}` as any),
                })}
              </p>
              <h4>{t("settings.knownHosts")}</h4>
              {hostKeys.length === 0 && <p className="muted small">{t("settings.noKnownHosts")}</p>}
              <ul className="hostkeys">
                {hostKeys.map((k) => (
                  <li key={`${k.host}:${k.port}`}>
                    <div>
                      <b>
                        {k.host}
                        {k.port !== 22 ? `:${k.port}` : ""}
                      </b>
                      <div className="mono small muted selectable">
                        {k.keyType} · {k.fingerprint}
                      </div>
                      <div className="small muted">{formatDate(k.addedAt)}</div>
                    </div>
                    <button
                      className="icon-btn"
                      title={t("common.delete")}
                      onClick={async () => {
                        await api.removeHostKey(k.host, k.port).catch(showError);
                        setHostKeys(await api.listHostKeys());
                      }}
                    >
                      <Trash2 size={14} />
                    </button>
                  </li>
                ))}
              </ul>
              <h4>{t("settings.trustedCerts")}</h4>
              {certs.length === 0 && <p className="muted small">{t("settings.noTrustedCerts")}</p>}
              <ul className="hostkeys">
                {certs.map((c) => (
                  <li key={`${c.host}:${c.port}`}>
                    <div>
                      <b>
                        {c.host}:{c.port}
                      </b>
                      <div className="small">{c.subject}</div>
                      <div className="mono small muted selectable">{c.fingerprint}</div>
                      <div className="small muted">{formatDate(c.addedAt)}</div>
                    </div>
                    <button
                      className="icon-btn"
                      title={t("common.delete")}
                      onClick={async () => {
                        await api.removeCertificate(c.host, c.port).catch(showError);
                        setCerts(await api.listCertificates());
                      }}
                    >
                      <Trash2 size={14} />
                    </button>
                  </li>
                ))}
              </ul>
            </>
          )}

          {section === "about" && (
            <div className="about">
              <img src="/logo.svg" width={72} height={72} alt="" />
              <h3>SFTPinguin</h3>
              <p>{t("settings.aboutText", { version: platform?.version ?? "" })}</p>
              <p className="muted small">
                {platform?.os} · Tauri 2 · Rust
              </p>
            </div>
          )}
        </div>
      </div>
    </Modal>
  );
}

function Toggle({ label, value, onChange }: { label: string; value: boolean; onChange: (v: boolean) => void }) {
  return (
    <label className="setting toggle-row">
      <span>{label}</span>
      <input type="checkbox" className="switch" checked={value} onChange={(e) => onChange(e.target.checked)} />
    </label>
  );
}
