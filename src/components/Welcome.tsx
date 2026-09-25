import { useState } from "react";
import { Clock, Download, Loader2, Plus, Zap } from "lucide-react";
import { AuthMethod, defaultPort, emptySite, Protocol, PROTOCOLS, protocolLabel } from "../lib/api";
import { useT } from "../lib/i18n";
import { useStore } from "../lib/store";
import { openSession } from "../lib/actions";
import { Logo, newSite } from "./Sidebar";
import { importFromFilezilla } from "./Settings";
import { openSiteEditor } from "./SiteEditor";

/** Parses things like `sftp://user@host:2222/path`. */
function parseUrl(input: string): { protocol?: Protocol; host: string; user?: string; port?: number; path?: string } {
  const m = input.trim().match(/^(?:([a-z0-9+-]+):\/\/)?(?:([^@/]+)@)?([^:/]+)(?::(\d+))?(\/.*)?$/i);
  if (!m) return { host: input.trim() };
  const scheme = m[1]?.toLowerCase();
  const map: Record<string, Protocol> = {
    sftp: "sftp",
    ssh: "sftp",
    ftp: "ftp",
    ftps: "ftps",
    ftpes: "ftps",
    dav: "webdav",
    http: "webdav",
    davs: "webdavs",
    https: "webdavs",
    webdav: "webdav",
    webdavs: "webdavs",
    s3: "s3",
  };
  return {
    protocol: scheme ? map[scheme] : undefined,
    user: m[2] ? decodeURIComponent(m[2]) : undefined,
    host: m[3],
    port: m[4] ? parseInt(m[4], 10) : undefined,
    path: m[5],
  };
}

export function QuickConnect({ compact }: { compact?: boolean }) {
  const t = useT();
  const connecting = useStore((s) => s.connecting["quick"]);
  const [protocol, setProtocol] = useState<Protocol>("sftp");
  const [host, setHost] = useState("");
  const [port, setPort] = useState("");
  const [user, setUser] = useState("");
  const [password, setPassword] = useState("");

  const buildSite = () => {
    const parsed = parseUrl(host);
    const proto = parsed.protocol ?? protocol;
    const site = emptySite(proto);
    site.host = parsed.host;
    site.username = user || parsed.user || "";
    site.port = port ? parseInt(port, 10) : (parsed.port ?? null);
    site.remotePath = parsed.path ?? "";
    site.auth = (!site.username && proto !== "sftp" && proto !== "s3" ? "anonymous" : "password") as AuthMethod;
    site.savePassword = false;
    return site;
  };

  const connect = () => {
    if (!host.trim()) return;
    openSession({ site: buildSite(), password: password || undefined });
  };

  return (
    <form
      className={`quick ${compact ? "compact" : ""}`}
      onSubmit={(e) => {
        e.preventDefault();
        connect();
      }}
    >
      <select className="input quick-proto" value={protocol} onChange={(e) => setProtocol(e.target.value as Protocol)}>
        {PROTOCOLS.map((p) => (
          <option key={p.id} value={p.id}>
            {p.label}
          </option>
        ))}
      </select>
      <input
        className="input quick-host"
        placeholder={`${t("quick.host")} (sftp://user@host)`}
        value={host}
        autoCapitalize="off"
        autoCorrect="off"
        spellCheck={false}
        onChange={(e) => setHost(e.target.value)}
      />
      <input
        className="input quick-port"
        placeholder={String(defaultPort(protocol))}
        inputMode="numeric"
        value={port}
        onChange={(e) => setPort(e.target.value.replace(/\D/g, ""))}
      />
      <input
        className="input quick-user"
        placeholder={t("quick.user")}
        value={user}
        autoCapitalize="off"
        autoCorrect="off"
        spellCheck={false}
        onChange={(e) => setUser(e.target.value)}
      />
      <input
        className="input quick-pass"
        type="password"
        placeholder={t("quick.password")}
        value={password}
        onChange={(e) => setPassword(e.target.value)}
      />
      <button className="btn btn-primary quick-go" type="submit" disabled={!host.trim() || !!connecting}>
        {connecting ? <Loader2 size={15} className="spin" /> : <Zap size={15} />} {t("common.connect")}
      </button>
      {!compact && (
        <button
          type="button"
          className="btn btn-ghost quick-save"
          disabled={!host.trim()}
          onClick={async () => {
            const r = await openSiteEditor(buildSite(), { password });
            if (r?.connect) openSession({ siteId: r.site.id });
          }}
        >
          {t("quick.saveAfter")}
        </button>
      )}
    </form>
  );
}

export function Welcome() {
  const t = useT();
  const sites = useStore((s) => s.sites);
  const connecting = useStore((s) => s.connecting);
  const recent = [...sites]
    .filter((s) => s.lastUsedAt)
    .sort((a, b) => (b.lastUsedAt ?? 0) - (a.lastUsedAt ?? 0))
    .slice(0, 6);
  const shown = recent.length ? recent : sites.slice(0, 6);

  return (
    <div className="welcome">
      <div className="welcome-inner">
        <div className="welcome-hero">
          <Logo size={64} />
          <h1>{t("welcome.title")}</h1>
          <p className="muted">{t("welcome.subtitle")}</p>
        </div>

        <section className="card">
          <h3>
            <Zap size={15} /> {t("quick.title")}
          </h3>
          <QuickConnect />
        </section>

        {shown.length > 0 && (
          <section className="card">
            <h3>
              <Clock size={15} /> {recent.length ? t("welcome.recent") : t("sidebar.sites")}
            </h3>
            <div className="recent-grid">
              {shown.map((s) => (
                <button key={s.id} className="recent-item" onClick={() => openSession({ siteId: s.id })}>
                  <span className="site-dot" style={s.color ? { background: s.color } : undefined}>
                    {protocolLabel(s.protocol).slice(0, 1)}
                  </span>
                  <span className="site-text">
                    <span className="site-name">{s.name || s.host}</span>
                    <span className="site-sub">
                      {protocolLabel(s.protocol)} · {s.username ? `${s.username}@` : ""}
                      {s.host || s.endpoint}
                    </span>
                  </span>
                  {connecting[s.id] && <Loader2 size={14} className="spin muted" />}
                </button>
              ))}
            </div>
          </section>
        )}

        <div className="welcome-actions">
          <button className="btn" onClick={() => newSite()}>
            <Plus size={15} /> {t("welcome.addSite")}
          </button>
          <button className="btn" onClick={() => importFromFilezilla()}>
            <Download size={15} /> {t("welcome.import")}
          </button>
        </div>
        <p className="muted small center">{t("welcome.tip")}</p>
      </div>
    </div>
  );
}
