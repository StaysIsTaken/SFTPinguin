import { CheckCircle2, Info, X, XCircle } from "lucide-react";
import { useStore } from "../lib/store";

export function Toasts() {
  const toasts = useStore((s) => s.toasts);
  const dismiss = useStore((s) => s.dismissToast);
  return (
    <div className="toasts" aria-live="polite">
      {toasts.map((t) => (
        <div key={t.id} className={`toast toast-${t.kind}`}>
          {t.kind === "success" ? <CheckCircle2 size={16} /> : t.kind === "error" ? <XCircle size={16} /> : <Info size={16} />}
          <div className="toast-body">
            <div className="toast-msg">{t.message}</div>
            {(t.action || t.secondary) && (
              <div className="toast-actions">
                {t.action && (
                  <button
                    className="btn btn-sm btn-primary"
                    onClick={() => {
                      t.action!.run();
                      dismiss(t.id);
                    }}
                  >
                    {t.action.label}
                  </button>
                )}
                {t.secondary && (
                  <button
                    className="btn btn-sm"
                    onClick={() => {
                      t.secondary!.run();
                      dismiss(t.id);
                    }}
                  >
                    {t.secondary.label}
                  </button>
                )}
              </div>
            )}
          </div>
          <button className="icon-btn" onClick={() => dismiss(t.id)}>
            <X size={14} />
          </button>
        </div>
      ))}
    </div>
  );
}
