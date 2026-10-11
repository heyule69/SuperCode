// @vitest-environment happy-dom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { Conversation } from './Conversation';
import { mergeEvent, reconcileMessages } from './events';
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

it('collapses immediately on completion and stays collapsed through a stale history refresh', async () => {
  const started = mergeEvent(items, {method:'turn/started',params:{turn:{id:'t'}}}, 's');
  await render(started, true);
  const completed = mergeEvent(started, {method:'turn/completed',params:{turn:{id:'t',status:'completed'}}}, 's');
  await render(completed, true);
  await render(reconcileMessages(completed, started.map((message,index) => ({...message,seq:index + 1})), new Set()), true);
  expect(node.querySelector('.turn-process-toggle')?.getAttribute('aria-expanded')).toBe('false');
  expect(node.querySelector('.turn-process-content')).toBeNull();
  expect(node.textContent).not.toContain('我会先检查布局');
  expect(node.querySelector('.turn-final-result')?.textContent).toContain('已完成布局优化');
  await toggle();
  // A later persisted native timestamp must not close details the user opened.
  const saved = completed.map((message,index) => ({ ...message, seq:index + 1,
    data:message.kind === 'runMarker' ? { ...message.data, startedAt:Number(message.data?.startedAt) - 50 } : message.data }));
  await render(reconcileMessages(completed, saved, new Set()), true);
  expect(node.querySelector('.turn-process-content')?.textContent).toContain('我会先检查布局');
});

it('closes manually expanded process details when the same mounted turn starts and finishes again', async () => {
  await render([...items, run('completed')]);
  await toggle();
  expect(node.querySelector('.turn-process-content')).not.toBeNull();
  await render([...items, run('inProgress')]);
  await render([...items, run('completed')]);
  expect(node.querySelector('.turn-process-toggle')?.getAttribute('aria-expanded')).toBe('false');
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

it('renders and copies readable questions through streaming, completion and expanding past history', async () => {
  const question = { question: '插件的界面形态用哪种？', header: '界面形态', options: [{ label: '全页 Dashboard', description: '完整管理面板' }, { label: 'Popup 弹窗' }] };
  const preparing = message('ask', 'tool', '', 'claudeToolCall', { tool: 'AskUserQuestion', arguments: {}, inputText: '{"questions":', status: 'preparing' });
  await render([items[0], preparing, run('inProgress')], true);
  expect(node.textContent).toContain('正在生成问题');
  expect(node.querySelector('.activity-body pre')).toBeNull();
  const pending = { ...preparing, data: { tool: 'AskUserQuestion', arguments: { questions: [question] }, status: 'inProgress' } };
  await render([items[0], pending, run('inProgress')], true);
  expect(node.textContent).toContain(question.question);
  expect(node.textContent).toContain('完整管理面板');
  expect(node.textContent).not.toContain('输入参数');
  expect(node.querySelector('.question-history button')?.getAttribute('aria-label')).toBe('复制提问详情');
  const completed = { ...pending, text: JSON.stringify({ answers: { [question.question]: 'Popup 弹窗' } }), data: { ...pending.data, status: 'completed' } };
  await render([items[0], completed, items[3], run('completed')], true);
  expect(node.querySelector('.question-history')).toBeNull();
  await toggle();
  expect(node.querySelector('.question-history-answer')?.textContent).toBe('你的回答Popup 弹窗');
  expect(node.querySelector('.question-history .is-selected')?.textContent).toContain('Popup 弹窗');
  await act(async () => node.querySelector<HTMLButtonElement>('[aria-label="复制提问详情"]')!.click());
  expect(writeText).toHaveBeenCalledWith('界面形态\n插件的界面形态用哪种？\n1. 全页 Dashboard\n完整管理面板\n2. Popup 弹窗\n你的回答：Popup 弹窗');
  await toggle();
  expect(node.querySelector('.question-history')).toBeNull();
});

it('retains technical input and output details for ordinary tools', async () => {
  await render([items[0], message('bash', 'tool', '构建通过', 'claudeToolCall', { tool: 'Bash', arguments: { command: 'npm run build' }, status: 'completed' }), run('inProgress')], true);
  expect(node.textContent).toContain('输入参数');
  expect(node.textContent).toContain('工具结果');
  expect(node.querySelectorAll('.activity-body pre')).toHaveLength(2);
  expect(node.querySelector('.question-history')).toBeNull();
});

it('shows the readable plan document when expanding a submitted plan in execution history', async () => {
  const plan = '# 下载功能计划\n\n## 第一步\n\n实现预览。\n\n## 验证\n\n确认下载结果。';
  await render([items[0], message('plan', 'tool', '已完成计划提交', 'claudeToolCall', { tool: 'ExitPlanMode', arguments: { plan }, status: 'completed' }), items[3], run('completed')], true);
  expect(node.querySelector('.plan-document')).toBeNull();
  await toggle(); await act(async () => { await vi.dynamicImportSettled(); });
  expect(node.querySelector('.plan-document h1')?.textContent).toBe('下载功能计划');
  expect(node.textContent).toContain('确认下载结果');
  expect(node.textContent).not.toContain('输入参数');
  expect(node.querySelector('.plan-request-execute')).toBeNull();
});
