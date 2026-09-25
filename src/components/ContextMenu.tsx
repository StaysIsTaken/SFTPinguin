import { ReactNode, useEffect, useLayoutEffect, useRef, useState } from "react";
import { create } from "zustand";

export interface MenuItem {
  label?: string;
  icon?: ReactNode;
  shortcut?: string;
  onClick?: () => void;
  danger?: boolean;
  disabled?: boolean;
  separator?: boolean;
}

const useMenu = create<{ x: number; y: number; items: MenuItem[] | null }>(() => ({ x: 0, y: 0, items: null }));

export function openContextMenu(x: number, y: number, items: MenuItem[]) {
  useMenu.setState({ x, y, items });
}

export function closeContextMenu() {
  useMenu.setState({ items: null });
}

export function ContextMenuHost() {
  const { x, y, items } = useMenu();
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ x, y });

  useLayoutEffect(() => {
    if (!items || !ref.current) return;
    const rect = ref.current.getBoundingClientRect();
    const nx = Math.max(8, Math.min(x, window.innerWidth - rect.width - 8));
    const ny = Math.max(8, Math.min(y, window.innerHeight - rect.height - 8));
    setPos({ x: nx, y: ny });
  }, [x, y, items]);

  useEffect(() => {
    if (!items) return;
    const close = () => closeContextMenu();
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && close();
    window.addEventListener("resize", close);
    window.addEventListener("blur", close);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("resize", close);
      window.removeEventListener("blur", close);
      window.removeEventListener("keydown", onKey);
    };
  }, [items]);

  if (!items) return null;
  return (
    <div
      className="menu-backdrop"
      onMouseDown={closeContextMenu}
      onContextMenu={(e) => {
        e.preventDefault();
        closeContextMenu();
      }}
    >
      <div className="menu" ref={ref} style={{ left: pos.x, top: pos.y }} onMouseDown={(e) => e.stopPropagation()}>
        {items.map((item, i) =>
          item.separator ? (
            <div key={i} className="menu-sep" />
          ) : (
            <button
              key={i}
              className={`menu-item ${item.danger ? "danger" : ""}`}
              disabled={item.disabled}
              onClick={() => {
                closeContextMenu();
                item.onClick?.();
              }}
            >
              <span className="menu-icon">{item.icon}</span>
              <span className="menu-label">{item.label}</span>
              {item.shortcut && <span className="menu-shortcut">{item.shortcut}</span>}
            </button>
          ),
        )}
      </div>
    </div>
  );
}
