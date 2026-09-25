import { useState } from "react";
import { LockOpen, ShieldAlert, ShieldQuestion } from "lucide-react";
import type { CertInfo, HostKey } from "../lib/api";
import { Modal, openDialog } from "../lib/dialogs";
import { t, TKey } from "../lib/i18n";
import { formatDate } from "../lib/format";

function hostLabel(host: string, port: number, defaultPort: number) {
  return port === defaultPort ? host : `${host}:${port}`;
}

/**
 * Confirmation for a changed key / certificate: the accept button only becomes active
 * after the user confirmed that the new fingerprint was verified.
 */
function ChangedFooter({ label, done }: { label: string; done: (v: boolean) => void }) {
  const [checked, setChecked] = useState(false);
  return (
    <div className="confirm-footer">
      <label className="check">
        <input type="checkbox" checked={checked} onChange={(e) => setChecked(e.target.checked)} />
        <span>{t("security.confirmChange")}</span>
      </label>
      <div className="confirm-buttons">
        <button className="btn btn-primary" autoFocus onClick={() => done(false)}>
          {t("common.cancel")}
        </button>
        <button className="btn btn-danger" disabled={!checked} onClick={() => done(true)}>
          {label}
        </button>
      </div>
    </div>
  );
}

export function hostKeyDialog(kind: "unknown" | "changed", presented: HostKey, previous?: HostKey): Promise<boolean> {
  const host = hostLabel(presented.host, presented.port, 22);
  return openDialog<boolean>((done) => (
    <Modal
      title={kind === "unknown" ? t("hostkey.unknownTitle") : t("hostkey.changedTitle")}
      onClose={() => done(false)}
      danger={kind === "changed"}
      width={540}
      icon={
        kind === "unknown" ? (
          <ShieldQuestion size={20} className="text-accent" />
        ) : (
          <ShieldAlert size={20} className="text-danger" />
        )
      }
      footer={
        kind === "changed" ? (
          <ChangedFooter label={t("hostkey.replace")} done={done} />
        ) : (
          <>
            <button className="btn" onClick={() => done(false)}>
              {t("common.cancel")}
            </button>
            <button className="btn btn-primary" onClick={() => done(true)} autoFocus>
              {t("hostkey.trust")}
            </button>
          </>
        )
      }
    >
      <p className="dialog-message">
        {t(kind === "unknown" ? "hostkey.unknownText" : "hostkey.changedText", { host })}
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
      <p className="muted small">{t("security.noCredentials")}</p>
    </Modal>
  ));
}

/** Formats `AB:CD:…` fingerprints in lines of 8 bytes, easy to compare. */
function Fingerprint({ value }: { value: string }) {
  const parts = value.split(":");
  const lines: string[] = [];
  for (let i = 0; i < parts.length; i += 8) lines.push(parts.slice(i, i + 8).join(":"));
  return (
    <span className="mono selectable fingerprint">
      {lines.map((l, i) => (
        <span key={i}>
          {l}
          <br />
        </span>
      ))}
    </span>
  );
}

export function certDialog(
  kind: "untrusted" | "changed",
  presented: CertInfo,
  reason: string,
  previous?: CertInfo | null,
): Promise<boolean> {
  const host = hostLabel(presented.host, presented.port, 443);
  const reasonText = t(`cert.reason.${reason}` as TKey) || reason;
  const validity =
    presented.notBefore && presented.notAfter
      ? `${formatDate(presented.notBefore)} – ${formatDate(presented.notAfter)}`
      : "–";
  return openDialog<boolean>((done) => (
    <Modal
      title={kind === "untrusted" ? t("cert.untrustedTitle") : t("cert.changedTitle")}
      onClose={() => done(false)}
      danger={kind === "changed"}
      width={560}
      icon={
        kind === "untrusted" ? (
          <ShieldQuestion size={20} className="text-accent" />
        ) : (
          <ShieldAlert size={20} className="text-danger" />
        )
      }
      footer={
        kind === "changed" ? (
          <ChangedFooter label={t("cert.replace")} done={done} />
        ) : (
          <>
            <button className="btn" onClick={() => done(false)} autoFocus>
              {t("common.cancel")}
            </button>
            <button className="btn btn-primary" onClick={() => done(true)}>
              {t("cert.trust")}
            </button>
          </>
        )
      }
    >
      <p className="dialog-message">
        {t(kind === "untrusted" ? "cert.untrustedText" : "cert.changedText", { host, reason: reasonText })}
      </p>
      <dl className="kv">
        <dt>{t("cert.subject")}</dt>
        <dd className="selectable">{presented.subject || "–"}</dd>
        <dt>{t("cert.issuer")}</dt>
        <dd className="selectable">{presented.issuer || "–"}</dd>
        <dt>{t("cert.validity")}</dt>
        <dd>{validity}</dd>
        <dt>{t("cert.fingerprint")}</dt>
        <dd>
          <Fingerprint value={presented.fingerprint} />
        </dd>
        {previous && (
          <>
            <dt>{t("cert.previous")}</dt>
            <dd className="muted">
              <Fingerprint value={previous.fingerprint} />
            </dd>
          </>
        )}
      </dl>
      <p className="muted small">{t("security.noCredentials")}</p>
    </Modal>
  ));
}

/** Asks before an unencrypted connection. Resolves with `remember` or null (cancel). */
export function insecureDialog(
  kind: "ftp" | "http" | "no_tls",
  canRemember: boolean,
): Promise<{ remember: boolean } | null> {
  return openDialog((done) => <InsecureBody kind={kind} canRemember={canRemember} done={done} />);
}

function InsecureBody({
  kind,
  canRemember,
  done,
}: {
  kind: "ftp" | "http" | "no_tls";
  canRemember: boolean;
  done: (v: { remember: boolean } | null) => void;
}) {
  const [remember, setRemember] = useState(false);
  const text = kind === "ftp" ? t("insecure.textFtp") : kind === "http" ? t("insecure.textHttp") : t("insecure.textNoTls");
  return (
    <Modal
      title={t("insecure.title")}
      onClose={() => done(null)}
      danger
      width={500}
      icon={<LockOpen size={20} className="text-danger" />}
      footer={
        <>
          <button className="btn btn-primary" autoFocus onClick={() => done(null)}>
            {t("common.cancel")}
          </button>
          <button className="btn btn-danger" onClick={() => done({ remember })}>
            {t("insecure.connect")}
          </button>
        </>
      }
    >
      <p className="dialog-message">{text}</p>
      {canRemember && (
        <label className="check">
          <input type="checkbox" checked={remember} onChange={(e) => setRemember(e.target.checked)} />
          <span>{t("insecure.remember")}</span>
        </label>
      )}
    </Modal>
  );
}
