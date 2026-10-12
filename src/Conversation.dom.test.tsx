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
const items = [message('u', 'user', '优化页面'), message('start', 'assistant', '我会先检查布局。', 'agentMessage', { phase:'commentary' }), message('tool', 'tool', '构建通过', 'commandExecution', { command: 'npm run build', status: 'completed' }), message('final', 'assistant', '已完成布局优化。')];
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
it('opens file operations in the side review and keeps command output behind its own detail toggle',async()=>{
  const diff='--- a/src/中文.ts\n+++ b/src/中文.ts\n@@ -1,2 +1,3 @@\n stable\n-old-value\n+new-value\n+added-value\n';
  const edits=Array.from({length:5},(_,i)=>message(`edit-${i}`,'tool','TOOL_RESULT_'+i,'claudeToolCall',{tool:i?'Edit':'Write',status:'completed',arguments:{file_path:'src/中文.ts'},changes:[{path:'src/中文.ts',kind:{type:i?'update':'add'},diff,added:2,removed:1}]}));
  const command='node "scripts/中文.mjs" --check';
  const check=message('check','tool',command+'\nFULL_COMMAND_OUTPUT','commandExecution',{command,exitCode:0,durationMs:1300,cwd:'D:/test',status:'completed'});
  const showDiff=vi.fn(), openFile=vi.fn();
  await act(async()=>root.render(<Conversation messages={[message('u','user','test'),...edits,check]} agentName="Pi" sessionStatus="idle" autoExpand={false} showDiff={showDiff} openFile={openFile}/>));
  expect(node.querySelectorAll('.activity-row')).toHaveLength(0);
  await act(async()=>node.querySelector<HTMLButtonElement>('.activity-group-summary')!.click());
  expect(node.querySelectorAll('.activity-row')).toHaveLength(6);
  expect(node.querySelector('.activity-code-diff')).toBeNull();
  expect(node.textContent).not.toContain('FULL_COMMAND_OUTPUT');
  expect(node.querySelector('.activity-row')!.textContent).toContain('已创建');
  expect(node.querySelector('.activity-row')!.textContent).toContain('+2−1');
  expect(node.querySelectorAll('.activity-row')[5].textContent).toContain(command);
  await act(async()=>node.querySelector<HTMLButtonElement>('.activity-row')!.click());
  expect(showDiff).toHaveBeenCalledWith('本次变更 · src/中文.ts',`diff --git a/src/中文.ts b/src/中文.ts\n${diff}`);
  expect(openFile).not.toHaveBeenCalled();
  expect(node.querySelector('.activity-body')).toBeNull();
  expect(node.textContent).not.toContain('new-value');
  expect(node.querySelector('.activity-row')?.hasAttribute('aria-expanded')).toBe(false);
  await act(async()=>node.querySelectorAll<HTMLButtonElement>('.activity-row')[5].click());
  expect(node.textContent).toContain('FULL_COMMAND_OUTPUT');
  expect(node.textContent).toContain('退出码 0');
  expect(node.textContent).toContain('D:/test');
  expect(node.querySelector('.activity-toggle-more')).toBeNull();
});
it('opens multi-file operation patches in the side review even when automatic detail expansion is enabled',async()=>{
  const showDiff=vi.fn();
  const first='diff --git a/a.ts b/a.ts\n@@ -1 +1 @@\n-old-a\n+new-a\n';
  const second='@@ -1 +1 @@\n-old-b\n+new-b\n';
  const file=message('files','tool','applied','fileChange',{status:'completed',changes:[{path:'a.ts',diff:first,added:1,removed:1},{path:'b.ts',diff:second,added:1,removed:1}]});
  await act(async()=>root.render(<Conversation messages={[message('u','user','修改文件'),file]} agentName="Codex" autoExpand={true} sessionStatus="idle" showDiff={showDiff} openFile={()=>{}}/>));
  expect(node.querySelector('.activity-body')).toBeNull();
  expect(node.textContent).not.toContain('new-a');
  await act(async()=>node.querySelector<HTMLButtonElement>('.activity-row')!.click());
  expect(showDiff).toHaveBeenCalledWith('本次变更',`${first}\ndiff --git a/b.ts b/b.ts\n${second}`);
  expect(node.querySelector('.activity-body')).toBeNull();
});
it('keeps legacy edit arguments out of the conversation when the operation patch was not recorded',async()=>{
  const showDiff=vi.fn();
  const edit=message('old-edit','tool','saved','claudeToolCall',{tool:'Edit',status:'completed',arguments:{file_path:'old.ts',old_string:'OLD_PRIVATE_CODE',new_string:'NEW_PRIVATE_CODE'}});
  await act(async()=>root.render(<Conversation messages={[message('u','user','修改文件'),edit]} agentName="Claude" autoExpand={true} sessionStatus="idle" showDiff={showDiff} openFile={()=>{}}/>));
  expect(node.querySelector('.activity-body')).toBeNull();
  expect(node.textContent).not.toContain('NEW_PRIVATE_CODE');
  await act(async()=>node.querySelector<HTMLButtonElement>('.activity-row')!.click());
  expect(showDiff).toHaveBeenCalledWith('本次变更 · old.ts','此记录没有保存可展示的代码差异。');
});
async function render(messages: Message[], autoExpand = false, lifecycle: TurnLifecycle = {}) { await act(async () => root.render(<Conversation messages={messages} agentName="Codex" autoExpand={autoExpand} sessionStatus={lifecycle.sessionStatus} activeTurnId={lifecycle.activeTurnId} showDiff={() => {}} openFile={() => {}}/>)); }
async function toggle() { await act(async () => node.querySelector<HTMLButtonElement>('.turn-process-toggle')!.click()); }

