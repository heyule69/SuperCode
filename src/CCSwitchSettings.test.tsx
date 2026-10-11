// @vitest-environment happy-dom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import CCSwitchSettings from './CCSwitchSettings';

const mocks = vi.hoisted(() => ({ call: vi.fn() }));
vi.mock('./api', () => ({ desktop: true, call: mocks.call }));
let root: Root, node: HTMLDivElement;
const updated = vi.fn(async () => {});
const database = 'C:\\Users\\测试\\.cc-switch\\cc-switch.db';
const button = (label: string) => [...node.querySelectorAll<HTMLButtonElement>('button')].find(item => item.textContent === label)!;
const scan = { path: database, profiles: [{ id: 'claude:valid', agent: 'claude', name: '有效连接', model: 'm', current: true }], warnings: ['错误配置：TOML 无效，已跳过'] };
beforeEach(() => {
  (globalThis as any).IS_REACT_ACT_ENVIRONMENT = true;
  node = document.createElement('div'); document.body.append(node); root = createRoot(node);
  updated.mockClear(); mocks.call.mockReset();
  mocks.call.mockImplementation(async (command: string) => command === 'scan_ccswitch' ? scan : 1);
});
afterEach(async () => { await act(async () => root.unmount()); node.remove(); });
async function render() { await act(async () => root.render(<CCSwitchSettings busy={false} updated={updated} back={() => {}} />)); }

it('automatically scans on entry, shows the detected path, and keeps valid rows when other rows fail', async () => {
  await render();
  expect(mocks.call).toHaveBeenCalledWith('scan_ccswitch', { path: null });
  expect(node.querySelector<HTMLInputElement>('#cc-path')!.value).toBe(database);
  expect(node.textContent).toContain('已自动识别 CC Switch · 1 个可导入连接');
  expect(node.querySelectorAll('.cc-source-row')).toHaveLength(1);
  expect(node.querySelector('.cc-scan-warnings')?.textContent).toContain('已跳过 1 个异常连接');
  expect(node.querySelector<HTMLInputElement>('input[type="checkbox"]')!.checked).toBe(true);
  await act(async () => button('导入所选 (1)').click());
  expect(mocks.call).toHaveBeenCalledWith('import_ccswitch', { path: database, ids: ['claude:valid'] });
  expect(updated).toHaveBeenCalledTimes(1);
});

it('clears old connections when editing the path and can automatically locate the database again', async () => {
  await render();
  await act(async () => {
    const input = node.querySelector<HTMLInputElement>('#cc-path')!;
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(input, 'D:\\missing');
    input.dispatchEvent(new Event('input', { bubbles: true }));
  });
  expect(node.querySelectorAll('.cc-source-row')).toHaveLength(0);
  expect(node.querySelector('.cc-scan-warnings')).toBeNull();
  expect(button('导入所选 (0)').disabled).toBe(true);
  await act(async () => button('自动识别').click());
  expect(mocks.call).toHaveBeenLastCalledWith('scan_ccswitch', { path: null });
  expect(node.querySelector<HTMLInputElement>('#cc-path')!.value).toBe(database);
});

it('distinguishes a database read failure from an empty scan and prevents stale imports', async () => {
  await render();
  mocks.call.mockRejectedValueOnce(new Error('数据库无法打开'));
  await act(async () => button('读取').click());
  expect(node.querySelector('[role="alert"]')?.textContent).toContain('数据库无法打开');
  expect(node.querySelector('.cc-source-list')?.textContent).toContain('读取失败');
  expect(node.querySelectorAll('.cc-source-row')).toHaveLength(0);
  expect(button('导入全部').disabled).toBe(true);
  expect(node.querySelector('.cc-scan-warnings')).toBeNull();
});
