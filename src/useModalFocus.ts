import { useEffect, useRef, type RefObject } from 'react';

export function useModalFocus(open: boolean, panel: RefObject<HTMLElement | null>, close: () => void, fallback?: RefObject<HTMLElement | null>, locked = false) {
  const latest = useRef({ close, locked }); latest.current = { close, locked };
  useEffect(() => {
    if (!open) return;
    const previous = document.activeElement as HTMLElement | null;
    const el = panel.current;
    const focusable = () => [...(el?.querySelectorAll<HTMLElement>('button:not(:disabled),input:not(:disabled),select:not(:disabled),textarea:not(:disabled),a[href],[tabindex="0"]') ?? [])].filter(item => !item.closest('[hidden],[inert]') && item.getAttribute('aria-hidden') !== 'true');
    (el?.querySelector<HTMLElement>('input:not([type=checkbox]):not(:disabled)') ?? focusable()[0] ?? el)?.focus();
    const trap = (event: KeyboardEvent) => {
      if (event.key === 'Escape') { event.preventDefault(); event.stopImmediatePropagation(); if (!latest.current.locked) latest.current.close(); return; }
      if (event.key !== 'Tab') return;
      const items = focusable(), first = items[0], last = items.at(-1);
      if (!first) { event.preventDefault(); el?.focus(); return; }
      if (!el?.contains(document.activeElement) || event.shiftKey && document.activeElement === first) { event.preventDefault(); (event.shiftKey ? last : first)?.focus(); }
      else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first.focus(); }
    };
    document.addEventListener('keydown', trap, true);
    return () => { document.removeEventListener('keydown', trap, true); if (previous?.isConnected && !previous.closest('[inert],[hidden]')) previous.focus(); else fallback?.current?.focus(); };
  }, [open, panel, fallback]);
}
