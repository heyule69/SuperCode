import { useLayoutEffect, useRef, useState, type KeyboardEvent, type PointerEvent } from 'react';
import './panel-resize.css';

type Side = 'left' | 'right';
type Panel = 'workspace' | 'browser' | 'conversation' | 'content';
export type PanelWidths = Record<Side, number>;
const key = 'supercode.panel-widths';
export const defaultWidths: PanelWidths = { left: 288, right: 284 };
export function decodePanelWidths(raw: string | null, rightDefault = defaultWidths.right): PanelWidths {
  try {
    const value = JSON.parse(raw ?? '{}');
    return { left: valid(value?.left, 180, 580, defaultWidths.left), right: valid(value?.right, 240, 620, rightDefault) };
  } catch { return { ...defaultWidths, right: rightDefault }; }
}
function valid(value: unknown, min: number, max: number, fallback: number) {
  return typeof value === 'number' && Number.isFinite(value) ? Math.max(min, Math.min(max, value)) : fallback;
}
export function panelLayout(widths: PanelWidths, available: number, left: boolean, right: boolean, panel: Panel = 'workspace') {
  const overlay = window.matchMedia(panel === 'content' ? '(max-width: 800px)' : '(max-width: 1100px)').matches;
  const minLeft = Math.min(180, Math.max(120, available - 320));
  const rightWidth = Math.min(widths.right, Math.max(240, available - (overlay ? 32 : (left ? minLeft : 0) + 360)));
  const leftWidth = Math.min(widths.left, Math.max(minLeft, available - (right && !overlay ? rightWidth : 0) - 360));
  return { left: leftWidth, right: Math.min(rightWidth, Math.max(0, available - 32)), overlay };
}

export function usePanelResize(left: boolean, right: boolean, disabled: boolean, panel: Panel = 'workspace', expanded = false) {
  const storageKey = panel === 'workspace' ? key : `${key}.${panel}`;
  const rightDefault = panel === 'workspace' ? defaultWidths.right : 520;
  const root = useRef<HTMLDivElement>(null);
  const [widths, setWidths] = useState(() => ({ ...decodePanelWidths(localStorage.getItem(key)), right:decodePanelWidths(localStorage.getItem(storageKey), rightDefault).right }));
  const preferred = useRef(widths);
  const options = useRef({ left, right, disabled, expanded });
  options.current = { left, right, disabled, expanded };
  const dragging = useRef<{ side: Side; x: number; initial: PanelWidths; width: number } | null>(null);
  function apply() {
    const el = root.current; if (!el) return;
    const o = options.current;
    const size = panelLayout(preferred.current, el.clientWidth, o.left, o.right, panel);
    const rightWidth = o.expanded ? Math.max(size.right, Math.min(820, el.clientWidth - (o.left ? size.left : 0) - (size.overlay ? 32 : 360))) : size.right;
    el.style.setProperty('--context-width', `${rightWidth}px`);
    // Full-page settings span all tracks. Keep the hidden chat's columns intact
    // so returning to it never animates its sidebar through a zero-width column.
    el.style.gridTemplateColumns = `${o.left ? size.left : 0}px minmax(0,1fr) ${o.right && !size.overlay ? rightWidth : 0}px`;
  }
  function commit() {
    setWidths({ ...preferred.current });
    try {
      localStorage.setItem(storageKey, JSON.stringify(preferred.current));
      if (panel !== 'workspace') localStorage.setItem(key, JSON.stringify({...decodePanelWidths(localStorage.getItem(key)),left:preferred.current.left}));
    } catch { /* Layout still works if storage is full. */ }
  }
  useLayoutEffect(()=>{
    const el=root.current;
    const transition=el?.style.transition;
    // Restore saved tracks before the first paint, without animating from CSS
    // defaults after apply() measures the container and flushes its old layout.
    if(el)el.style.transition='none';
    const saved=localStorage.getItem(storageKey);
    preferred.current={...preferred.current,right:decodePanelWidths(saved, rightDefault).right};
    setWidths({...preferred.current});apply();
    if(el){el.getBoundingClientRect();if(transition)el.style.transition=transition;else el.style.removeProperty('transition');}
  },[panel]);
  useLayoutEffect(() => {
    apply(); const observer = new ResizeObserver(apply); if (root.current) observer.observe(root.current);
    return () => { observer.disconnect(); root.current?.classList.remove('panel-resizing'); };
  }, [left, right, disabled, expanded]);
  function set(side: Side, value: number) {
    const el = root.current; if (!el) return;
    const size = panelLayout(preferred.current, el.clientWidth, left, right, panel);
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
      'aria-label': side === 'left' ? '调整侧栏宽度' : panel === 'browser' ? '调整浏览器宽度' : panel === 'conversation' ? '调整对话面板宽度' : panel === 'content' ? '调整右侧栏宽度' : '调整工作区宽度',
      'aria-valuemin': side === 'left' ? 180 : 240, 'aria-valuemax': side === 'left' ? 580 : 620,
      'aria-valuenow': widths[side], className: `panel-resize-handle ${side}`,
      title: '拖动调整宽度，双击恢复默认',
      onPointerDown: (e: PointerEvent<HTMLDivElement>) => {
        if (e.button !== 0) return; e.preventDefault(); e.currentTarget.focus();
        e.currentTarget.setPointerCapture(e.pointerId);
        const size = panelLayout(preferred.current, root.current!.clientWidth, left, right, panel);
        dragging.current = { side, x: e.clientX, initial: { ...preferred.current }, width: size[side] };
        root.current?.classList.add('panel-resizing');
      },
      onPointerMove: (e: PointerEvent<HTMLDivElement>) => {
        const drag = dragging.current; if (!drag || drag.side !== side) return;
        set(side, drag.width + (e.clientX - drag.x) * (side === 'left' ? 1 : -1));
      },
      onPointerUp: () => finish(), onPointerCancel: () => finish(true), onLostPointerCapture: () => finish(),
      onDoubleClick: () => { set(side, side === 'right' ? rightDefault : defaultWidths.left); commit(); },
      onKeyDown: (e: KeyboardEvent<HTMLDivElement>) => {
        if (e.key === 'Escape') { finish(true); return; }
        if (!['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(e.key)) return;
        e.preventDefault();
        const size = panelLayout(preferred.current, root.current!.clientWidth, left, right, panel);
        set(side, e.key === 'Home' ? 0 : e.key === 'End' ? 10000 : size[side] + (e.key === 'ArrowRight' ? 1 : -1) * (side === 'left' ? 1 : -1) * (e.shiftKey ? 40 : 10)); commit();
      },
    };
  }
  return { root, handle };
}
