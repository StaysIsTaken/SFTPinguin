import { useState } from "react";
import { Modal, openDialog } from "../lib/dialogs";
import { t } from "../lib/i18n";

export function chmodDialog(initial: number, allowRecursive: boolean): Promise<{ mode: number; recursive: boolean } | null> {
  return openDialog((done) => <ChmodBody initial={initial} allowRecursive={allowRecursive} done={done} />);
}

const WHO = ["owner", "group", "others"] as const;
const WHAT = [
  ["read", 4],
  ["write", 2],
  ["execute", 1],
] as const;

function ChmodBody({
  initial,
  allowRecursive,
  done,
}: {
  initial: number;
  allowRecursive: boolean;
  done: (v: { mode: number; recursive: boolean } | null) => void;
}) {
  const [mode, setMode] = useState(initial & 0o7777);
  const [text, setText] = useState((initial & 0o777).toString(8).padStart(3, "0"));
  const [recursive, setRecursive] = useState(false);

  const toggle = (shift: number, bit: number) => {
    const next = mode ^ (bit << shift);
    setMode(next);
    setText((next & 0o777).toString(8).padStart(3, "0"));
  };

  return (
    <Modal
      title={t("chmod.title")}
      onClose={() => done(null)}
      width={400}
      footer={
        <>
          <button className="btn" onClick={() => done(null)}>
            {t("common.cancel")}
          </button>
          <button className="btn btn-primary" onClick={() => done({ mode, recursive })}>
            {t("common.ok")}
          </button>
        </>
      }
    >
      <table className="chmod">
        <thead>
          <tr>
            <th />
            {WHAT.map(([w]) => (
              <th key={w}>{t(`chmod.${w}`)}</th>
            ))}
          </tr>
        </thead>
        <tbody>
          {WHO.map((who, i) => {
            const shift = 6 - i * 3;
            return (
              <tr key={who}>
                <td>{t(`chmod.${who}`)}</td>
                {WHAT.map(([w, bit]) => (
                  <td key={w}>
                    <input type="checkbox" checked={!!(mode & (bit << shift))} onChange={() => toggle(shift, bit)} />
                  </td>
                ))}
              </tr>
            );
          })}
        </tbody>
      </table>
      <label className="field">
        <span>{t("chmod.numeric")}</span>
        <input
          className="input mono"
          value={text}
          maxLength={4}
          onChange={(e) => {
            const v = e.target.value.replace(/[^0-7]/g, "");
            setText(v);
            const n = parseInt(v || "0", 8);
            if (Number.isFinite(n)) setMode(n);
          }}
        />
      </label>
      {allowRecursive && (
        <label className="check">
          <input type="checkbox" checked={recursive} onChange={(e) => setRecursive(e.target.checked)} />
          <span>{t("chmod.recursive")}</span>
        </label>
      )}
    </Modal>
  );
}
