// @vitest-environment happy-dom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import AppUpdateSettings, { AppUpdateNotice, type AppUpdateStatus } from './AppUpdate';
const mocks = vi.hoisted(() => ({ call: vi.fn(), listen: vi.fn() }));
vi.mock('./api', () => ({ desktop: true, call: mocks.call }));
vi.mock('@tauri-apps/api/event', () => ({ listen: mocks.listen }));
let root: Root, node: HTMLDivElement;
let status: AppUpdateStatus;
beforeEach(() => {
  (globalThis as any).IS_REACT_ACT_ENVIRONMENT = true;
  node = document.createElement('div'); document.body.append(node); root = createRoot(node);
  status = { currentVersion: '0.1.0', latestVersion: '0.2.0', notes: '修复与优化', phase: 'available', progress: 0, automatic: true, checkedAt: 1, error: null, releaseUrl: 'https://github.com/heyule69/SuperCode/releases' };
  mocks.call.mockReset(); mocks.listen.mockReset(); mocks.listen.mockResolvedValue(() => {});
  mocks.call.mockImplementation(async () => status);
});
afterEach(async () => { await act(async () => root.unmount()); node.remove(); });
const button = (text: string) => [...node.querySelectorAll<HTMLButtonElement>('button')].find(b => b.textContent === text)!;
async function render(busy = false) { await act(async () => root.render(<AppUpdateSettings busy={busy}/>)); }
it('checks explicitly and downloads without silently restarting', async () => {
  await render(); expect(node.textContent).toContain('发现新版本 0.2.0');
  await act(async () => button('检查更新').click()); expect(mocks.call).toHaveBeenCalledWith('check_app_update', { force: true });
  await act(async () => button('下载更新').click()); expect(mocks.call).toHaveBeenCalledWith('download_app_update', {});
  expect(mocks.call).not.toHaveBeenCalledWith('install_app_update', {});
});
it('requires tasks to finish before restarting but permits downloading', async () => {
  await render(true); expect(button('下载更新').disabled).toBe(false);
  status = { ...status, phase: 'ready', progress: 100 }; await act(async () => root.unmount()); root = createRoot(node); await render(true);
  expect(button('重启并更新').disabled).toBe(true); expect(node.textContent).toContain('请先完成正在运行的任务');
  await render(false); await act(async () => button('重启并更新').click()); expect(mocks.call).toHaveBeenCalledWith('install_app_update', {});
});
it('renders download progress and retryable errors accurately', async () => {
  status = { ...status, phase: 'downloading', progress: 47 }; await render();
  expect(node.querySelector<HTMLProgressElement>('progress')?.value).toBe(47); expect(button('检查更新').disabled).toBe(true);
  status = { ...status, phase: 'error', error: '无法连接 GitHub' }; await act(async () => root.unmount()); root = createRoot(node); await render();
  expect(node.textContent).not.toContain('已是最新版本'); expect(node.querySelector('[role=alert]')?.textContent).toBe('无法连接 GitHub'); expect(button('检查更新').disabled).toBe(false);
});
it('persists automatic checking preferences through the native backend', async () => {
  await render(); await act(async () => node.querySelector<HTMLInputElement>('input')!.click());
  expect(mocks.call).toHaveBeenCalledWith('set_automatic_app_updates', { enabled: false });
});
it('shows a dismissible notice that opens update settings', async () => {
  const open = vi.fn(); await act(async () => root.render(<AppUpdateNotice open={open}/>));
  await act(async () => button('查看更新').click()); expect(open).toHaveBeenCalledOnce();
  await act(async () => node.querySelector<HTMLButtonElement>('[aria-label="暂不更新"]')!.click()); expect(node.textContent).toBe('');
});
it('preserves ready state and shows backend task rejection without losing the package', async () => {
  status = { ...status, phase: 'ready' }; await render();
  mocks.call.mockImplementation(async (command: string) => { if (command === 'install_app_update') throw new Error('还有任务正在运行'); return status; });
  await act(async () => button('重启并更新').click()); expect(node.textContent).toContain('还有任务正在运行'); expect(button('重启并更新').disabled).toBe(false);
});
