// @vitest-environment happy-dom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { Conversation } from './Conversation';
import type { Message } from './types';

vi.mock('./api', () => ({ desktop: true, call: vi.fn(async () => ({ token: 'full', previewToken: 'small' })) }));
vi.mock('@tauri-apps/api/core', () => ({ convertFileSrc: (token: string) => `http://supercode-media.localhost/${token}` }));
const message = (id: string, role: string, text: string, kind = 'agentMessage', data: Message['data'] = null): Message => ({ id, seq: 0, sessionId: 's', role, kind, text, data });
const items = [message('u', 'user', '优化页面'), message('start', 'assistant', '我会先检查布局。'), message('tool', 'tool', '构建通过', 'commandExecution', { command: 'npm run build', status: 'completed' }), message('final', 'assistant', '已完成布局优化。')];
const run = (status: string, id = 'run') => message(id, 'activity', '', 'runMarker', { status, durationMs: 691000 });
let root: Root, node: HTMLDivElement;
const writeText = vi.fn(async () => {});
beforeEach(async () => {
  (globalThis as any).IS_REACT_ACT_ENVIRONMENT = true;
  vi.stubGlobal('IntersectionObserver', undefined);
  Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true }); writeText.mockClear();
  node = document.createElement('div'); document.body.append(node); root = createRoot(node);
  await act(async () => { await import('./Markdown'); });
});
afterEach(async () => { await act(async () => root.unmount()); node.remove(); vi.unstubAllGlobals(); });
async function render(messages: Message[], autoExpand = false) { await act(async () => root.render(<Conversation messages={messages} agentName="Codex" autoExpand={autoExpand} showDiff={() => {}} openFile={() => {}}/>)); }
async function toggle() { await act(async () => node.querySelector<HTMLButtonElement>('.turn-process-toggle')!.click()); }

it('streams the full process, collapses at completion, and expands the original chronology without duplicating the final answer', async () => {
  await render([...items, run('inProgress')]);
  expect(node.textContent).toContain('我会先检查布局');
  expect(node.querySelector('.activity-group-summary')).not.toBeNull();
  const final = node.querySelector('.message.assistant:last-child');
  await render([...items, run('completed')]);
  expect(node.querySelector('.turn-final-result .message')).toBe(final);
  expect(node.textContent).not.toContain('我会先检查布局');
  expect(node.textContent).toContain('用时 11 分钟 31 秒');
  expect(node.querySelector('.activity-group-summary')).toBeNull();
  const button = node.querySelector<HTMLButtonElement>('.turn-process-toggle')!;
  expect(button.getAttribute('aria-expanded')).toBe('false');
  await toggle();
  const process = node.querySelector('.turn-process-content')!;
  expect(button.getAttribute('aria-expanded')).toBe('true');
  expect(process.textContent).toContain('我会先检查布局');
  expect(process.textContent).toContain('运行了命令');
  expect(process.textContent).not.toContain('已完成布局优化');
  expect(node.textContent!.match(/已完成布局优化/g)).toHaveLength(1);
  await toggle();
  expect(node.querySelector('.turn-process-content')).toBeNull();
  expect(node.querySelector('.turn-final-result .message')).toBe(final);
  await act(async () => node.querySelector<HTMLButtonElement>('[aria-label="复制回复"]')!.click());
  expect(writeText).toHaveBeenCalledWith('已完成布局优化。');
});

it('keeps completed history collapsed even when individual tool details auto-expand, and resets for a new run', async () => {
  await render([...items, run('completed')], true);
  expect(node.querySelector('.activity-row')).toBeNull();
  await toggle();
  expect(node.querySelector('.activity-body')?.textContent).toContain('npm run build');
  await render([...items, run('inProgress', 'new-run')], true);
  await render([...items, run('completed', 'new-run')], true);
  expect(node.querySelector('.turn-process-content')).toBeNull();
});

it('keeps stop and error information visible rather than presenting partial output as a completed result', async () => {
  for (const status of ['failed', 'interrupted']) {
    await render([...items, message('notice', 'system', '任务未完成'), run(status)]);
    expect(node.querySelector('.turn-process-toggle')).toBeNull();
    expect(node.textContent).toContain('我会先检查布局');
    expect(node.textContent).toContain('任务未完成');
    expect(node.textContent).toContain(status === 'failed' ? '运行失败' : '已停止');
  }
});

it('keeps a final media player mounted when the process finishes or is toggled', async () => {
  const withVideo = [...items.slice(0, -1), message('final', 'assistant', '![演示](<D:/output/demo.mp4>)', 'agentMessage', { phase: 'final_answer' })];
  await render([...withVideo, run('inProgress')]);
  await act(async () => node.querySelector<HTMLButtonElement>('.chat-video-start')!.click());
  const video = node.querySelector('video');
  await render([...withVideo, run('completed')]);
  expect(node.querySelector('video')).toBe(video);
  await toggle(); await toggle();
  expect(node.querySelector('video')).toBe(video);
});
