import { useEffect, useId, useLayoutEffect, useRef, type KeyboardEvent } from 'react';

export function useFloatingLayer(open: boolean, close: () => void, { focusFirst = false, position = true } = {}) {
  const root = useRef<HTMLDivElement>(null);
  const id = useId();
  const latestClose = useRef(close); latestClose.current = close;
  const dismiss = (restoreFocus = false) => {
    latestClose.current();
    if (restoreFocus) root.current?.querySelector<HTMLButtonElement>(':scope > button')?.focus();
  };
  useEffect(() => {
    if (!open) return;
    document.dispatchEvent(new CustomEvent('supercode:layer-open', { detail: id }));
    const outside = (event: Event) => { if (!root.current?.contains(event.target as Node)) dismiss(); };
    const another = (event: Event) => { if ((event as CustomEvent).detail !== id) dismiss(); };
    const escape = (event: globalThis.KeyboardEvent) => { if (event.key === 'Escape') { event.preventDefault(); event.stopImmediatePropagation(); dismiss(true); } };
    document.addEventListener('pointerdown', outside); document.addEventListener('focusin', outside);
    document.addEventListener('supercode:layer-open', another); document.addEventListener('keydown', escape, true);
    if (focusFirst) root.current?.querySelector<HTMLElement>('[role^="menuitem"]')?.focus();
    return () => {
      document.removeEventListener('pointerdown', outside); document.removeEventListener('focusin', outside);
      document.removeEventListener('supercode:layer-open', another); document.removeEventListener('keydown', escape, true);
    };
  }, [open, id, focusFirst]);
  useLayoutEffect(() => {
    if (!open || !position) return;
    const popup = root.current?.querySelector<HTMLElement>('.composer-popover,.usage-popover');
    if (!popup) return;
    const update = () => {
      const trigger = root.current?.getBoundingClientRect();
      if (!trigger) return;
      const above = trigger.top - 22, below = window.innerHeight - trigger.bottom - 22;
      const down = above < Math.min(popup.scrollHeight, 280) && below > above;
      popup.style.top = down ? 'calc(100% + 10px)' : 'auto';
      popup.style.bottom = down ? 'auto' : 'calc(100% + 10px)';
      popup.style.setProperty('--layer-height', `${Math.max(80, down ? below : above)}px`);
      popup.style.setProperty('--layer-shift', '0px');
      const bounds = popup.getBoundingClientRect();
      popup.style.setProperty('--layer-shift', `${Math.max(12 - bounds.left, Math.min(0, window.innerWidth - 12 - bounds.right))}px`);
    };
    update(); const observer = new ResizeObserver(update); observer.observe(popup);
    window.addEventListener('resize', update);
    return () => { observer.disconnect(); window.removeEventListener('resize', update); };
  }, [open, position]);
  function navigate(event: KeyboardEvent) {
    if (!['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key) || (event.target as HTMLElement).matches('input,select,textarea')) return;
    const popup = root.current?.querySelector('.composer-popover,.usage-popover');
    const items = Array.from(popup?.querySelectorAll<HTMLElement>('button[role^="menuitem"]:not(:disabled)') ?? []);
    if (!items.length) return;
    event.preventDefault();
    const current = items.indexOf(document.activeElement as HTMLElement);
    const next = event.key === 'Home' ? 0 : event.key === 'End' ? items.length - 1 : event.key === 'ArrowDown' ? (current + 1) % items.length : (current <= 0 ? items.length : current) - 1;
    items[next].focus();
  }
  return { root, id, dismiss, navigate, restoreFocus: () => root.current?.querySelector<HTMLButtonElement>(':scope > button')?.focus() };
}
