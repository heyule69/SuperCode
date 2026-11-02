// @vitest-environment happy-dom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { TurnConnection } from './TurnConnection';

let root: Root, node: HTMLDivElement;
const at = 2000000000000;
beforeEach(() => {
  (globalThis as any).IS_REACT_ACT_ENVIRONMENT = true;
  vi.useFakeTimers(); vi.setSystemTime(at);
  node = document.createElement('div'); document.body.append(node); root = createRoot(node);
});
afterEach(async () => { await act(async () => root.unmount()); node.remove(); vi.useRealTimers(); });
async function render(props: Partial<Parameters<typeof TurnConnection>[0]> = {}) { await act(async () => root.render(<TurnConnection active normalLabel="正在处理…" {...props} />)); }
async function advance(ms: number) { await act(async () => { await vi.advanceTimersByTimeAsync(ms); }); }

it('shows confirmed reconnection, real attempt counts and a truthful backoff countdown', async () => {
  await render({ connection:{ status:'retrying', at, kind:'network', attempt:2, maxAttempts:5, retryAt:at + 3000 } });
  expect(node.textContent).toContain('正在重新连接模型'); expect(node.textContent).toContain('第 2 / 5 次');
  expect(node.textContent).toContain('预计 3 秒后重试'); expect(node.querySelector('.working-indicator')).toBeNull();
  await advance(4000); expect(node.textContent).toContain('正在等待重试结果');
  expect(node.textContent).not.toContain('已恢复'); expect(node.textContent).not.toContain('预计 0 秒');
});
it('does not invent attempt counts or delays when the engine does not report them', async () => {
  await render({ connection:{ status:'retrying', at, kind:'unknown' } });
  expect(node.textContent).toContain('正在重试模型请求'); expect(node.textContent).not.toContain('次'); expect(node.textContent).not.toContain('秒后');
});
it('never displays a longer backoff than reported when a new retry arrives between clock ticks', async () => {
  await render();
  vi.setSystemTime(at + 900);
  await render({ connection:{ status:'retrying', at:at + 900, kind:'network', retryAt:at + 3900 } });
  expect(node.textContent).toContain('预计 3 秒后重试');
  expect(node.textContent).not.toContain('预计 4 秒');
});
it('explains long silence without claiming a disconnect or automatically retrying', async () => {
  await render(); expect(node.textContent).toBe('正在处理…');
  await advance(30000); expect(node.textContent).toContain('正在等待模型响应');
  expect(node.textContent).toContain('已 30 秒未收到新进展'); expect(node.textContent).toContain('无法确认是否正在重连');
  expect(node.querySelector('.connection-spinner')).toBeNull(); expect(node.querySelector('button')).toBeNull();
});
it('distinguishes a quiet tool and startup from a model network failure', async () => {
  await render({ waitingForTool:true }); await advance(31000);
  expect(node.textContent).toContain('工具仍在运行'); expect(node.textContent).toContain('这不代表连接已断开');
  await render({ waitingForTool:false, starting:true }); expect(node.textContent).toContain('正在等待 Agent 启动');
});
it('briefly confirms recovery then returns to the normal task status', async () => {
  await render({ connection:{ status:'restored', at, kind:'network' }, lastProgressAt:at });
  expect(node.textContent).toContain('连接已恢复'); await advance(6000); expect(node.textContent).toBe('正在处理…');
  await render({ active:false, normalLabel:undefined }); expect(node.textContent).toBe(''); expect(vi.getTimerCount()).toBe(0);
});
it('shows retry exhaustion, actual disconnect and intentional stop without a running animation', async () => {
  for (const [status, title] of [['failed', '重试未能恢复连接'], ['disconnected', '与 Agent 的连接已断开'], ['stopped', '已停止重试']]) {
    await render({ active:false, connection:{ status, at, kind:'network' } });
    expect(node.textContent).toContain(title); expect(node.querySelector('.connection-spinner')).toBeNull(); expect(vi.getTimerCount()).toBe(0);
  }
});
it('uses rate limit, authentication, billing and server retry explanations', async () => {
  for (const [kind, label] of [['rateLimit', '请求受限'], ['auth', '认证失败'], ['billing', '计费异常'], ['server', '服务暂不可用']]) {
    await render({ connection:{ status:'retrying', at, kind } }); expect(node.textContent).toContain(label);
  }
});