it('offers execution only on the latest successfully completed native plan, not on streaming or historical plans',async()=>{
  await act(async()=>{await import('./ProposedPlan');});
  const onPlanAction=vi.fn(async()=>{});
  const plan=message('native-plan','assistant','# 完整原生计划\n\n包括全部边界验收。','plan',{status:'inProgress',turnId:'t'});
  const base=[message('question','user','先规划'),plan];
  const draw=async(messages:Message[],status:string)=>act(async()=>root.render(<Conversation messages={messages} agentName="Codex" sessionStatus={status} autoExpand={false} showDiff={()=>{}} openFile={()=>{}} onPlanAction={onPlanAction}/>));
  await draw([...base,run('inProgress')],'running');
  expect(node.querySelector('.plan-request-execute')).toBeNull();
  const finished=[base[0],{...plan,data:{...plan.data,status:'completed'}},run('completed')];
  await draw(finished,'idle');
  expect(node.querySelector('.plan-request-execute')).not.toBeNull();
  expect(onPlanAction).not.toHaveBeenCalled();
  await draw([...finished,message('next','user','继续'),message('reply','assistant','后续回复'),run('completed','run2')],'idle');
  expect(node.textContent).toContain('包括全部边界验收');
  expect(node.querySelector('.plan-request-execute')).toBeNull();
});

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

