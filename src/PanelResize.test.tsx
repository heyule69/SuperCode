// @vitest-environment happy-dom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { decodePanelWidths, defaultWidths, panelLayout, usePanelResize } from './PanelResize';

let root: Root, container: HTMLDivElement;
let narrow = false;
function Layout() {
  const panels = usePanelResize(true, true, false);
  return <div ref={panels.root}><div {...panels.handle('left')}/><div {...panels.handle('right')}/></div>;
}
beforeEach(() => {
  (globalThis as any).IS_REACT_ACT_ENVIRONMENT = true;
  localStorage.clear(); narrow = false;
  vi.spyOn(window, 'matchMedia').mockImplementation(() => ({ matches:narrow } as MediaQueryList));
  vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(1400);
  HTMLElement.prototype.setPointerCapture = vi.fn();
  container = document.createElement('div'); document.body.append(container); root = createRoot(container);
});
afterEach(async () => { await act(async () => root.unmount()); container.remove(); vi.restoreAllMocks(); });
const handle = (side: string) => container.querySelector<HTMLDivElement>(`.${side}`)!;
async function pointer(side: string, type: string, x: number) {
  await act(async () => handle(side).dispatchEvent(new PointerEvent(type, { clientX:x, button:0, pointerId:1, bubbles:true })));
}
it('drags both boundaries in the correct direction and restores persisted widths', async () => {
  await act(async () => root.render(<Layout/>));
  await pointer('left', 'pointerdown', 288); await pointer('left', 'pointermove', 350); await pointer('left', 'pointerup', 350);
  await pointer('right', 'pointerdown', 1116); await pointer('right', 'pointermove', 1000); await pointer('right', 'pointerup', 1000);
  expect(JSON.parse(localStorage.getItem('supercode.panel-widths')!)).toEqual({ left:350, right:400 });
  expect(container.firstElementChild?.getAttribute('style')).toContain('350px minmax(0,1fr) 400px');
  await act(async () => root.unmount()); root = createRoot(container);
  await act(async () => root.render(<Layout/>));
  expect(handle('left').getAttribute('aria-valuenow')).toBe('350');
  expect(handle('right').getAttribute('aria-valuenow')).toBe('400');
});
it('cancels a drag, supports keyboard resizing and resets on double click', async () => {
  await act(async () => root.render(<Layout/>));
  await pointer('left', 'pointerdown', 288); await pointer('left', 'pointermove', 500); await pointer('left', 'pointercancel', 500);
  expect(handle('left').getAttribute('aria-valuenow')).toBe('288');
  await act(async () => handle('right').dispatchEvent(new KeyboardEvent('keydown', { key:'ArrowLeft', bubbles:true })));
  expect(handle('right').getAttribute('aria-valuenow')).toBe('294');
  await act(async () => handle('right').dispatchEvent(new MouseEvent('dblclick', { bubbles:true })));
  expect(handle('right').getAttribute('aria-valuenow')).toBe('284');
});
it('bounds saved values and preserves the chat at smaller window sizes', () => {
  expect(decodePanelWidths('bad')).toEqual(defaultWidths);
  expect(decodePanelWidths('{"left":5000,"right":-1}')).toEqual({ left:580, right:240 });
  const layout = panelLayout({ left:580, right:620 }, 1200, true, true);
  expect(1200 - layout.left - layout.right).toBeGreaterThanOrEqual(360);
  narrow = true;
  expect(panelLayout(defaultWidths, 900, true, true).overlay).toBe(true);
});
