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
async function render(runningConnections: Record<string, string[]> = {}) { await act(async () => root.render(<OfficialAccounts agent="codex" runningConnections={runningConnections} back={() => {}} updated={updated}/>)); }
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

it('locks only a running account while other accounts, refresh, defaults, and adding remain available', async () => {
  await render({ codex: ['account:a'], claude: ['account:b'] });
  expect(node.querySelector<HTMLButtonElement>('[aria-label="重命名 个人账号"]')!.disabled).toBe(true);
  expect(node.querySelector<HTMLButtonElement>('[aria-label="移除 个人账号"]')!.disabled).toBe(true);
  expect(node.querySelector<HTMLButtonElement>('[aria-label="重命名 工作账号"]')!.disabled).toBe(false);
  expect(node.querySelector<HTMLButtonElement>('[aria-label="移除 工作账号"]')!.disabled).toBe(false);
  expect(node.querySelector('.official-account-row:nth-child(2)')?.textContent).toContain('任务使用中');
  expect(buttons('刷新账号')[0].disabled).toBe(false);
  await act(async () => buttons('刷新账号')[0].click());
  expect(mocks.call).toHaveBeenCalledWith('list_official_accounts', { agent: 'codex', force: true });
  const row = node.querySelector('.official-account-row:nth-child(3)')!;
  await act(async () => row.querySelector<HTMLButtonElement>('.quiet-button')!.click());
  expect(mocks.call).toHaveBeenCalledWith('use_official_account', { agent: 'codex', accountId: 'b' });
  await act(async () => buttons('添加账号')[0].click());
  expect(node.querySelector<HTMLInputElement>('#official-account-name')!.disabled).toBe(false);
});

it('allows an isolated managed login to start while the native account is in use', async () => {
  const original = mocks.call.getMockImplementation()!;
  mocks.call.mockImplementation(async (command: string, args: unknown) => command === 'start_official_account_login' ? { accountId: 'new', message: '已打开登录页' } : original(command, args));
  await render({ codex: ['@official'] });
  await act(async () => buttons('添加账号')[0].click());
  await act(async () => {
    const input = node.querySelector<HTMLInputElement>('#official-account-name')!;
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(input, '新增账号');
    input.dispatchEvent(new Event('input', { bubbles: true }));
  });
  await act(async () => buttons('前往官方登录')[0].click());
  expect(mocks.call).toHaveBeenCalledWith('start_official_account_login', { agent: 'codex', name: '新增账号' });
  expect(node.textContent).toContain('等待浏览器登录');
});

it('completes a pending managed login while a different account runs', async () => {
  sessionStorage.setItem('supercode.officialAccountLogin', JSON.stringify({ agent: 'codex', accountId: 'pending', name: '新账号' }));
  const original = mocks.call.getMockImplementation()!;
  mocks.call.mockImplementation(async (command: string, args: unknown) => command === 'finish_official_account_login' ? { loggedIn: true, isDefault: false } : original(command, args));
  await render({ codex: ['account:a'] });
  expect(buttons('检查登录')[0].disabled).toBe(false);
  await act(async () => buttons('检查登录')[0].click());
  expect(mocks.call).toHaveBeenCalledWith('finish_official_account_login', { accountId: 'pending' });
  expect(node.textContent).toContain('账号已添加');
});

it('disables a rename already open when that account starts running', async () => {
  await render();
  await act(async () => node.querySelector<HTMLButtonElement>('[aria-label="重命名 工作账号"]')!.click());
  await render({ codex: ['account:b'] });
  expect(node.querySelector<HTMLInputElement>('[aria-label="修改账号名称"]')!.disabled).toBe(true);
  expect(buttons('保存')[0].disabled).toBe(true);
  await act(async () => node.querySelector<HTMLFormElement>('.official-account-details form')!.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true })));
  expect(mocks.call.mock.calls.some(([command]) => command === 'rename_official_account')).toBe(false);
  await render();
  expect(buttons('保存')[0].disabled).toBe(false);
});

it('disables an open removal confirmation when its account starts running', async () => {
  await render();
  await act(async () => node.querySelector<HTMLButtonElement>('[aria-label="移除 工作账号"]')!.click());
  await render({ codex: ['account:b'] });
  expect(buttons('移除')[0].disabled).toBe(true);
  await act(async () => buttons('移除')[0].click());
  expect(mocks.call.mock.calls.some(([command]) => command === 'remove_official_account')).toBe(false);
  await render();
  expect(buttons('移除')[0].disabled).toBe(false);
});
