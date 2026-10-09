// @vitest-environment happy-dom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import OfficialAccounts from './OfficialAccounts';
const mocks = vi.hoisted(() => ({ call: vi.fn() }));
vi.mock('./api', () => ({ desktop: true, call: mocks.call }));
let node: HTMLDivElement, root: Root;
const updated = vi.fn(async () => {});
const accounts = [
  { id: '@official', accountId: null, agent: 'codex', name: '本机账号', loggedIn: true, current: false },
  { id: 'account:a', accountId: 'a', agent: 'codex', name: '个人账号', loggedIn: true, current: true },
  { id: 'account:b', accountId: 'b', agent: 'codex', name: '工作账号', loggedIn: true, current: false },
  { id: 'account:c', accountId: 'c', agent: 'codex', name: '过期账号', loggedIn: false, current: false },
];
beforeEach(() => {
  (globalThis as any).IS_REACT_ACT_ENVIRONMENT = true;
  node = document.createElement('div'); document.body.append(node); root = createRoot(node);
  sessionStorage.clear(); updated.mockClear(); mocks.call.mockReset();
  mocks.call.mockImplementation(async (command: string) => command === 'list_official_accounts' ? accounts : undefined);
});
afterEach(async () => { await act(async () => root.unmount()); node.remove(); });
async function render() { await act(async () => root.render(<OfficialAccounts agent="codex" busy={false} back={() => {}} updated={updated}/>)); }
const buttons = (label: string) => [...node.querySelectorAll<HTMLButtonElement>('button')].filter(button => button.textContent === label);
it('lists native and multiple managed accounts without any key or model form', async () => {
  await render(); expect(node.querySelectorAll('.official-account-row')).toHaveLength(4);
  expect(node.textContent).toContain('个人账号'); expect(node.textContent).toContain('工作账号');
  expect(node.querySelector('input')).toBeNull(); expect(node.textContent).not.toContain('API Key');
  expect(node.querySelector<HTMLButtonElement>('.official-account-row:last-child .quiet-button')!.disabled).toBe(true);
});
it('sets exactly the selected account as default', async () => {
  await render(); const row = [...node.querySelectorAll('.official-account-row')].find(row => row.textContent?.includes('工作账号'))!;
  await act(async () => row.querySelector<HTMLButtonElement>('.quiet-button')!.click());
  expect(mocks.call).toHaveBeenCalledWith('use_official_account', { agent: 'codex', accountId: 'b' }); expect(updated).toHaveBeenCalledWith('codex');
});
it('a native account does not complete a different pending managed login', async () => {
  sessionStorage.setItem('supercode.officialAccountLogin', JSON.stringify({ agent: 'codex', accountId: 'pending', name: '新账号' }));
  const original = mocks.call.getMockImplementation()!;
  mocks.call.mockImplementation(async (command: string, args: unknown) => command === 'finish_official_account_login' ? { loggedIn: false } : original(command, args));
  await render(); await act(async () => buttons('检查登录')[0].click());
  expect(mocks.call).toHaveBeenCalledWith('finish_official_account_login', { accountId: 'pending' });
  expect(node.textContent).toContain('等待浏览器登录'); expect(updated).not.toHaveBeenCalled();
  expect(mocks.call.mock.calls.some(([command]) => command === 'use_official_account')).toBe(false);
});
it.each([true, false])('completes a managed login and only resets the draft when it becomes default (%s)', async isDefault => {
  sessionStorage.setItem('supercode.officialAccountLogin', JSON.stringify({ agent: 'codex', accountId: 'pending', name: '新账号' }));
  const original = mocks.call.getMockImplementation()!;
  mocks.call.mockImplementation(async (command: string, args: unknown) => command === 'finish_official_account_login' ? { loggedIn: true, isDefault } : original(command, args));
  await render(); await act(async () => buttons('检查登录')[0].click());
  expect(sessionStorage.getItem('supercode.officialAccountLogin')).toBeNull();
  expect(updated).toHaveBeenCalledWith(isDefault ? 'codex' : undefined);
  expect(node.textContent).toContain('账号已添加');
});
it('cancels only the pending login and never removes an existing account', async () => {
  sessionStorage.setItem('supercode.officialAccountLogin', JSON.stringify({ agent: 'codex', accountId: 'pending', name: '新账号' }));
  await render(); await act(async () => buttons('取消')[0].click());
  expect(mocks.call).toHaveBeenCalledWith('cancel_official_account_login', { accountId: 'pending' });
  expect(sessionStorage.getItem('supercode.officialAccountLogin')).toBeNull(); expect(node.querySelectorAll('.official-account-row')).toHaveLength(4);
});
it('keeps a bound account when the backend refuses removal', async () => {
  const original = mocks.call.getMockImplementation()!;
  mocks.call.mockImplementation(async (command: string, args: unknown) => { if (command === 'remove_official_account') throw new Error('还有聊天使用此连接'); return original(command, args); });
  await render(); await act(async () => node.querySelector<HTMLButtonElement>('[aria-label="移除 工作账号"]')!.click());
  await act(async () => buttons('移除')[0].click());
  expect(node.querySelector('[role="alert"]')?.textContent).toContain('还有聊天使用此连接'); expect(node.textContent).toContain('工作账号'); expect(updated).not.toHaveBeenCalled();
});
