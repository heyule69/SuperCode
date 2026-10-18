// @vitest-environment happy-dom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { ComposerTextarea } from './ComposerTextarea';

let root: Root, container: HTMLDivElement;
let observer: ResizeObserverCallback;
let width = 700;
beforeEach(() => {
  (globalThis as any).IS_REACT_ACT_ENVIRONMENT = true;
  vi.stubGlobal('ResizeObserver', class {
    constructor(callback: ResizeObserverCallback) { observer = callback; }
    observe() {}
    disconnect() {}
  });
  width = 700;
  vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockImplementation(() => width);
  vi.spyOn(HTMLElement.prototype, 'scrollHeight', 'get').mockImplementation(function (this: HTMLElement) {
    return width === 0 ? 0 : (this as HTMLTextAreaElement).value ? 180 : 32;
  });
  container = document.createElement('div'); document.body.append(container); root = createRoot(container);
});
afterEach(async () => { await act(async () => root.unmount()); container.remove(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });
async function render(value = '', minHeight = 48) {
  await act(async () => root.render(<ComposerTextarea value={value} minHeight={minHeight} onChange={() => {}}/>));
  return container.querySelector('textarea')!;
}

it('grows with a draft and shrinks to the same baseline when it is cleared', async () => {
  expect((await render()).style.height).toBe('48px');
  expect((await render('long draft')).style.height).toBe('180px');
  expect((await render()).style.height).toBe('48px');
});

it('preserves the minimum height across the hidden and visible widths of the settings page', async () => {
  const textarea = await render();
  width = 0; await act(async () => observer([], {} as ResizeObserver));
  width = 700; await act(async () => observer([], {} as ResizeObserver));
  expect(textarea.style.height).toBe('48px');
  expect((await render('', 28)).style.height).toBe('32px');
});

it('does not collapse a long draft while its chat is hidden behind settings', async () => {
  const textarea = await render('long draft');
  container.hidden = true; width = 0;
  await act(async () => observer([], {} as ResizeObserver));
  expect(textarea.style.height).toBe('180px');
  container.hidden = false; width = 700;
  await act(async () => observer([], {} as ResizeObserver));
  expect(textarea.style.height).toBe('180px');
});
