// @vitest-environment happy-dom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import RequestCard from './RequestCard';
import type { RpcEvent } from './types';

vi.mock('./api', () => ({ desktop: false, call: vi.fn() }));
const plan = '# ImageHive 实施计划\n\n## 实现步骤\n\n1. 创建 Dashboard\n2. 支持图片批量下载\n\n| 内容 | 状态 |\n| --- | --- |\n| 图片 | 首期 |\n\n```ts\nconst preview = true;\n```\n\n## 验收\n\n确认图片可下载。\n\n[说明文档](./docs/plan.md)';
const request = (input: unknown = { plan }, id = 'claude:plan'): RpcEvent => ({ id, method: 'claude/tool/requestApproval', params: { toolName: 'ExitPlanMode', input } });
const respond = vi.fn(), openFile = vi.fn();
let root: Root, node: HTMLDivElement;
beforeEach(async () => {
  (globalThis as any).IS_REACT_ACT_ENVIRONMENT = true;
  respond.mockReset(); respond.mockResolvedValue(undefined); openFile.mockReset();
  node = document.createElement('div'); document.body.append(node); root = createRoot(node);
  await act(async () => { await import('./PlanRequest'); await import('./Markdown'); });
});
afterEach(async () => { await act(async () => root.unmount()); node.remove(); });
async function render(value = request()) {
  await act(async () => { root.render(<RequestCard request={value} respond={respond} openFile={openFile} />); await vi.dynamicImportSettled(); });
}
async function click(selector: string) { await act(async () => node.querySelector<HTMLButtonElement>(selector)!.click()); }
async function feedback(value: string) {
  const input = node.querySelector<HTMLTextAreaElement>('[aria-label="计划修改意见"]')!;
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')!.set!.call(input, value);
    input.dispatchEvent(new Event('input', { bubbles: true }));
  });
}

it('renders the whole plan as Markdown before requiring explicit native execution approval', async () => {
  await render();
  expect(node.querySelector('.plan-document h1')?.textContent).toBe('ImageHive 实施计划');
  expect(node.querySelector('.plan-document table')?.textContent).toContain('首期');
  expect(node.querySelector('.plan-document pre')?.textContent).toContain('const preview = true;');
  expect(node.querySelector('.plan-document')?.textContent).toContain('确认图片可下载。');
  expect(node.textContent).not.toContain('请求使用 ExitPlanMode');
  expect(node.textContent).not.toContain('"plan":');
  expect(respond).not.toHaveBeenCalled();
  await click('.plan-document a'); expect(openFile).toHaveBeenCalledWith('./docs/plan.md', undefined);
  await click('.plan-request-execute');
  expect(respond).toHaveBeenCalledExactlyOnceWith({ decision: 'accept' });
});

it('sends revision feedback through denial, preserving newlines and never granting permission', async () => {
  await render(); await feedback('  第一阶段只做图片\n视频延后  '); await click('.plan-request-revise');
  expect(respond).toHaveBeenCalledExactlyOnceWith({ decision: 'decline', feedback: '第一阶段只做图片\n视频延后' });
  expect(node.textContent).toContain('已要求修改');
  expect(node.querySelector<HTMLButtonElement>('.plan-request-execute')?.disabled).toBe(true);
});

it('allows continuing to plan without extra feedback', async () => {
  await render(); await click('.plan-request-revise');
  expect(respond).toHaveBeenCalledExactlyOnceWith({ decision: 'decline', feedback: '' });
});

it('does not approve a missing or invalid plan and lets the user request its completion', async () => {
  for (const input of [{}, { plan: { text: 'not a document' } }, '{broken json', { plan: '  ' }]) {
    await render(request(input, JSON.stringify(input)));
    expect(node.textContent).toContain('尚未提供完整计划');
    expect(node.querySelector<HTMLButtonElement>('.plan-request-execute')?.disabled).toBe(true);
    await click('.plan-request-execute'); expect(respond).not.toHaveBeenCalled();
  }
  await click('.plan-request-revise');
  expect(respond).toHaveBeenCalledExactlyOnceWith({ decision: 'decline', feedback: '' });
});

it('retains the full long document without truncating its ending or exposing JSON escapes', async () => {
  const long = '# 长计划\n\n' + Array.from({ length: 80 }, (_, i) => `## 阶段 ${i + 1}\n\n完整说明：实现与验证。`).join('\n\n') + '\n\n最后确认步骤。';
  await render(request(JSON.stringify({ plan: long })));
  expect(node.querySelectorAll('.plan-document h2')).toHaveLength(80);
  expect(node.textContent).toContain('最后确认步骤');
  expect(node.textContent).not.toContain('\\n');
  expect(respond).not.toHaveBeenCalled();
});

it('prevents duplicate decisions and preserves revision feedback when submission fails', async () => {
  let reject!: (reason: Error) => void;
  respond.mockImplementationOnce(() => new Promise((_, fail) => { reject = fail; }));
  await render(); await feedback('不要增加依赖'); await click('.plan-request-revise'); await click('.plan-request-execute');
  expect(respond).toHaveBeenCalledOnce();
  await act(async () => reject(new Error('暂时断开连接')));
  expect(node.querySelector('[role="alert"]')?.textContent).toBe('暂时断开连接');
  expect(node.querySelector<HTMLTextAreaElement>('textarea')?.value).toBe('不要增加依赖');
  await click('.plan-request-revise');
  expect(respond).toHaveBeenLastCalledWith({ decision: 'decline', feedback: '不要增加依赖' });
});

it('resets the confirmation state when the Agent submits a revised plan', async () => {
  await render(); await feedback('缩小首期范围'); await click('.plan-request-revise');
  await render(request({ plan: '# 新计划\n\n只实现图片。' }, 'claude:revised'));
  expect(node.textContent).toContain('只实现图片');
  expect(node.querySelector<HTMLTextAreaElement>('textarea')?.value).toBe('');
  expect(node.querySelector<HTMLButtonElement>('.plan-request-execute')?.disabled).toBe(false);
  await click('.plan-request-execute'); expect(respond).toHaveBeenLastCalledWith({ decision: 'accept' });
});

it('does not treat an ordinary operation containing a plan field as plan confirmation', async () => {
  await render({ ...request(), params: { toolName: 'Bash', input: { plan: 'example', command: 'build' } } });
  expect(node.querySelector('.plan-request')).toBeNull();
  expect(node.querySelector('.request-card')).not.toBeNull();
  expect(respond).not.toHaveBeenCalled();
});
