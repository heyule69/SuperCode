// @vitest-environment happy-dom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { browserDockLayout, browserDockSize, useBrowserDock } from './BrowserDock';

let root: Root, container: HTMLDivElement;
let width = 1200;
function Layout({ expanded = false }: { expanded?: boolean }) {
  const dock = useBrowserDock(true, expanded);
  return <div ref={dock.root}><div {...dock.handle}/></div>;
}
beforeEach(() => {
  (globalThis as any).IS_REACT_ACT_ENVIRONMENT = true;
  localStorage.clear(); width = 1200;
  vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockImplementation(() => width);
  vi.spyOn(HTMLElement.prototype, 'clientHeight', 'get').mockReturnValue(800);
  HTMLElement.prototype.setPointerCapture = vi.fn();
  container = document.createElement('div'); document.body.append(container); root = createRoot(container);
});
afterEach(async () => { await act(async () => root.unmount()); container.remove(); vi.restoreAllMocks(); });
const handle = () => container.querySelector<HTMLDivElement>('[role="separator"]')!;
async function pointer(type: string, x: number, y = 0) {
  await act(async () => handle().dispatchEvent(new PointerEvent(type, { clientX: x, clientY: y, button: 0, pointerId: 1, bubbles: true })));
}
it('resizes the browser without changing sidebar or review preferences and cancels on Escape', async () => {
  const panels = '{"left":288,"right":284}'; localStorage.setItem('supercode.panel-widths', panels);
  await act(async () => root.render(<Layout/>));
  await pointer('pointerdown', 680); await pointer('pointermove', 620); await pointer('pointerup', 620);
  expect(JSON.parse(localStorage.getItem('supercode.browser-size')!)).toEqual({ width: 580, height: 360 });
  expect(localStorage.getItem('supercode.panel-widths')).toBe(panels);
  await pointer('pointerdown', 620); await pointer('pointermove', 480);
  await act(async () => handle().dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })));
  expect(handle().getAttribute('aria-valuenow')).toBe('580');
});
it('stacks in a small conversation area, resizes height and preserves the chat space', async () => {
  width = 640; await act(async () => root.render(<Layout/>));
  expect(handle().getAttribute('aria-orientation')).toBe('horizontal');
  await pointer('pointerdown', 0, 440); await pointer('pointermove', 0, 350); await pointer('pointerup', 0, 350);
  expect(JSON.parse(localStorage.getItem('supercode.browser-size')!)).toEqual({ width: 520, height: 450 });
  expect(browserDockLayout({ width: 900, height: 800 }, 640, 700)).toEqual({ stacked: true, extent: 420 });
  expect(browserDockLayout({ width: 900, height: 800 }, 1000, 700)).toEqual({ stacked: false, extent: 620 });
  expect(browserDockSize('{"width":null,"height":-8}')).toEqual({ width: 520, height: 220 });
});
