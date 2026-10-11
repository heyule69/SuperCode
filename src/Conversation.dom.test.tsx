// @vitest-environment happy-dom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { Conversation } from './Conversation';
import { mergeEvent, reconcileMessages } from './events';
import type { Message } from './types';
import type { TurnLifecycle } from './turnPresentation';

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
async function render(messages: Message[], autoExpand = false, lifecycle: TurnLifecycle = {}) { await act(async () => root.render(<Conversation messages={messages} agentName="Codex" autoExpand={autoExpand} sessionStatus={lifecycle.sessionStatus} activeTurnId={lifecycle.activeTurnId} showDiff={() => {}} openFile={() => {}}/>)); }
async function toggle() { await act(async () => node.querySelector<HTMLButtonElement>('.turn-process-toggle')!.click()); }

it('streams the full process, collapses at completion, and expands the original chronology without duplicating the final answer', async () => {
  await render([...items, run('inProgress')]);
  expect(node.textContent).toContain('我会先检查布局');
  expect(node.querySelector('.activity-group-summary')).not.toBeNull();
  expect(node.querySelector('.turn-footer')).toBeNull();
  const final = [...node.querySelectorAll('.message.assistant')].at(-1);
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

it('hides reply actions while starting, running or waiting without a run marker, then restores them on completion', async () => {
  for (const sessionStatus of ['starting', 'running', 'waiting']) {
    await render(items, false, { sessionStatus });
    expect(node.textContent).toContain('已完成布局优化');
    expect(node.querySelector('[aria-label="复制回复"]')).toBeNull();
    expect(node.querySelector('[aria-label="复制消息"]')).not.toBeNull();
  }
  await render(items, false, { sessionStatus: 'idle' });
  await act(async () => node.querySelector<HTMLButtonElement>('[aria-label="复制回复"]')!.click());
  expect(writeText).toHaveBeenCalledWith('已完成布局优化。');
});

it('hides actions for streaming assistant messages before session or run metadata arrives', async () => {
  for (const status of ['inProgress', 'running', 'preparing']) {
    await render([items[0], { ...items[1], data: { status } }]);
    expect(node.querySelector('.turn-footer')).toBeNull();
  }
});

it('keeps actions for completed history while the current reply is still generating', async () => {
  const old = [...items, run('completed')];
  const current = [message('u2', 'user', '继续优化'), message('a2', 'assistant', '我会检查下一处。')];
  await render([...old, ...current], false, { sessionStatus: 'running', activeTurnId: 't2' });
  const turns = node.querySelectorAll('.assistant-turn');
  expect(turns[0].querySelector('[aria-label="复制回复"]')).not.toBeNull();
  expect(turns[1].querySelector('.turn-footer')).toBeNull();
  await render([...old, ...current, run('completed', 'run2')], false, { sessionStatus: 'running' });
  expect(node.querySelectorAll('[aria-label="复制回复"]')).toHaveLength(2);
});

it('shows animated current reasoning, editing and running after a tool failure without a run marker', async () => {
  const failed = message('failure', 'tool', '下载失败', 'commandExecution', { command: 'node download.mjs', status: 'completed', exitCode: 1 });
  const thinking = message('thinking', 'activity', '我会检查失败原因。', 'reasoning', { status: 'inProgress' });
  const trace = [items[0], failed, thinking];
  for (const [current, label] of [
    [thinking, '正在思考'],
    [message('edit', 'tool', '', 'claudeToolCall', { tool: 'Write', arguments: { file_path: 'scripts/test.mjs' }, status: 'preparing' }), '正在编辑文件'],
    [message('edit', 'tool', '', 'fileChange', { changes: [{ path: 'scripts/test.mjs' }], status: 'inProgress' }), '正在编辑文件'],
    [message('command', 'tool', '', 'commandExecution', { command: 'node scripts/test.mjs', status: 'running' }), '正在运行命令'],
    [message('native-tool', 'tool', '', 'mcpToolCall', { tool: 'test', status: 'inProgress' }), '正在调用工具'],
  ] as const) {
    await render([...trace.filter(message => message.id !== current.id), current], true, { sessionStatus: 'running' });
    const summary = node.querySelector('.activity-group-summary')!;
    expect(summary.textContent).toContain(label);
    expect(summary.textContent).not.toContain('调用失败');
    expect(summary.querySelector('.pending-text')?.textContent).toBe(label);
    expect(summary.closest('.activity-group')?.classList.contains('failed')).toBe(false);
    expect(node.querySelector('.activity-item.failed')?.textContent).toContain('失败');
  }
});

it('keeps one animated status across tool completion and text output, then removes it when the turn ends', async () => {
  const failed = message('failure', 'tool', '失败', 'commandExecution', { status: 'completed', exitCode: 1 });
  const trace = [items[0], failed];
  await render(trace, false, { sessionStatus: 'running' });
  expect(node.querySelector('.activity-group-summary .pending-text')?.textContent).toBe('正在思考');
  expect(node.querySelector('.working-indicator')).toBeNull();
  const commentary = message('commentary', 'assistant', '正在检查失败原因。', 'agentMessage', { phase: 'commentary' });
  await render([...trace, commentary], false, { sessionStatus: 'running' });
  expect(node.querySelector('.working-indicator .pending-text')?.textContent).toBe('正在思考');
  expect(node.querySelectorAll('.working-indicator')).toHaveLength(1);
  expect(node.querySelector('.turn-footer')).toBeNull();
  await render([...trace, commentary, items[3], run('completed')], false, { sessionStatus: 'idle' });
  expect(node.querySelector('.pending-text')).toBeNull();
  expect(node.querySelector('.working-indicator')).toBeNull();
  expect(node.querySelector('[aria-label="复制回复"]')).not.toBeNull();
  await toggle();
  const history = node.querySelector<HTMLButtonElement>('.turn-process-content .activity-group-summary')!;
  expect(history.textContent).toContain('调用失败');
  await act(async () => history.click());
  expect(node.querySelector('.activity-item.failed')?.textContent).toContain('失败');
});

it('keeps active tool progress visible when assistant text arrives while the tool is still running', async () => {
  const tool = message('command', 'tool', '', 'commandExecution', { command: 'npm run build', status: 'running' });
  await render([items[0], tool, items[1]], false, { sessionStatus: 'running' });
  expect(node.querySelector('.working-indicator .pending-text')?.textContent).toBe('正在运行命令');
});

it('shows progress before any assistant event arrives in both empty and optimistic user-message histories', async () => {
  for (const messages of [[], [items[0]], [items[0], run('inProgress')]]) {
    await render(messages, false, { sessionStatus: 'starting' });
    expect(node.querySelectorAll('.working-indicator')).toHaveLength(1);
    expect(node.querySelector('.working-indicator .pending-text')?.textContent).toBe('正在启动 Agent…');
  }
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

it('opens old idle history already collapsed without receiving a live completion event', async () => {
  for (const marker of [[], [run('inProgress')]]) {
    await render([...items, ...marker], true, { sessionStatus: 'idle' });
    expect(node.querySelector('.turn-process-toggle')?.getAttribute('aria-expanded')).toBe('false');
    expect(node.textContent).not.toContain('我会先检查布局');
    expect(node.querySelector('.turn-final-result')?.textContent).toContain('已完成布局优化');
    await toggle();
    expect(node.querySelector('.turn-process-content')?.textContent).toContain('我会先检查布局');
    await toggle();
  }
});

it('restores a paginated run and keeps its final reply and manually expanded process stable when earlier pages arrive', async () => {
  const marker = run('completed', 'run-old');
  const page = items.slice(1).map((item, index) => ({ ...item, seq: index + 101,
    ...(index === 0 ? { historyRun: { id: marker.id, turnKey: items[0].id, data: marker.data } } : {}) }));
  await render(page, true);
  expect(node.querySelector('.turn-process-toggle')?.getAttribute('aria-expanded')).toBe('false');
  const final = node.querySelector('.turn-final-result .message');
  await toggle();
  await render([marker, message('earlier', 'assistant', '更早的过程'), ...page], true);
  expect(node.querySelector('.turn-process-content')?.textContent).toContain('更早的过程');
  expect(node.querySelector('.turn-final-result .message')).toBe(final);
  await render([items[0], marker, message('earlier', 'assistant', '更早的过程'), ...page], true);
  expect(node.querySelector('.turn-final-result .message')).toBe(final);
  expect(node.querySelector('.turn-process-content')?.textContent).toContain('更早的过程');
  expect(node.querySelector('.turn-process-content')?.textContent).toContain('我会先检查布局');
  expect(node.textContent!.match(/已完成布局优化/g)).toHaveLength(1);
  await toggle();
  expect(node.querySelector('.turn-final-result .message')).toBe(final);
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
