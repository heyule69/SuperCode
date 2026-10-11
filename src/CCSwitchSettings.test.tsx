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
async function render(agent = 'claude') { await act(async () => root.render(<CCSwitchSettings agent={agent} busy={false} updated={updated} back={() => {}} />)); }

it('automatically scans on entry, shows the detected path, and keeps valid rows when other rows fail', async () => {
  await render();
  expect(mocks.call).toHaveBeenCalledWith('scan_ccswitch', { path: null, agent: 'claude' });
  expect(node.querySelector<HTMLInputElement>('#cc-path')!.value).toBe(database);
  expect(node.textContent).toContain('已自动识别 CC Switch · 1 个 Claude Code 可导入连接');
  expect(node.querySelectorAll('.cc-source-row')).toHaveLength(1);
  expect(node.querySelector('.cc-scan-warnings')?.textContent).toContain('已跳过 1 个异常连接');
  expect(node.querySelector<HTMLInputElement>('input[type="checkbox"]')!.checked).toBe(true);
  await act(async () => button('导入所选 (1)').click());
  expect(mocks.call).toHaveBeenCalledWith('import_ccswitch', { path: database, agent: 'claude', ids: ['claude:valid'] });
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
  expect(mocks.call).toHaveBeenLastCalledWith('scan_ccswitch', { path: null, agent: 'claude' });
  expect(node.querySelector<HTMLInputElement>('#cc-path')!.value).toBe(database);
});

const mixed = {
  path: database,
  profiles: [
    ...scan.profiles,
    { id: 'claude:other', agent: 'claude', name: '另一 Claude 连接', current: false },
    { id: 'codex:valid', agent: 'codex', name: 'Codex 连接', current: true },
    { id: 'opencode:valid', agent: 'opencode', name: 'OpenCode 连接', model: 'kimi-k3', current: true },
    { id: 'pi:valid', agent: 'pi', name: 'Pi 连接', model: 'glm-5.1', current: true },
  ],
  warnings: [],
};
it.each(['claude', 'codex', 'opencode', 'pi'])('limits rows, selection, counts, and Import All to %s even with a mixed response', async agent => {
  mocks.call.mockImplementation(async (command: string) => command === 'scan_ccswitch' ? mixed : 1);
  await render(agent);
  const own = mixed.profiles.filter(p => p.agent === agent);
  expect(node.querySelectorAll('.cc-source-row')).toHaveLength(own.length);
  expect(node.querySelector('.cc-list-toolbar')?.textContent).toContain(`已选 1 / ${own.length}`);
  await act(async () => button('全选').click());
  expect(button(`导入所选 (${own.length})`).disabled).toBe(false);
  await act(async () => button(`导入所选 (${own.length})`).click());
  expect(mocks.call).toHaveBeenLastCalledWith('import_ccswitch', { path: database, agent, ids: own.map(p => p.id) });
  await act(async () => button('导入全部').click());
  expect(mocks.call).toHaveBeenLastCalledWith('import_ccswitch', { path: database, agent, ids: own.map(p => p.id) });
});

it.each(['opencode', 'pi'])('does not offer another Agent configuration when %s has no connections', async agent => {
  mocks.call.mockResolvedValue({ ...mixed, profiles: mixed.profiles.filter(p => !['opencode', 'pi'].includes(p.agent)) });
  await render(agent);
  expect(node.querySelectorAll('.cc-source-row')).toHaveLength(0);
  expect(node.querySelector('.cc-list-toolbar')?.textContent).toContain('已选 0 / 0');
  expect(button('导入全部').disabled).toBe(true);
  expect(button('导入所选 (0)').disabled).toBe(true);
  expect(node.querySelector('.cc-source-list')?.textContent).toContain(`暂无 ${agent === 'pi' ? 'Pi' : 'OpenCode'} 可导入连接`);
});

it('keeps the database across Agent switches, resets selection, and ignores a delayed scan from the previous tab', async () => {
  const manual = 'D:\\共享配置\\cc-switch.db';
  let finish!: (value: typeof mixed) => void;
  const pending = new Promise<typeof mixed>(resolve => { finish = resolve; });
  mocks.call.mockImplementation(async (command: string, args: { agent: string }) => command === 'scan_ccswitch' && args.agent === 'claude' ? pending : mixed);
  await render();
  await render('codex');
  expect(node.querySelectorAll('.cc-source-row')).toHaveLength(1);
  expect(node.querySelector('.cc-source-list')?.textContent).toContain('Codex 连接');
  await act(async () => { finish(mixed); await pending; });
  expect(node.querySelectorAll('.cc-source-row')).toHaveLength(1);
  expect(node.querySelector('.cc-source-list')?.textContent).not.toContain('有效连接');
  await act(async () => button('清空').click());
  await act(async () => {
    const input = node.querySelector<HTMLInputElement>('#cc-path')!;
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(input, manual);
    input.dispatchEvent(new Event('input', { bubbles: true }));
  });
  mocks.call.mockResolvedValue({ ...mixed, path: manual });
  await render('claude');
  expect(mocks.call).toHaveBeenLastCalledWith('scan_ccswitch', { path: manual, agent: 'claude' });
  expect(node.querySelector('.cc-list-toolbar')?.textContent).toContain('已选 1 / 2');
  expect(node.querySelector<HTMLInputElement>('#cc-path')!.value).toBe(manual);
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
