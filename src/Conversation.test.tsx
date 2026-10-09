import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { Conversation } from './Conversation';
import { mergeEvent } from './events';
import type { Message } from './types';

const message = (id: string, role: string, text: string, kind = 'agentMessage', data: Message['data'] = null): Message => ({seq: 0, id, role, text, kind, data, sessionId: 's'});
const messages = [
  message('u', 'user', '把 PDF 的排版修好', 'userMessage'),
  message('a1', 'assistant', '排版已经修好了。清理一下预览用的临时图片：'),
  message('tool', 'tool', '清理完成', 'claudeToolCall', {tool: 'Bash', arguments: {command: '清理临时图片'}, status: 'completed'}),
  message('a2', 'assistant', '搞定，PDF 已保存。'),
  message('run', 'activity', '', 'runMarker', {status: 'completed', durationMs: 15000}),
];
function render(items = messages, autoExpand = false) {
  return renderToStaticMarkup(<Conversation messages={items} agentName="Claude Code" autoExpand={autoExpand} showDiff={() => {}} openFile={() => {}}/>);
}

describe('continuous task replies', () => {
  it('shows tool screenshots with collapsed details and allows a media-only assistant reply', () => {
    const media = [{ kind: 'image', path: 'D:/cache/截图.png', name: '截图' }];
    const html = render([messages[0], message('shot', 'tool', '', 'mcpToolCall', { tool: 'screenshot', status: 'completed', media })]);
    expect(html).not.toContain('class="activity-row"');
    expect(html).toContain('data-media-source="D:/cache/截图.png"');
    const assistant = render([messages[0], message('picture', 'assistant', '', 'agentMessage', { media })]);
    expect(assistant).toContain('data-media-source="D:/cache/截图.png"');
  });
  it('keeps intermediate and final text in one reply with one copy action', () => {
    const html = render();
    expect(html.match(/class="assistant-turn"/g)).toHaveLength(1);
    expect(html.match(/aria-label="复制回复"/g)).toHaveLength(1);
    expect(html).toContain('排版已经修好了');
    expect(html).toContain('PDF 已保存');
    expect(html).not.toContain('message-avatar');
    expect(html).not.toContain('message-label');
    expect(html).toContain('aria-label="你的消息"');
    expect(html).toContain('用时 15 秒');
    expect(html).toContain('<div class="turn-summary completed">');
    expect(html).not.toContain('<button class="turn-summary');
  });
  it('shows a short tool summary below the reply, including after completion', () => {
    const compact = render();
    expect(compact).not.toContain('class="activity-row"');
    expect(compact).toContain('class="activity-group-summary"');
    expect(compact).toContain('运行了命令');
    expect(compact.indexOf('排版已经修好了')).toBeLessThan(compact.indexOf('运行了命令'));
    expect(compact.indexOf('运行了命令')).toBeLessThan(compact.indexOf('PDF 已保存'));
    const expanded = render(messages, true);
    expect(expanded).toContain('class="activity-row"');
    expect(expanded.indexOf('排版已经修好了')).toBeLessThan(expanded.indexOf('执行命令'));
    expect(expanded.indexOf('执行命令')).toBeLessThan(expanded.indexOf('PDF 已保存'));
  });
  it('shows tools during a run and only separates another user request', () => {
    const running = [...messages.slice(0, -1), message('run', 'activity', '', 'runMarker', {status: 'inProgress'})];
    expect(render(running)).toContain('class="activity-group-summary"');
    const next = [...messages, message('u2', 'user', '再修改一下标题', 'userMessage'), message('a3', 'assistant', '正在修改标题。')];
    expect(render(next).match(/class="assistant-turn"/g)).toHaveLength(2);
  });
  it('updates the live heading from reading to reasoning to the next tool for both agents', () => {
    for (const kind of ['claudeToolCall', 'commandExecution']) {
      const tool = kind === 'claudeToolCall' ? { tool: 'Read', arguments: { file_path: 'test/page.html' } } : { command: 'read file', commandActions: [{ type: 'read', path: 'test/page.html' }] };
      let items = mergeEvent([message('u', 'user', '继续优化', 'userMessage')], { method: 'turn/started', params: { turn: { id: 't' } } }, 's');
      items = mergeEvent(items, { method: 'item/started', params: { turnId: 't', item: { id: 'read', type: kind, ...tool, status: 'inProgress' } } }, 's');
      expect(render(items)).toContain('aria-label="正在读取文件，1 条记录"');
      items = mergeEvent(items, { method: 'item/completed', params: { turnId: 't', item: { id: 'read', type: kind, ...tool, status: 'completed' } } }, 's');
      expect(render(items)).toContain('aria-label="正在思考，1 条记录"');
      items = mergeEvent(items, { method: 'item/started', params: { turnId: 't', item: { id: 'thinking', type: 'reasoning', summary: [], status: 'inProgress' } } }, 's');
      const thinking = render(items, true);
      expect(thinking).toContain('aria-label="正在思考，2 条记录"');
      expect(thinking).toContain('读取了文件');
      expect(thinking).not.toContain('class="spin"');
      items = mergeEvent(items, { method: 'item/started', params: { turnId: 't', item: { id: 'command', type: 'commandExecution', command: 'build', status: 'inProgress' } } }, 's');
      expect(render(items)).toContain('aria-label="正在运行命令，3 条记录"');
      items = mergeEvent(items, { method: 'turn/completed', params: { turn: { id: 't', status: 'completed' } } }, 's');
      expect(render(items)).not.toContain('正在思考');
    }
  });
  it('does not relabel a historical tool group when a reply separates it from the current activity', () => {
    const running = [...messages.slice(0, -1), message('thinking', 'activity', '', 'reasoning', { status: 'inProgress' }), message('empty', 'assistant', '', 'agentMessage', { status: 'inProgress' }), message('run', 'activity', '', 'runMarker', { status: 'inProgress' })];
    const html = render(running);
    expect(html).toContain('aria-label="运行了命令，1 条记录"');
    expect(html).toContain('aria-label="正在思考，1 条记录"');
    expect(html.match(/正在思考，1 条记录/g)).toHaveLength(1);
  });
  it('combines adjacent tools without merging the text on either side', () => {
    const extra = message('edit', 'tool', '已保存', 'claudeToolCall', { tool: 'Edit', arguments: { file_path: 'src/App.tsx' }, status: 'completed' });
    const html = render([...messages.slice(0, 3), extra, ...messages.slice(3)]);
    expect(html.match(/class="activity-group-summary"/g)).toHaveLength(1);
    expect(html).toContain('运行了命令、编辑了文件');
    expect(html.indexOf('排版已经修好了')).toBeLessThan(html.indexOf('运行了命令、编辑了文件'));
    expect(html.indexOf('运行了命令、编辑了文件')).toBeLessThan(html.indexOf('PDF 已保存'));
    expect(html).not.toContain('清理临时图片');
  });
  it('shows per-file and total line counts with three files initially visible', () => {
    const diff = ['src/a.ts', 'src/b.ts', 'src/c.ts', 'docs/readme.md'].map(path => `diff --git a/${path} b/${path}\n--- a/${path}\n+++ b/${path}\n@@ -1 +1,2 @@\n-old\n+new\n+more\n`).join('');
    const html = render([...messages, message('diff', 'activity', diff, 'turnDiff', { status: 'completed' })]);
    expect(html).toContain('已编辑 4 个文件');
    expect(html).toContain('<span class="added-count">+8</span>');
    expect(html).toContain('<span class="removed-count">−4</span>');
    expect(html.match(/class="turn-change-file"/g)).toHaveLength(3);
    expect(html).toContain('再显示 1 个文件');
    expect(html).toContain('data-resource-path="src/a.ts"');
  });
});

it('shows a supplier switch once at the handoff boundary, without repeating reply headers', () => {
  const items=[...messages,message('switch','system','上下文已压缩','modelSwitch',{from:{providerName:'Kimi Code'},to:{providerName:'智谱 GLM'},fromModel:'k3',toModel:'glm-test'}),message('u2','user','继续','userMessage'),message('a3','assistant','继续完成任务')];
  const html=render(items);
  expect(html.match(/class="model-switch-marker"/g)).toHaveLength(1);
  expect(html).toContain('Kimi Code'); expect(html).toContain('智谱 GLM');
  expect(html.indexOf('用时 15 秒')).toBeLessThan(html.indexOf('model-switch-marker'));
  expect(html.indexOf('model-switch-marker')).toBeLessThan(html.lastIndexOf('你的消息'));
  expect(html).not.toContain('message-label'); expect(html).not.toContain('message-avatar');
});

it('labels preserved history honestly when no compaction was performed', () => {
  const html = render([message('switch','system','上下文已保留','modelSwitch',{contextMode:'history',from:{providerName:'Kimi'},to:{providerName:'智谱'}})]);
  expect(html).toContain('上下文已保留');
  expect(html).not.toContain('上下文已压缩');
});
