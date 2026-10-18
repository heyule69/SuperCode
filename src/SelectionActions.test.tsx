// @vitest-environment happy-dom
import { act, useRef } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { SelectionActions } from './SelectionActions';

let root: Root, container: HTMLDivElement;
const add = vi.fn(), details = vi.fn(), ask = vi.fn();
function Fixture() {
  const area = useRef<HTMLDivElement>(null);
  return <><div ref={area} className="test-area"><article className="message user"><p>用户输入的文字</p></article><article className="message assistant"><p>回复的文字</p></article></div><SelectionActions area={area} add={add} details={details} ask={ask}/></>;
}
beforeEach(async () => {
  (globalThis as any).IS_REACT_ACT_ENVIRONMENT = true;
  add.mockClear(); details.mockClear(); ask.mockClear();
  container = document.createElement('div'); document.body.append(container); root = createRoot(container);
  const original = HTMLElement.prototype.getBoundingClientRect;
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) { return this.classList.contains('selection-actions') ? new DOMRect(0,0,160,34) : original.call(this); });
  await act(async () => root.render(<Fixture/>));
});
afterEach(async () => { await act(async () => root.unmount()); container.remove(); window.getSelection()?.removeAllRanges(); vi.restoreAllMocks(); });
async function select(role: 'user' | 'assistant', rect = new DOMRect(100,100,150,20)) {
  const range = document.createRange(); range.selectNodeContents(container.querySelector(`.message.${role} p`)!);
  Object.defineProperty(range, 'getBoundingClientRect', {value:() => rect});
  window.getSelection()?.removeAllRanges(); window.getSelection()?.addRange(range);
  await act(async () => container.querySelector('.test-area')!.dispatchEvent(new MouseEvent('mouseup', {bubbles:true})));
}
it('offers only side-chat questions for a user message and passes the exact selection', async () => {
  await select('user');
  expect([...container.querySelectorAll('.selection-actions button')].map(button => button.textContent)).toEqual(['在侧边聊天中提问']);
  await act(async () => container.querySelector<HTMLButtonElement>('.selection-actions button')!.click());
  expect(ask).toHaveBeenCalledWith('用户输入的文字');
  expect(add).not.toHaveBeenCalled(); expect(details).not.toHaveBeenCalled();
  expect(container.querySelector('.selection-actions')).toBeNull();
});
it('retains the existing actions for assistant content', async () => {
  await select('assistant');
  expect(container.querySelectorAll('.selection-actions button')).toHaveLength(3);
});
it('keeps the measured toolbar inside the viewport without focusing or scrolling', async () => {
  const focus = vi.spyOn(HTMLElement.prototype, 'focus');
  await select('user', new DOMRect(window.innerWidth-20,4,100,20));
  const toolbar = container.querySelector<HTMLElement>('.selection-actions')!;
  expect(parseFloat(toolbar.style.left) + 160).toBeLessThanOrEqual(window.innerWidth-12);
  expect(parseFloat(toolbar.style.top)).toBe(32);
  expect(toolbar.style.visibility).toBe('visible');
  expect(container.querySelector('.test-area')!.scrollLeft).toBe(0);
  expect(focus).not.toHaveBeenCalled();
});
it('stays dismissed after Escape keyup and closes when the selection is cleared', async () => {
  await select('user');
  await act(async () => { window.dispatchEvent(new KeyboardEvent('keydown', {key:'Escape'})); container.querySelector('.test-area')!.dispatchEvent(new KeyboardEvent('keyup', {key:'Escape',bubbles:true})); });
  expect(container.querySelector('.selection-actions')).toBeNull();
  await select('user');
  await act(async () => { window.getSelection()?.removeAllRanges(); document.dispatchEvent(new Event('selectionchange')); });
  expect(container.querySelector('.selection-actions')).toBeNull();
});
