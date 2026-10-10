import { useLayoutEffect, useRef, useState, type KeyboardEvent, type PointerEvent } from 'react';
import './browser-dock.css';

type Size = { width: number; height: number };
const storageKey = 'supercode.browser-size';
const defaults: Size = { width: 520, height: 360 };
function bounded(value: unknown, min: number, max: number, fallback: number) {
  return typeof value === 'number' && Number.isFinite(value) ? Math.max(min, Math.min(max, value)) : fallback;
}
export function browserDockSize(raw: string | null): Size {
  try {
    const value = JSON.parse(raw ?? '{}');
    return { width: bounded(value?.width, 320, 960, defaults.width), height: bounded(value?.height, 220, 800, defaults.height) };
  } catch { return { ...defaults }; }
}
export function browserDockLayout(size: Size, width: number, height: number) {
  const stacked = width < 740;
  return {
    stacked,
    extent: stacked ? Math.min(size.height, Math.max(220, height - 280)) : Math.min(size.width, Math.max(320, width - 380)),
  };
}
export function useBrowserDock(open: boolean, expanded: boolean) {
  const root = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState(() => browserDockSize(localStorage.getItem(storageKey)));
  const [stacked, setStacked] = useState(false);
  const preferred = useRef(size);
  const options = useRef({ open, expanded }); options.current = { open, expanded };
  const dragging = useRef<{ initial: Size; extent: number; coordinate: number; stacked: boolean } | null>(null);
  function apply() {
    const el = root.current; if (!el) return;
    const layout = browserDockLayout(preferred.current, el.clientWidth, el.clientHeight);
    setStacked(layout.stacked);
    el.classList.toggle('browser-stacked', layout.stacked);
    el.style.setProperty('--browser-extent', `${layout.extent}px`);
  }
  useLayoutEffect(() => {
    apply(); const observer = new ResizeObserver(apply); if (root.current) observer.observe(root.current);
    return () => { observer.disconnect(); root.current?.classList.remove('browser-resizing'); };
  }, [open, expanded]);
  function commit() {
    setSize({ ...preferred.current });
    try { localStorage.setItem(storageKey, JSON.stringify(preferred.current)); } catch { /* Resizing still works without storage. */ }
  }
  function set(value: number, vertical = stacked) {
    const el = root.current; if (!el) return;
    const dimension = vertical ? 'height' : 'width';
    const min = vertical ? 220 : 320;
    const max = vertical ? Math.max(min, el.clientHeight - 280) : Math.max(min, el.clientWidth - 380);
    preferred.current = { ...preferred.current, [dimension]: bounded(value, min, Math.min(vertical ? 800 : 960, max), defaults[dimension]) };
    apply();
  }
  function finish(cancel = false) {
    const drag = dragging.current; if (!drag) return;
    if (cancel) preferred.current = drag.initial;
    dragging.current = null; root.current?.classList.remove('browser-resizing'); apply(); commit();
  }
  const handle = {
    role: 'separator', tabIndex: 0, 'aria-orientation': stacked ? 'horizontal' as const : 'vertical' as const,
    'aria-label': stacked ? '调整浏览器高度' : '调整浏览器宽度',
    'aria-valuemin': stacked ? 220 : 320, 'aria-valuemax': stacked ? 800 : 960,
    'aria-valuenow': stacked ? size.height : size.width,
    className: `browser-resize-handle${expanded ? ' browser-resize-hidden' : ''}`,
    title: '拖动调整浏览器大小，双击恢复默认',
    onPointerDown: (e: PointerEvent<HTMLDivElement>) => {
      if (e.button !== 0 || !options.current.open || options.current.expanded) return;
      e.preventDefault(); e.currentTarget.focus(); e.currentTarget.setPointerCapture(e.pointerId);
      const el = root.current!; const layout = browserDockLayout(preferred.current, el.clientWidth, el.clientHeight);
      dragging.current = { initial: { ...preferred.current }, extent: layout.extent, coordinate: layout.stacked ? e.clientY : e.clientX, stacked: layout.stacked };
      root.current?.classList.add('browser-resizing');
    },
    onPointerMove: (e: PointerEvent<HTMLDivElement>) => {
      const drag = dragging.current; if (!drag) return;
      set(drag.extent + drag.coordinate - (drag.stacked ? e.clientY : e.clientX), drag.stacked);
    },
    onPointerUp: () => finish(), onPointerCancel: () => finish(true), onLostPointerCapture: () => finish(),
    onDoubleClick: () => { set(stacked ? defaults.height : defaults.width); commit(); },
    onKeyDown: (e: KeyboardEvent<HTMLDivElement>) => {
      if (e.key === 'Escape') { finish(true); return; }
      const arrows = stacked ? ['ArrowUp', 'ArrowDown'] : ['ArrowLeft', 'ArrowRight'];
      if (![...arrows, 'Home', 'End'].includes(e.key) || options.current.expanded) return;
      e.preventDefault(); const el = root.current!; const layout = browserDockLayout(preferred.current, el.clientWidth, el.clientHeight);
      set(e.key === 'Home' ? 0 : e.key === 'End' ? 10000 : layout.extent + (e.key === arrows[0] ? 1 : -1) * (e.shiftKey ? 40 : 10)); commit();
    },
  };
  return { root, handle };
}
