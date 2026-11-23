import { ReactNode, useEffect, useId, useRef } from "react";
import Icon from "./Icon";

interface Props {
  title: ReactNode;
  subtitle?: ReactNode;
  onClose: () => void;
  children: ReactNode;
  footer?: ReactNode;
  size?: "sm" | "md" | "lg";
  /** ignore Esc / backdrop clicks and hide the close button (while busy) */
  locked?: boolean;
}

// Only the top-most dialog answers Esc.
const stack: number[] = [];
let nextId = 0;

export default function Modal({ title, subtitle, onClose, children, footer, size = "md", locked }: Props) {
  const labelId = useId();
  const close = useRef(onClose);
  const lockedRef = useRef(locked);
  close.current = onClose;
  lockedRef.current = locked;

  useEffect(() => {
    const id = ++nextId;
    stack.push(id);
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || stack[stack.length - 1] !== id) return;
      e.stopPropagation();
      if (!lockedRef.current) close.current();
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
      const at = stack.indexOf(id);
      if (at >= 0) stack.splice(at, 1);
    };
  }, []);

  return (
    <div
      className="overlay"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget && !locked) onClose();
      }}
    >
      <div className={`dialog ${size}`} role="dialog" aria-modal="true" aria-labelledby={labelId}>
        <div className="dialogHead">
          <div className="dialogTitle">
            <h2 id={labelId}>{title}</h2>
            {subtitle && <p>{subtitle}</p>}
          </div>
          {!locked && (
            <button className="iconBtn" onClick={onClose} aria-label="关闭">
              <Icon name="x" />
            </button>
          )}
        </div>
        <div className="dialogBody">{children}</div>
        {footer && <div className="dialogFoot">{footer}</div>}
      </div>
    </div>
  );
}
