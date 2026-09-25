import { useMemo, useState } from "react";
import {
  ArrowDown,
  ArrowUp,
  Ban,
  ChevronDown,
  ChevronUp,
  CircleCheck,
  CircleX,
  FolderOpen,
  ListChecks,
  RotateCw,
  ScrollText,
  Trash2,
  X,
} from "lucide-react";
import { api, TransferInfo } from "../lib/api";
import { useT } from "../lib/i18n";
import { useStore } from "../lib/store";
import { formatEta, formatSize, formatSpeed, formatTime, lparent } from "../lib/format";
import { showError } from "../lib/actions";

export function TransferPanel() {
  const t = useT();
  const open = useStore((s) => s.settings.panelOpen);
  const height = useStore((s) => s.settings.panelHeight);
  const update = useStore((s) => s.updateSettings);
  const transfers = useStore((s) => s.transfers);
  const order = useStore((s) => s.transferOrder);
  const logs = useStore((s) => s.logs);
  const [tab, setTab] = useState<"transfers" | "log">("transfers");
  const lang = navigator.language;

  const list = useMemo(() => order.map((id) => transfers[id]).filter(Boolean), [order, transfers]);
  const stats = useMemo(() => {
    let running = 0,
      queued = 0,
      failed = 0,
      total = 0,
      done = 0,
      speed = 0;
    for (const x of list) {
      if (x.status === "running") {
        running++;
        speed += x.speed;
      }
      if (x.status === "queued") queued++;
      if (x.status === "failed") failed++;
      if (x.status === "running" || x.status === "queued" || x.status === "done") {
        total += x.size;
        done += x.status === "done" ? x.size : x.transferred;
      }
    }
    return { running, queued, failed, total, done, speed };
  }, [list]);

  const active = stats.running + stats.queued;
  const overall = stats.total ? Math.min(100, (stats.done / stats.total) * 100) : 0;

  const startResize = (e: React.PointerEvent) => {
    e.preventDefault();
    const startY = e.clientY;
    const startH = height;
    const move = (ev: PointerEvent) => {
      const h = Math.min(window.innerHeight * 0.7, Math.max(120, startH + (startY - ev.clientY)));
      update({ panelHeight: Math.round(h) });
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      document.body.classList.remove("resizing-v");
    };
    document.body.classList.add("resizing-v");
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  };

  const clearFinished = async () => {
    await api.clearTransfers(false).catch(showError);
    useStore.getState().pruneTransfers((x) => x.status === "running" || x.status === "queued");
  };

  return (
    <div className={`panel ${open ? "open" : ""}`} style={open ? { height } : undefined}>
      {open && <div className="panel-resizer" onPointerDown={startResize} />}
      <div className="panel-header" onDoubleClick={() => update({ panelOpen: !open })}>
        <div className="panel-tabs">
          <button
            className={tab === "transfers" ? "active" : ""}
            onClick={() => {
              setTab("transfers");
              if (!open) update({ panelOpen: true });
            }}
          >
            <ListChecks size={14} /> {t("transfers.title")}
            {active > 0 && <span className="badge">{active}</span>}
            {stats.failed > 0 && <span className="badge badge-danger">{stats.failed}</span>}
          </button>
          <button
            className={tab === "log" ? "active" : ""}
            onClick={() => {
              setTab("log");
              if (!open) update({ panelOpen: true });
            }}
          >
            <ScrollText size={14} /> {t("transfers.log")}
          </button>
        </div>
        {active > 0 && (
          <div className="panel-summary">
            <div className="mini-progress">
              <div style={{ width: `${overall}%` }} />
            </div>
            <span className="muted small">
              {stats.running > 0 && t("transfers.active", { count: stats.running })}
              {stats.queued > 0 && ` · ${t("transfers.queued", { count: stats.queued })}`}
              {stats.speed > 0 && ` · ${formatSpeed(stats.speed, lang)}`}
            </span>
          </div>
        )}
        <div className="spacer" />
        {open && tab === "transfers" && (
          <>
            {active > 0 && (
              <button className="btn btn-ghost btn-sm" onClick={() => api.cancelAllTransfers().catch(showError)}>
                <Ban size={13} /> <span className="hide-narrow">{t("transfers.cancelAll")}</span>
              </button>
            )}
            {list.length > active && (
              <button className="btn btn-ghost btn-sm" onClick={clearFinished}>
                <Trash2 size={13} /> <span className="hide-narrow">{t("transfers.clearFinished")}</span>
              </button>
            )}
          </>
        )}
        {open && tab === "log" && logs.length > 0 && (
          <button className="btn btn-ghost btn-sm" onClick={() => useStore.getState().clearLogs()}>
            <Trash2 size={13} />
          </button>
        )}
        <button className="icon-btn" onClick={() => update({ panelOpen: !open })}>
          {open ? <ChevronDown size={16} /> : <ChevronUp size={16} />}
        </button>
      </div>
      {open && (
        <div className="panel-body">
          {tab === "transfers" ? (
            list.length === 0 ? (
              <div className="panel-empty muted">{t("transfers.empty")}</div>
            ) : (
              <div className="transfer-list">
                {[...list].reverse().map((x) => (
                  <TransferRow key={x.id} x={x} lang={lang} />
                ))}
              </div>
            )
          ) : logs.length === 0 ? (
            <div className="panel-empty muted">{t("transfers.logEmpty")}</div>
          ) : (
            <div className="log-list selectable">
              {[...logs].reverse().map((l, i) => (
                <div key={i} className={`log-row log-${l.level}`}>
                  <span className="log-time">{formatTime(l.time, lang)}</span>
                  <span className="log-msg">{l.message}</span>
                </div>
              ))}
            </div>
          )}
        </div>
      )}
    </div>
  );
}

