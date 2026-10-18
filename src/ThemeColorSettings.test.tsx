// @vitest-environment happy-dom
import { act, useLayoutEffect, useState } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it } from 'vitest';
import ClientSettings from './ClientSettings';
import { applyPreferences, loadPreferences } from './preferences';

let root: Root, container: HTMLDivElement;
function Appearance() {
  const [prefs, setPrefs] = useState(loadPreferences);
  useLayoutEffect(() => applyPreferences(prefs), [prefs]);
  return <ClientSettings tab="appearance" prefs={prefs} setPrefs={setPrefs} projectId="" busy={false} addSkill={() => {}}/>;
}
beforeEach(() => {
  (globalThis as any).IS_REACT_ACT_ENVIRONMENT = true;
  localStorage.clear();
  container = document.createElement('div'); document.body.append(container); root = createRoot(container);
});
afterEach(async () => { await act(async () => root.unmount()); container.remove(); localStorage.clear(); document.documentElement.style.cssText = ''; });
async function render() { await act(async () => root.render(<Appearance/>)); }
async function enter(label: string, value: string, commit = true) {
  const input = container.querySelector<HTMLInputElement>(`[aria-label="${label}"]`)!;
  await act(async () => { Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(input, value); input.dispatchEvent(new Event('input', {bubbles:true})); });
  if (commit) await act(async () => input.dispatchEvent(new FocusEvent('focusout', {bubbles:true})));
  return input;
}
async function theme(name: string) { await act(async () => [...container.querySelectorAll<HTMLButtonElement>('.theme-option')].find(button => button.textContent === name)!.click()); }

it('saves per-theme colors, restores them on reopen and resets only the chosen field', async () => {
  await render();
  await enter('背景色值', '#09121b'); await enter('强调色值', '#a78bfa');
  expect(document.documentElement.style.getPropertyValue('--bg')).toBe('#09121B');
  expect(loadPreferences().themeColors?.light).toEqual({background:'#09121B',accent:'#A78BFA'});
  await act(async () => root.unmount()); root = createRoot(container); await render();
  expect(container.querySelector<HTMLInputElement>('[aria-label="背景色值"]')!.value).toBe('#09121B');
  await theme('暗黑');
  expect(document.documentElement.style.getPropertyValue('--bg')).toBe('');
  await enter('背景色值', '#101820');
  await theme('白色');
  expect(document.documentElement.style.getPropertyValue('--bg')).toBe('#09121B');
  await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="重置背景色"]')!.click());
  expect(document.documentElement.style.getPropertyValue('--bg')).toBe('');
  expect(loadPreferences().themeColors).toEqual({light:{accent:'#A78BFA'},dark:{background:'#101820'}});
});

it('allows a complete hex draft and never persists invalid input', async () => {
  await render();
  const input = await enter('强调色值', '#abc', false);
  expect(input.value).toBe('#abc');
  expect(loadPreferences().themeColors).toBeUndefined();
  await enter('强调色值', '#abcdef');
  expect(input.value).toBe('#ABCDEF');
  await enter('强调色值', 'red');
  expect(input.getAttribute('aria-invalid')).toBe('true');
  expect(loadPreferences().themeColors?.light?.accent).toBe('#ABCDEF');
  await act(async () => input.dispatchEvent(new KeyboardEvent('keydown', {key:'Escape',bubbles:true})));
  expect(input.value).toBe('#ABCDEF');
  expect(input.getAttribute('aria-invalid')).toBe('false');
});

it('applies color-picker choices immediately', async () => {
  await render();
  const input = container.querySelector<HTMLInputElement>('[aria-label="选择前景色"]')!;
  await act(async () => { Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(input, '#123456'); input.dispatchEvent(new Event('input', {bubbles:true})); });
  expect(document.documentElement.style.getPropertyValue('--text')).toBe('#123456');
  expect(loadPreferences().themeColors?.light?.foreground).toBe('#123456');
});
