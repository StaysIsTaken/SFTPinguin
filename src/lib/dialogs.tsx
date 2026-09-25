import { ReactNode, useEffect, useRef, useState } from "react";
import { create } from "zustand";
import { AlertTriangle, X } from "lucide-react";
import { t } from "./i18n";

interface DialogEntry {
  id: number;
  node: ReactNode;
}

const useDialogStore = create<{ dialogs: DialogEntry[] }>(() => ({ dialogs: [] }));

let nextId = 1;

/** Opens a dialog; `render` gets a `done` callback which closes it and resolves the promise. */
export function openDialog<T>(render: (done: (value: T) => void) => ReactNode): Promise<T> {
  return new Promise<T>((resolve) => {
    const id = nextId++;
    const done = (value: T) => {
      useDialogStore.setState((s) => ({ dialogs: s.dialogs.filter((d) => d.id !== id) }));
      resolve(value);
    };
    useDialogStore.setState((s) => ({ dialogs: [...s.dialogs, { id, node: render(done) }] }));
  });
}

export function DialogHost() {
  const dialogs = useDialogStore((s) => s.dialogs);
  return (
    <>
      {dialogs.map((d) => (
        <div key={d.id}>{d.node}</div>
      ))}
    </>
  );
}

export function Modal({
  title,
  onClose,
  children,
  footer,
  width = 440,
  danger,
  icon,
}: {
  title: ReactNode;
  onClose: () => void;
  children: ReactNode;
  footer?: ReactNode;
  width?: number;
  danger?: boolean;
  icon?: ReactNode;
}) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [onClose]);

  return (
    <div className="modal-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className={`modal ${danger ? "modal-danger" : ""}`} style={{ maxWidth: width }} role="dialog" aria-modal>
        <div className="modal-header">
          {icon}
          <h2>{title}</h2>
          <button className="icon-btn" onClick={onClose} aria-label={t("common.close")}>
            <X size={16} />
          </button>
        </div>
        <div className="modal-body">{children}</div>
        {footer && <div className="modal-footer">{footer}</div>}
      </div>
    </div>
  );
}

export function confirmDialog(opts: {
  title: string;
  message?: ReactNode;
  confirmLabel?: string;
  danger?: boolean;
}): Promise<boolean> {
  return openDialog<boolean>((done) => (
    <Modal
      title={opts.title}
      onClose={() => done(false)}
      danger={opts.danger}
      icon={opts.danger ? <AlertTriangle size={18} className="text-danger" /> : undefined}
      footer={
        <>
          <button className="btn" onClick={() => done(false)}>
            {t("common.cancel")}
          </button>
          <button className={`btn ${opts.danger ? "btn-danger" : "btn-primary"}`} autoFocus onClick={() => done(true)}>
            {opts.confirmLabel ?? t("common.ok")}
          </button>
        </>
      }
    >
      {opts.message && <div className="dialog-message">{opts.message}</div>}
    </Modal>
  ));
}

function PromptBody({
  label,
  initial,
  selectStem,
  password,
  onSubmit,
  onCancel,
  title,
  confirmLabel,
  checkbox,
  hint,
}: {
  label: string;
  initial: string;
  selectStem?: boolean;
  password?: boolean;
  title: string;
  confirmLabel?: string;
  checkbox?: { label: string; initial: boolean };
  hint?: ReactNode;
  onSubmit: (v: string, checked: boolean) => void;
  onCancel: () => void;
}) {
  const [value, setValue] = useState(initial);
  const [checked, setChecked] = useState(checkbox?.initial ?? false);
  const ref = useRef<HTMLInputElement>(null);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    el.focus();
    if (selectStem) {
      const dot = initial.lastIndexOf(".");
      el.setSelectionRange(0, dot > 0 ? dot : initial.length);
    } else {
      el.select();
    }
  }, [initial, selectStem]);

  return (
    <Modal
      title={title}
      onClose={onCancel}
      footer={
        <>
          <button className="btn" onClick={onCancel}>
            {t("common.cancel")}
          </button>
          <button className="btn btn-primary" onClick={() => onSubmit(value, checked)} disabled={!password && !value.trim()}>
            {confirmLabel ?? t("common.ok")}
          </button>
        </>
      }
    >
      {hint && <div className="dialog-message">{hint}</div>}
      <label className="field">
        <span>{label}</span>
        <input
          ref={ref}
          className="input"
          type={password ? "password" : "text"}
          value={value}
          autoComplete="off"
          autoCapitalize="off"
          spellCheck={false}
          onChange={(e) => setValue(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && (password || value.trim())) onSubmit(value, checked);
          }}
        />
      </label>
      {checkbox && (
        <label className="check">
          <input type="checkbox" checked={checked} onChange={(e) => setChecked(e.target.checked)} />
          <span>{checkbox.label}</span>
        </label>
      )}
    </Modal>
  );
}

export function promptDialog(opts: {
  title: string;
  label: string;
  initial?: string;
  selectStem?: boolean;
  confirmLabel?: string;
}): Promise<string | null> {
  return openDialog<string | null>((done) => (
    <PromptBody
      title={opts.title}
      label={opts.label}
      initial={opts.initial ?? ""}
      selectStem={opts.selectStem}
      confirmLabel={opts.confirmLabel}
      onSubmit={(v) => done(v.trim())}
      onCancel={() => done(null)}
    />
  ));
}

export function passwordDialog(opts: {
  title: string;
  label: string;
  hint?: ReactNode;
  remember?: boolean;
}): Promise<{ password: string; remember: boolean } | null> {
  return openDialog((done) => (
    <PromptBody
      title={opts.title}
      label={opts.label}
      initial=""
      password
      hint={opts.hint}
      confirmLabel={t("common.connect")}
      checkbox={opts.remember !== undefined ? { label: t("connect.remember"), initial: opts.remember } : undefined}
      onSubmit={(v, checked) => done({ password: v, remember: checked })}
      onCancel={() => done(null)}
    />
  ));
}

export interface Choice<T extends string> {
  value: T;
  label: string;
  hint?: string;
  icon?: ReactNode;
  primary?: boolean;
}

export function choiceDialog<T extends string>(opts: {
  title: string;
  message?: ReactNode;
  choices: Choice<T>[];
}): Promise<{ value: T } | null> {
  return openDialog((done) => (
    <Modal
      title={opts.title}
      onClose={() => done(null)}
      width={460}
      footer={
        <button className="btn" onClick={() => done(null)}>
          {t("common.cancel")}
        </button>
      }
    >
      {opts.message && <div className="dialog-message">{opts.message}</div>}
      <div className="choices">
        {opts.choices.map((c) => (
          <button
            key={c.value}
            className={`choice ${c.primary ? "primary" : ""}`}
            autoFocus={c.primary}
            onClick={() => done({ value: c.value })}
          >
            {c.icon && <span className="choice-icon">{c.icon}</span>}
            <span className="choice-text">
              <span className="choice-label">{c.label}</span>
              {c.hint && <span className="choice-hint">{c.hint}</span>}
            </span>
          </button>
        ))}
      </div>
    </Modal>
  ));
}
