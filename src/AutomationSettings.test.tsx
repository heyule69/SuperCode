// @vitest-environment happy-dom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { beforeEach, afterEach, expect, it, vi } from 'vitest';
import AutomationSettings from './AutomationSettings';
import { call } from './api';
import type { AutomationStatus } from './automation';

vi.mock('./api', () => ({ desktop: true, call: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }));
const fresh: AutomationStatus[] = [
  { kind: 'browser', phase: 'ready', message: '软件自带 · 尚未测试', enabled: true, received: null, total: null, installation: { version: '1', tools: 6, testedAt: 0, summary: '随 SuperCode 提供，无需另行安装。尚未进行本机测试。', root: 'app' } },
  { kind: 'computer', phase: 'ready', message: '软件自带 · 尚未测试', enabled: true, received: null, total: null, installation: { version: '1', tools: 9, testedAt: 0, summary: '随 SuperCode 提供，无需另行安装。尚未进行本机测试。', root: 'app' } },
];
let node: HTMLDivElement, root: Root;
beforeEach(() => { (globalThis as any).IS_REACT_ACT_ENVIRONMENT = true; node = document.createElement('div'); document.body.append(node); root = createRoot(node); vi.mocked(call).mockReset(); vi.mocked(call).mockResolvedValue(fresh); });
afterEach(async () => { await act(async () => root.unmount()); node.remove(); });
async function render() { await act(async () => root.render(<AutomationSettings busy={false}/>)); }
it('makes bundled computer control available without pretending it has passed a test or requiring installation', async () => {
  await render();
  const computer = node.querySelector('[aria-label="电脑自动化"]')!;
  expect(computer.textContent).toContain('软件自带 · 尚未测试');
  expect(computer.textContent).not.toContain('测试通过');
  expect(computer.textContent).not.toContain('上次通过测试');
  expect(computer.textContent).not.toContain('修复安装');
  expect(computer.textContent).not.toContain('安装并测试');
  expect(computer.textContent).toContain('SuperCode 正在控制电脑 · 按 Esc 取消');
  expect(computer.textContent).toContain('普通鼠标、键盘操作和窗口切换不会取消控制');
  const browser = node.querySelector('[aria-label="浏览器自动化"]')!;
  expect(browser.textContent).toContain('测试浏览器');
  expect(browser.textContent).not.toContain('安装并测试');
  expect(browser.textContent).not.toContain('修复安装');
  const test = [...computer.querySelectorAll('button')].find(b => b.textContent?.includes('测试电脑操作'))!;
  await act(async () => test.click());
  expect(call).toHaveBeenCalledWith('test_automation', { kind: 'computer' });
});
it('shows a verified timestamp only after a real probe returns it', async () => {
  vi.mocked(call).mockResolvedValue(fresh.map(s => s.kind === 'computer' ? { ...s, installation: { ...s.installation!, testedAt: 1234567890, summary: '已验证截图和窗口读取' } } : s));
  await render();
  const computer = node.querySelector('[aria-label="电脑自动化"]')!;
  expect(computer.textContent).toContain('测试通过');
  expect(computer.textContent).toContain('上次通过测试');
  expect(computer.textContent).toContain('已验证截图和窗口读取');
});
