// @vitest-environment happy-dom
import { act, useRef } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { useModalFocus } from './useModalFocus';

let root: Root, node: HTMLDivElement;
const close = vi.fn();
function Dialog({ open }: { open: boolean }) {
  const panel = useRef<HTMLElement>(null), fallback = useRef<HTMLButtonElement>(null);
  useModalFocus(open, panel, close, fallback);
  return <><button ref={fallback} id="background" inert={open}>background</button>{open ? <section ref={panel} role="dialog" tabIndex={-1}><button id="first">close</button><button id="last">copy</button></section> : null}</>;
}
beforeEach(() => { (globalThis as any).IS_REACT_ACT_ENVIRONMENT = true; close.mockClear(); node = document.createElement('div'); document.body.append(node); root = createRoot(node); });
afterEach(async () => { await act(async () => root.unmount()); node.remove(); });
it('focuses the dialog, traps both directions, handles Escape and restores the opener', async () => {
  await act(async () => root.render(<Dialog open={false}/>)); node.querySelector<HTMLButtonElement>('#background')!.focus();
  await act(async () => root.render(<Dialog open/>));
  expect(document.activeElement?.id).toBe('first');
  document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Tab', shiftKey: true, bubbles: true, cancelable: true }));
  expect(document.activeElement?.id).toBe('last');
  document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Tab', bubbles: true, cancelable: true }));
  expect(document.activeElement?.id).toBe('first');
  document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }));
  expect(close).toHaveBeenCalledOnce();
  await act(async () => root.render(<Dialog open={false}/>));
  expect(document.activeElement?.id).toBe('background');
});
