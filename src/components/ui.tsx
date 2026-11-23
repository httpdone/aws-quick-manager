import { ReactNode, createContext, useCallback, useContext, useEffect, useMemo, useRef, useState } from "react";
import { useT } from "../i18n";
import { copyText } from "../lib/clipboard";
import { Tone } from "../lib/status";
import Icon, { IconName } from "./Icon";
import Modal from "./Modal";

export const cx = (...parts: (string | false | null | undefined)[]) => parts.filter(Boolean).join(" ");

// ---------- toasts and confirmations ----------

type ToastTone = "ok" | "bad" | "info";

interface ToastAction {
  label: string;
  run: () => void;
}

interface Toast {
  id: number;
  tone: ToastTone;
  text: string;
  action?: ToastAction;
}

export interface ConfirmOptions {
  title: string;
  message?: ReactNode;
  confirmLabel?: string;
  danger?: boolean;
}

interface UI {
  /** success and info fade out; errors stay until closed */
  toast: (text: string, tone?: ToastTone, action?: ToastAction) => void;
  confirm: (options: ConfirmOptions) => Promise<boolean>;
}

const UIContext = createContext<UI | null>(null);

export function useUI(): UI {
  const ui = useContext(UIContext);
  if (!ui) throw new Error("UIProvider is missing");
  return ui;
}

export function UIProvider({ children }: { children: ReactNode }) {
  const t = useT();
  const [toasts, setToasts] = useState<Toast[]>([]);
  const [pending, setPending] = useState<(ConfirmOptions & { resolve: (ok: boolean) => void }) | null>(null);
  const seq = useRef(0);

  const dismiss = useCallback((id: number) => setToasts((list) => list.filter((t) => t.id !== id)), []);

  const toast = useCallback(
    (text: string, tone: ToastTone = "ok", action?: ToastAction) => {
      if (!text) return;
      const id = ++seq.current;
      setToasts((list) => [...list.filter((t) => t.text !== text).slice(-3), { id, tone, text, action }]);
      if (tone !== "bad") window.setTimeout(() => dismiss(id), action ? 9000 : 4500);
    },
    [dismiss],
  );

  const confirm = useCallback(
    (options: ConfirmOptions) => new Promise<boolean>((resolve) => setPending({ ...options, resolve })),
    [],
  );

  const answer = (ok: boolean) => {
    pending?.resolve(ok);
    setPending(null);
  };

  const value = useMemo(() => ({ toast, confirm }), [toast, confirm]);

  return (
    <UIContext.Provider value={value}>
      {children}
      <div className="toasts" role="status" aria-live="polite">
        {toasts.map((item) => {
          const action = item.action;
          return (
            <div key={item.id} className={`toast ${item.tone}`}>
              <Icon name={item.tone === "bad" ? "alert" : item.tone === "ok" ? "check" : "info"} />
              <span className="toastText">{item.text}</span>
              {action && (
                <button
                  className="btn sm"
                  onClick={() => {
                    action.run();
                    dismiss(item.id);
                  }}
                >
                  {action.label}
                </button>
              )}
              <button className="iconBtn xs" onClick={() => dismiss(item.id)} aria-label={t.common.close}>
                <Icon name="x" size={14} />
              </button>
            </div>
          );
        })}
      </div>
      {pending && (
        <Modal
          title={pending.title}
          size="sm"
          onClose={() => answer(false)}
          footer={
            <>
              <button className="btn ghost" onClick={() => answer(false)}>
                {t.common.cancel}
              </button>
              <button className={`btn ${pending.danger ? "danger" : "primary"}`} onClick={() => answer(true)} autoFocus>
                {pending.confirmLabel ?? t.common.ok}
              </button>
            </>
          }
        >
          {typeof pending.message === "string" ? <p className="prewrap">{pending.message}</p> : pending.message}
        </Modal>
      )}
    </UIContext.Provider>
  );
}

// ---------- small pieces ----------

export const Spinner = ({ size = 14 }: { size?: number }) => (
  <span className="spinner" style={{ width: size, height: size }} aria-hidden="true" />
);

export const StatusPill = ({ tone, label }: { tone: Tone; label: string }) => (
  <span className={`pill ${tone}`}>
    <i />
    {label}
  </span>
);

export function Toggle({
  checked,
  onChange,
  disabled,
  label,
}: {
  checked: boolean;
  onChange: (on: boolean) => void;
  disabled?: boolean;
  label: string;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      className={cx("toggle", checked && "on")}
      disabled={disabled}
      onClick={() => onChange(!checked)}
    >
      <span />
    </button>
  );
}

export interface SegmentOption<T extends string> {
  value: T;
  label: ReactNode;
  count?: number;
  title?: string;
}

export function Segmented<T extends string>({
  value,
  options,
  onChange,
  disabled,
}: {
  value: T;
  options: SegmentOption<T>[];
  onChange: (v: T) => void;
  disabled?: boolean;
}) {
  return (
    <div className="segmented" role="tablist">
      {options.map((o) => (
        <button
          key={o.value}
          type="button"
          role="tab"
          aria-selected={o.value === value}
          className={cx(o.value === value && "on")}
          onClick={() => onChange(o.value)}
          disabled={disabled}
          title={o.title}
        >
          {o.label}
          {o.count !== undefined && <em>{o.count}</em>}
        </button>
      ))}
    </div>
  );
}

export function Empty({ icon, title, children }: { icon: IconName; title: string; children?: ReactNode }) {
  return (
    <div className="empty">
      <div className="emptyIcon">
        <Icon name={icon} size={22} />
      </div>
      <b>{title}</b>
      {children}
    </div>
  );
}

export function Alert({ tone, children, icon }: { tone: "bad" | "warn" | "info" | "ok"; children: ReactNode; icon?: IconName }) {
  return (
    <div className={`alert ${tone}`}>
      <Icon name={icon ?? (tone === "ok" ? "check" : tone === "info" ? "info" : "alert")} />
      <div>{children}</div>
    </div>
  );
}

export function CopyButton({ text, label }: { text: string; label?: string }) {
  const t = useT();
  const [done, setDone] = useState(false);
  useEffect(() => {
    if (!done) return;
    const timer = window.setTimeout(() => setDone(false), 1500);
    return () => window.clearTimeout(timer);
  }, [done]);
  return (
    <button
      type="button"
      className={cx(label ? "btn ghost sm" : "iconBtn xs", done && "copied")}
      title={done ? t.common.copied : t.common.copy}
      disabled={!text}
      onClick={async () => setDone(await copyText(text))}
    >
      <Icon name={done ? "check" : "copy"} size={13} />
      {label && (done ? t.common.copied : label)}
    </button>
  );
}

/** A label / value row; mono values get a copy button. */
export function Field({ label, value, mono, copy, children }: { label: string; value?: string; mono?: boolean; copy?: boolean; children?: ReactNode }) {
  return (
    <div className="field">
      <span className="fieldLabel">{label}</span>
      <span className={cx("fieldValue", mono && "mono")}>
        {children ?? (value || "—")}
        {copy && value ? <CopyButton text={value} /> : null}
      </span>
    </div>
  );
}

export function Avatar({ name, hue, size = 36 }: { name: string; hue: number; size?: number }) {
  const chars = [...name.trim()];
  return (
    <span className="avatar" style={{ width: size, height: size, fontSize: size * 0.42, background: `hsl(${hue} 62% 52%)` }} aria-hidden="true">
      {chars.length ? chars[0].toUpperCase() : "?"}
    </span>
  );
}
