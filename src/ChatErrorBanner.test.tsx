// @vitest-environment happy-dom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { call } from './api';
import { ChatErrorBanner } from './ChatErrorBanner';

vi.mock('./api', () => ({ desktop:true, call:vi.fn() }));
const offline = 'Magpie 网关未连接。请先打开 Magpie（后台运行也可以）后再发送；已保存的模型目录不代表当前可调用。';
let root: Root, node: HTMLDivElement;
const dismiss = vi.fn();
beforeEach(() => {
  (globalThis as any).IS_REACT_ACT_ENVIRONMENT = true;
  vi.mocked(call).mockReset(); dismiss.mockClear();
  node = document.createElement('div'); document.body.append(node); root = createRoot(node);
});
afterEach(async () => { await act(async () => root.unmount()); node.remove(); });
async function render(error = offline) { await act(async () => root.render(<ChatErrorBanner error={error} dismiss={dismiss}/>)); }
function openButton() { return [...node.querySelectorAll<HTMLButtonElement>('button')].find(button => button.textContent === '打开 Magpie')!; }

it('opens only on an explicit click, confirms the request without claiming readiness or sending a task', async () => {
  await render();
  expect(call).not.toHaveBeenCalled();
  expect(node.textContent).toContain('草稿已保留');
  await act(async () => openButton().click());
  expect(vi.mocked(call).mock.calls).toEqual([['open_magpie']]);
  expect(node.textContent).toContain('已请求打开，请等待网关就绪后再次发送');
  expect(node.textContent).not.toContain('已连接');
  await act(async () => node.querySelector<HTMLButtonElement>('[aria-label="关闭提示"]')!.click());
  expect(dismiss).toHaveBeenCalledOnce();
});
it('prevents duplicate launches and retains a useful retry action when opening fails', async () => {
  let fail!: (error: Error) => void;
  vi.mocked(call).mockImplementationOnce(() => new Promise((_, reject) => { fail = reject; }));
  await render();
  const button = openButton();
  await act(async () => { button.click(); button.click(); });
  expect(call).toHaveBeenCalledOnce(); expect(button.disabled).toBe(true);
  expect(node.textContent).toContain('正在打开…');
  await act(async () => fail(new Error('未找到 Magpie，请先下载并运行一次桌面版，然后重试。')));
  expect(node.textContent).toContain('打开失败：未找到 Magpie');
  expect(node.textContent).not.toContain('已请求打开');
  expect(openButton().disabled).toBe(false);
  await act(async () => openButton().click());
  expect(call).toHaveBeenCalledTimes(2);
  expect(node.textContent).not.toContain('打开失败');
});
it('does not add the launch action to unrelated errors mentioning Magpie', async () => {
  await render('Magpie 认证失败，请检查账号。');
  expect(node.textContent).toContain('Magpie 认证失败，请检查账号。');
  expect(openButton()).toBeUndefined(); expect(call).not.toHaveBeenCalled();
});
it('ignores a late launch result after the error changes', async () => {
  let finish!: () => void;
  vi.mocked(call).mockImplementationOnce(() => new Promise<void>(resolve => { finish = resolve; }));
  await render(); await act(async () => openButton().click());
  await render('其他连接错误');
  await act(async () => finish());
  expect(node.textContent).toContain('其他连接错误');
  expect(node.textContent).not.toContain('已请求打开');
  await render(); expect(openButton().disabled).toBe(false);
  expect(node.textContent).not.toContain('已请求打开');
});
