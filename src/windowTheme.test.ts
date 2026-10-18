// @vitest-environment happy-dom
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { call } from './api';
const setTheme = vi.hoisted(() => vi.fn(async () => {}));
let watchWindowTheme: typeof import('./windowTheme').watchWindowTheme;
vi.mock('./api', () => ({call:vi.fn(async () => true)}));
vi.mock('@tauri-apps/api/window', () => ({getCurrentWindow:() => ({setTheme})}));
beforeEach(async () => { vi.resetModules(); ({watchWindowTheme} = await import('./windowTheme')); });
afterEach(() => { vi.clearAllMocks(); document.documentElement.style.cssText = ''; });
const settle = () => new Promise(resolve => setTimeout(resolve, 10));

it('sends custom canvas and title-bar colors as hex values through the existing native command', async () => {
  const error = vi.fn();
  const dispose = watchWindowTheme('light', error, {light:{background:'#f8f4ec',foreground:'#352d25',accent:'#99734a'}});
  await settle();
  expect(setTheme).toHaveBeenCalledWith('light');
  expect(call).toHaveBeenCalledWith('set_window_colors', {colors:expect.objectContaining({background:'#F8F4EC',canvas:'#F8F4EC',text:'#352D25',border:expect.stringMatching(/^#[0-9A-F]{6}$/)})});
  expect(error).not.toHaveBeenCalled();
  dispose();
});

it('skips superseded native theme work instead of applying an older palette after a newer choice', async () => {
  const error = vi.fn();
  const first = watchWindowTheme('light', error, {light:{background:'#112233'}});
  first();
  const second = watchWindowTheme('dark', error, {dark:{background:'#223344'}});
  await settle();
  expect(setTheme).toHaveBeenCalledTimes(1);
  expect(setTheme).toHaveBeenCalledWith('dark');
  expect(call).toHaveBeenCalledTimes(1);
  expect(call).toHaveBeenCalledWith('set_window_colors', {colors:expect.objectContaining({canvas:'#223344'})});
  second();
});

it('updates custom colors without resetting the same native theme again', async () => {
  const error = vi.fn();
  const first = watchWindowTheme('light', error, {light:{background:'#FAF0E6'}});
  await settle(); first();
  const second = watchWindowTheme('light', error, {light:{background:'#FCF6EF'}});
  await settle();
  expect(setTheme).toHaveBeenCalledTimes(1);
  expect(call).toHaveBeenCalledTimes(2);
  expect(call).toHaveBeenLastCalledWith('set_window_colors', {colors:expect.objectContaining({canvas:'#FCF6EF'})});
  second();
});
