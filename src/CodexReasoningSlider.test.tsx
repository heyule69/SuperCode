// @vitest-environment happy-dom
import { act, useState } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { CodexReasoningSlider } from './CodexReasoningSlider';

const levels = ['low', 'medium', 'high', 'xhigh', 'max', 'ultra'];
let node: HTMLDivElement, root: Root;
const choose = vi.fn(), showModels = vi.fn();
beforeEach(() => {
  (globalThis as any).IS_REACT_ACT_ENVIRONMENT = true;
  choose.mockClear(); showModels.mockClear();
  node = document.createElement('div'); document.body.append(node); root = createRoot(node);
});
afterEach(async () => { await act(async () => root.unmount()); node.remove(); });
function Picker({ initial = '', initialDefault = 'low', offered = levels }: { initial?: string; initialDefault?: string; offered?: string[] }) {
  const [value, setValue] = useState(initial);
  return <CodexReasoningSlider modelName="GPT-6.2-Sol" efforts={offered} value={value} defaultEffort={initialDefault} choose={value => { choose(value); setValue(value); }} showModels={showModels}/>;
}
async function render(props: Parameters<typeof Picker>[0] = {}) { await act(async () => root.render(<Picker {...props}/>)); }
async function input(value: string) {
  await act(async () => {
    const slider = node.querySelector<HTMLInputElement>('input')!;
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(slider, value);
    slider.dispatchEvent(new Event('input', { bubbles: true }));
  });
}

it('shows five default desktop stops without presenting the native Max capability as a normal option', async () => {
  await render();
  expect(node.querySelector('input')?.max).toBe('4');
  expect(node.querySelector('strong')?.textContent).toBe('低');
  expect(node.querySelector<HTMLButtonElement>('[aria-label="恢复默认推理强度"]')?.disabled).toBe(true);
  expect(node.textContent).not.toMatch(/Max|最大/);
  expect(choose).not.toHaveBeenCalled();
});

it('sends the exact underlying effort for each stop, including Ultra after xhigh', async () => {
  await render();
  const values = ['low', 'medium', 'high', 'xhigh', 'ultra'];
  const labels = ['低', '中', '高', '极高', 'Ultra'];
  for (let index = 0; index < values.length; index++) {
    await input(String(index));
    expect(choose).toHaveBeenLastCalledWith(values[index]);
    expect(node.querySelector('input')?.getAttribute('aria-valuetext')).toBe(labels[index]);
    expect(!!node.querySelector('.is-ultra')).toBe(index === values.length - 1);
  }
});

it('resets the override to the actual model default without adding a sixth Default stop', async () => {
  await render({ initial: 'ultra', initialDefault: 'medium' });
  await act(async () => node.querySelector<HTMLButtonElement>('[aria-label="恢复默认推理强度"]')!.click());
  expect(choose).toHaveBeenLastCalledWith('');
  expect(node.querySelector('strong')?.textContent).toBe('中');
  expect(node.querySelector('input')?.value).toBe('1');
  expect(node.querySelector('input')?.max).toBe('4');
});

it('preserves an existing advanced Max selection until the user changes it', async () => {
  await render({ initial: 'max' });
  expect(node.querySelector('strong')?.textContent).toBe('最大');
  expect(choose).not.toHaveBeenCalled();
  await input('3');
  expect(choose).toHaveBeenLastCalledWith('xhigh');
  expect(node.querySelector('input')?.max).toBe('4');
  expect(node.textContent).not.toContain('最大');
});

it('does not invent an Ultra step for a model that does not support it', async () => {
  await render({ offered: ['low', 'medium', 'high', 'xhigh', 'max'] });
  expect(node.querySelector('input')?.max).toBe('3');
  await input('3');
  expect(choose).toHaveBeenLastCalledWith('xhigh');
  expect(node.querySelector('.is-ultra')).toBeNull();
});

it('keeps an unknown default unset and lets the user select a level with the keyboard', async () => {
  await render({ initialDefault: '' });
  expect(node.querySelector('strong')?.textContent).toBe('默认');
  expect(node.querySelector('.is-unset')).not.toBeNull();
  expect(choose).not.toHaveBeenCalled();
  await act(async () => node.querySelector('input')!.dispatchEvent(new KeyboardEvent('keydown', { key: 'End', bubbles: true })));
  expect(choose).toHaveBeenLastCalledWith('ultra');
  expect(node.querySelector('.is-unset')).toBeNull();
});

it('returns to the model list without changing the reasoning selection', async () => {
  await render({ initial: 'high' });
  await act(async () => node.querySelector<HTMLButtonElement>('.reasoning-slider-model')!.click());
  expect(showModels).toHaveBeenCalledOnce();
  expect(choose).not.toHaveBeenCalled();
});
