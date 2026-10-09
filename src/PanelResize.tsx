import { useLayoutEffect, useRef, useState, type KeyboardEvent, type PointerEvent } from 'react';
import './panel-resize.css';

type Side = 'left' | 'right';
export type PanelWidths = Record<Side, number>;
const key = 'supercode.panel-widths';
export const defaultWidths: PanelWidths = { left: 288, right: 284 };
export function decodePanelWidths(raw: string | null): PanelWidths {
  try {
    const value = JSON.parse(raw ?? '{}');
    return { left: valid(value?.left, 180, 580, defaultWidths.left), right: valid(value?.right, 240, 620, defaultWidths.right) };
  } catch { return { ...defaultWidths }; }
}
function valid(value: unknown, min: number, max: number, fallback: number) {
  return typeof value === 'number' && Number.isFinite(value) ? Math.max(min, Math.min(max, value)) : fallback;
}
export function panelLayout(widths: PanelWidths, available: number, left: boolean, right: boolean) {
  const overlay = window.matchMedia('(max-width: 1100px)').matches;
  const minLeft = Math.min(180, Math.max(120, available - 320));
  const rightWidth = Math.min(widths.right, Math.max(240, available - (left ? minLeft : 0) - 360));
  const leftWidth = Math.min(widths.left, Math.max(minLeft, available - (right && !overlay ? rightWidth : 0) - 360));
  return { left: leftWidth, right: Math.min(rightWidth, Math.max(0, available - 32)), overlay };
}

export function usePanelResize(left: boolean, right: boolean, disabled: boolean) {
  const root = useRef<HTMLDivElement>(null);
  const [widths, setWidths] = useState(() => decodePanelWidths(localStorage.getItem(key)));
  const preferred = useRef(widths);
  const options = useRef({ left, right, disabled });
  options.current = { left, right, disabled };
  const dragging = useRef<{ side: Side; x: number; initial: PanelWidths; width: number } | null>(null);
  function apply() {
    const el = root.current; if (!el) return;
    const o = options.current;
    const size = panelLayout(preferred.current, el.clientWidth, o.left, o.right);
    el.style.setProperty('--context-width', `${size.right}px`);
    el.style.gridTemplateColumns = o.disabled ? 'minmax(0,1fr)' : `${o.left ? `${size.left}px ` : ''}minmax(0,1fr)${o.right && !size.overlay ? ` ${size.right}px` : ''}`;
  }
  function commit() {
    setWidths({ ...preferred.current });
    try { localStorage.setItem(key, JSON.stringify(preferred.current)); } catch { /* Layout still works if storage is full. */ }
  }
  useLayoutEffect(() => {
    apply(); const observer = new ResizeObserver(apply); if (root.current) observer.observe(root.current);
    return () => { observer.disconnect(); root.current?.classList.remove('panel-resizing'); };
  }, [left, right, disabled]);
  function set(side: Side, value: number) {
    const el = root.current; if (!el) return;
    const size = panelLayout(preferred.current, el.clientWidth, left, right);
    const other = side === 'left' ? right && !size.overlay ? size.right : 0 : left ? size.left : 0;
    const min = side === 'left' ? Math.min(180, Math.max(120, el.clientWidth - 320)) : 240;
    const max = size.overlay && side === 'right' ? el.clientWidth - 32 : el.clientWidth - other - 360;
    preferred.current = { ...preferred.current, [side]: Math.max(min, Math.min(side === 'left' ? 580 : 620, Math.max(min, max), value)) };
    apply();
  }
  function finish(cancel = false) {
    const drag = dragging.current; if (!drag) return;
    if (cancel) preferred.current = drag.initial;
    dragging.current = null; root.current?.classList.remove('panel-resizing'); apply(); commit();
  }
  function handle(side: Side) {
    return {
      role: 'separator', tabIndex: 0, 'aria-orientation': 'vertical' as const,
      'aria-label': side === 'left' ? '调整侧栏宽度' : '调整工作区宽度',
      'aria-valuemin': side === 'left' ? 180 : 240, 'aria-valuemax': side === 'left' ? 580 : 620,
      'aria-valuenow': widths[side], className: `panel-resize-handle ${side}`,
      title: '拖动调整宽度，双击恢复默认',
      onPointerDown: (e: PointerEvent<HTMLDivElement>) => {
        if (e.button !== 0) return; e.preventDefault(); e.currentTarget.focus();
        e.currentTarget.setPointerCapture(e.pointerId);
        const size = panelLayout(preferred.current, root.current!.clientWidth, left, right);
        dragging.current = { side, x: e.clientX, initial: { ...preferred.current }, width: size[side] };
        root.current?.classList.add('panel-resizing');
      },
      onPointerMove: (e: PointerEvent<HTMLDivElement>) => {
        const drag = dragging.current; if (!drag || drag.side !== side) return;
        set(side, drag.width + (e.clientX - drag.x) * (side === 'left' ? 1 : -1));
      },
      onPointerUp: () => finish(), onPointerCancel: () => finish(true), onLostPointerCapture: () => finish(),
      onDoubleClick: () => { set(side, defaultWidths[side]); commit(); },
      onKeyDown: (e: KeyboardEvent<HTMLDivElement>) => {
        if (e.key === 'Escape') { finish(true); return; }
        if (!['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(e.key)) return;
        e.preventDefault();
        const size = panelLayout(preferred.current, root.current!.clientWidth, left, right);
        set(side, e.key === 'Home' ? 0 : e.key === 'End' ? 10000 : size[side] + (e.key === 'ArrowRight' ? 1 : -1) * (side === 'left' ? 1 : -1) * (e.shiftKey ? 40 : 10)); commit();
      },
    };
  }
  return { root, handle };
}
