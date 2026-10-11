// @vitest-environment happy-dom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import NotificationSettings from './NotificationSettings';
import { defaults } from './preferences';

const mocks = vi.hoisted(() => ({ call: vi.fn() }));
vi.mock('./api', () => ({ desktop: true, call: mocks.call }));
let root: Root, node: HTMLDivElement;
const setPrefs = vi.fn();
const button = () => node.querySelector<HTMLButtonElement>('[aria-label="打开系统通知设置"]')!;
beforeEach(() => {
  (globalThis as any).IS_REACT_ACT_ENVIRONMENT = true;
  node = document.createElement('div'); document.body.append(node); root = createRoot(node);
  setPrefs.mockClear(); mocks.call.mockReset();
  mocks.call.mockImplementation(async (command: string) => command === 'notification_status' ? { state: 'systemBlocked', message: '系统通知已关闭' } : undefined);
});
afterEach(async () => { await act(async () => root.unmount()); node.remove(); });
async function render() { await act(async () => root.render(<NotificationSettings prefs={{ ...defaults, notificationsEnabled: false }} setPrefs={setPrefs} />)); }

it('opens OS settings independently of the app preference, prevents duplicate launches, and rechecks on return', async () => {
  await render();
  expect(button().disabled).toBe(false);
  expect(node.querySelector('[role="status"]')?.textContent).toBe('系统通知已关闭');
  let finish!: () => void;
  const pending = new Promise<void>(resolve => { finish = resolve; });
  mocks.call.mockImplementation(async (command: string) => command === 'open_system_notification_settings' ? pending : { state: 'systemBlocked', message: '系统通知已关闭' });
  await act(async () => { button().click(); button().click(); });
  expect(mocks.call.mock.calls.filter(([command]) => command === 'open_system_notification_settings')).toHaveLength(1);
  expect(button().disabled).toBe(true);
  expect(button().textContent).toBe('正在打开…');
  await act(async () => { finish(); await pending; });
  expect(button().disabled).toBe(false);
  expect(node.querySelector('[role="status"]')?.textContent).toBe('系统通知已关闭');
  mocks.call.mockResolvedValue({ state: 'enabled', message: '系统允许发送通知' });
  await act(async () => window.dispatchEvent(new Event('focus')));
  expect(mocks.call).toHaveBeenLastCalledWith('notification_status');
  expect(node.querySelector('[role="status"]')?.textContent).toBe('系统允许发送通知');
  expect(node.querySelector('.notification-status-badge')?.textContent).toBe('可用');
  expect(setPrefs).not.toHaveBeenCalled();
});

it('keeps the launch action available when permission checks fail', async () => {
  mocks.call.mockRejectedValueOnce(new Error('WinRT status unavailable'));
  await render();
  expect(node.querySelector('[role="status"]')?.textContent).toBe('暂时无法检查系统通知状态');
  expect(button().disabled).toBe(false);
  await act(async () => button().click());
  expect(mocks.call).toHaveBeenLastCalledWith('open_system_notification_settings');
});

it('shows a manual path on launch failure and clears the error on retry without claiming permission', async () => {
  await render();
  mocks.call.mockRejectedValueOnce(new Error('No handler for ms-settings'));
  await act(async () => button().click());
  expect(node.querySelector('[role="alert"]')?.textContent).toContain('Win + I');
  expect(button().disabled).toBe(false);
  expect(node.querySelector('.notification-status-badge')?.textContent).toBe('已关闭');
  await act(async () => button().click());
  expect(node.querySelector('[role="alert"]')).toBeNull();
  expect(node.querySelector('.notification-status-badge')?.textContent).toBe('已关闭');
  expect(setPrefs).not.toHaveBeenCalled();
});