function TransferRow({ x, lang }: { x: TransferInfo; lang: string }) {
  const t = useT();
  const pct = x.size ? Math.min(100, (x.transferred / x.size) * 100) : x.status === "done" ? 100 : 0;
  const Dir = x.direction === "upload" ? ArrowUp : ArrowDown;
  return (
    <div className={`transfer status-${x.status}`}>
      <Dir size={14} className={`dir-icon ${x.direction}`} />
      <div className="transfer-main">
        <div className="transfer-top">
          <span className="transfer-name" title={x.direction === "upload" ? x.remotePath : x.localPath}>
            {x.name}
          </span>
          <span className="transfer-meta muted">
            {x.status === "running" ? (
              <>
                {formatSize(x.transferred, lang)} / {formatSize(x.size, lang)}
                {x.speed > 0 && ` · ${formatSpeed(x.speed, lang)} · ${formatEta(x.size - x.transferred, x.speed)}`}
              </>
            ) : x.status === "failed" ? (
              <span className="text-danger" title={x.error ?? ""}>
                {x.error}
              </span>
            ) : (
              <>
                {formatSize(x.size, lang)} · {t(`transfers.status.${x.status}`)}
              </>
            )}
          </span>
        </div>
        {(x.status === "running" || x.status === "queued") && (
          <div className="progress">
            <div style={{ width: `${pct}%` }} />
          </div>
        )}
      </div>
      <div className="transfer-actions">
        {x.status === "done" && <CircleCheck size={15} className="text-success" />}
        {x.status === "failed" && <CircleX size={15} className="text-danger" />}
        {x.status === "done" && x.direction === "download" && (
          <button className="icon-btn" title={t("transfers.showInFolder")} onClick={() => api.openLocalPath(lparent(x.localPath)).catch(showError)}>
            <FolderOpen size={14} />
          </button>
        )}
        {(x.status === "failed" || x.status === "cancelled") && (
          <button className="icon-btn" title={t("transfers.retry")} onClick={() => api.retryTransfer(x.id).catch(showError)}>
            <RotateCw size={14} />
          </button>
        )}
        {(x.status === "running" || x.status === "queued") && (
          <button className="icon-btn" title={t("transfers.cancel")} onClick={() => api.cancelTransfer(x.id).catch(showError)}>
            <X size={14} />
          </button>
        )}
      </div>
    </div>
  );
}