it('folds intermediate Pi responses while keeping every block of the complete final response visible and copyable', async () => {
  const progress = '先检查环境，之后继续验证配置。';
  const first = '第一类报错：这个 bash 环境里没有安装 Python 和 Node。';
  const second = '第二类报错：配置路径不正确，需要调整路径后再运行。';
  const final = '处理方法：安装运行环境，然后修复配置路径。';
  for (const grouped of [false, true]) {
    const trace = [
      message('question', 'user', '为什么运行报错了'),
      message('t-msg-1-0', 'assistant', progress, 'agentMessage', grouped ? { messageId:'t-msg-1' } : null),
      message('tool-check', 'tool', 'python: command not found', 'commandExecution', { command:'python --version', status:'completed' }),
      message('t-msg-3-0', 'assistant', first, 'agentMessage', grouped ? { messageId:'t-msg-3' } : null),
      message('thought-check', 'activity', '检查配置路径', 'reasoning', { status:'completed' }),
      message('t-msg-3-1', 'assistant', second, 'agentMessage', grouped ? { messageId:'t-msg-3' } : null),
      message('t-msg-3-2', 'assistant', final, 'agentMessage', grouped ? { messageId:'t-msg-3' } : null),
    ];
    for (const marker of [[], [run('completed')]]) {
      await render([...trace, ...marker], false, { sessionStatus:'idle' });
      const result = node.querySelector('.turn-final-result')!;
      for (const section of [first, second, final]) expect(result.textContent).toContain(section);
      expect(result.querySelectorAll('.message.assistant')).toHaveLength(3);
      expect(result.textContent).not.toContain(progress);
      expect(node.querySelector('.turn-process-content')).toBeNull();
      await act(async () => node.querySelector<HTMLButtonElement>('.turn-copy')!.click());
      expect(writeText).toHaveBeenLastCalledWith([first, second, final].join('\n\n'));
      await toggle();
      const process = node.querySelector('.turn-process-content')!;
      expect(process.querySelectorAll('.message.assistant')).toHaveLength(1);
      expect(process.textContent).toContain(progress);
      expect(node.querySelectorAll('.turn-final-result .message.assistant')).toHaveLength(3);
      await toggle();
    }
  }
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

it('keeps the process and waiting indicator visible until plan approval and continuation finish', async () => {
  const marker = { ...run('completed'), data:{ status:'completed', turnId:'plan-first', durationMs:61000 } };
  const trace = [items[0], marker, items[1], items[2], message('planning', 'assistant', '还在设计方案，请稍等。')];
  await render(trace, false, { sessionStatus:'waiting', activeTurnId:'plan-first' });
  expect(node.querySelector('.turn-process-header')).toBeNull();
  expect(node.querySelector('.turn-live-content')?.textContent).toContain('还在设计方案');
  expect(node.querySelector('.working-indicator .pending-text')?.textContent).toContain('等待你的确认');
  expect(node.querySelector('.turn-footer')).toBeNull();
  await render(trace, false, { sessionStatus:'running', activeTurnId:'plan-next' });
  expect(node.querySelector('.turn-process-header')).toBeNull();
  expect(node.querySelector('.working-indicator .pending-text')).not.toBeNull();
  await render([...trace, { ...run('completed', 'next'), data:{ status:'completed', turnId:'plan-next' } }], false,
    { sessionStatus:'running', activeTurnId:'plan-next' });
  expect(node.querySelector('.turn-process-header')).not.toBeNull();
  expect(node.querySelector('.working-indicator')).toBeNull();
});

it('shows the streaming plan without expanding tool details and fades only newly arriving text', async () => {
  const plan = '# 实施计划\n\n## 第一阶段\n\n创建原型。';
  const draft = message('plan-draft', 'tool', '', 'claudeToolCall', { tool:'ExitPlanMode', status:'preparing', turnId:'plan', inputText:JSON.stringify({ plan }).slice(0, -2) });
  await act(async () => { await import('./PlanDocument'); });
  await render([items[0], draft], false, { sessionStatus:'running', activeTurnId:'plan' });
  const document = node.querySelector('.plan-generation .plan-document');
  expect(document?.textContent).toContain('创建原型');
  expect(document?.querySelector('.stream-text-fragment')).not.toBeNull();
  expect(node.querySelector('[aria-label="复制详情"]')).toBeNull();
  const heading = document?.querySelector('h1');
  const continued = { ...draft, data:{ ...draft.data, inputText:JSON.stringify({ plan:plan+'\n\n## 验收\n\n验证完整流程。' }).slice(0, -2) } };
  await render([items[0], continued], false, { sessionStatus:'running', activeTurnId:'plan' });
  expect(node.querySelector('.plan-generation h1')).toBe(heading);
  expect(node.querySelector('.plan-generation')?.textContent).toContain('验证完整流程');
  expect(node.textContent).not.toContain('"plan":');
  await act(async () => node.querySelector<HTMLButtonElement>('.activity-group-summary')!.click());
  expect(node.querySelector('.activity-body .plan-document')?.textContent).toContain('验证完整流程');
  expect(node.querySelector('.plan-generation')).toBeNull();
  await render([items[0], { ...continued, data:{ ...continued.data, status:'completed', arguments:{ plan } } }], true,
    { sessionStatus:'idle' });
  expect(node.querySelector('.plan-generation')).toBeNull();
  expect(node.querySelector('.plan-document .stream-text-fragment')).toBeNull();
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
  const earlier = message('earlier', 'assistant', '更早的过程', 'agentMessage', { phase:'commentary' });
  await render([marker, earlier, ...page], true);
  expect(node.querySelector('.turn-process-content')?.textContent).toContain('更早的过程');
  expect(node.querySelector('.turn-final-result .message')).toBe(final);
  await render([items[0], marker, earlier, ...page], true);
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
  expect(node.textContent).toContain('命令');
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
