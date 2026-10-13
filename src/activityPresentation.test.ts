import { describe, expect, it } from 'vitest';
import { compactActivities, concisePath, summarizeActivities } from './activityPresentation';
import type { Message } from './types';

const item = (id: string, data: Record<string, unknown> = {}) => ({ id, seq: 0, sessionId: 'test', role: 'tool', kind: 'commandExecution', text: '', data }) as Message;
describe('compact execution records', () => {
  it('keeps the latest progress and an earlier failed command visible', () => {
    const messages = [item('first'), item('failure', { exitCode: 1 }), item('middle'), item('latest-file'), item('running', { status: 'inProgress' })];
    expect(compactActivities(messages).map(message => message.id)).toEqual(['failure', 'latest-file', 'running']);
  });
  it('retains order and gives short groups in full', () => {
    expect(compactActivities([item('a'), item('b')]).map(message => message.id)).toEqual(['a', 'b']);
    expect(compactActivities(['a', 'b', 'c', 'd'].map(id => item(id))).map(message => message.id)).toEqual(['b', 'c', 'd']);
    expect(concisePath('C:\\project\\src\\App.tsx')).toBe('src/App.tsx');
  });
});

describe('activity summaries', () => {
  const tool = (id: string, name: string, path?: string, status = 'completed') => ({ ...item(id, { tool: name, arguments: { file_path: path }, status }), kind: 'claudeToolCall' });
  it('shows animated plan progress for native planning tools', () => {
    expect(summarizeActivities([tool('plan', 'ExitPlanMode', undefined, 'preparing')], true))
      .toEqual({ label:'正在生成计划', category:'plan', status:'preparing' });
  });
  it('combines consecutive operations in order and counts distinct edited files', () => {
    const result = summarizeActivities([
      tool('a', 'Edit', 'C:\\project\\src\\App.tsx'),
      tool('b', 'Write', 'C:/project/src/styles.css'),
      item('c'),
      { ...item('d', { server: 'playwright', tool: 'browser_click' }), kind: 'mcpToolCall' },
    ]);
    expect(result).toEqual({ label: '编辑了多个文件、运行了命令、使用了浏览器', category: 'edit', status: 'completed' });
  });
  it('does not call repeated edits to one Windows file multiple files', () => {
    expect(summarizeActivities([tool('a', 'Edit', 'C:\\project\\App.tsx'), tool('b', 'Edit', 'c:/project/App.tsx')]).label).toBe('编辑了文件');
  });
  it('summarizes Codex file changes and command actions from their metadata', () => {
    const changes = { ...item('a', { changes: [{ path: 'src/a.ts' }, { path: 'src/b.ts' }] }), kind: 'fileChange' };
    expect(summarizeActivities([changes, item('b', { commandActions: [{ type: 'read' }, { type: 'search' }] })]).label).toBe('编辑了多个文件、读取了文件、搜索了代码');
  });
  it('keeps live and failed states visible without making up shell actions', () => {
    expect(summarizeActivities([tool('a', 'Edit', 'a.ts'), item('b', { status: 'inProgress', command: 'playwright edit file' })])).toEqual({ label: '编辑了文件、正在执行命令', category: 'edit', status: 'inProgress' });
    expect(summarizeActivities([item('failed', { exitCode: 1 }), item('running', { status: 'running' })]).status).toBe('failed');
    expect(summarizeActivities([item('browser-in-command', { command: 'browser edit file' })]).label).toBe('运行了命令');
  });
  it('keeps reasoning available but out of a mixed tool summary', () => {
    const reasoning = { ...item('r', { status: 'completed' }), role: 'activity', kind: 'reasoning' };
    expect(summarizeActivities([reasoning]).label).toBe('思考摘要');
    expect(summarizeActivities([reasoning, item('c')]).label).toBe('运行了命令');
    expect(summarizeActivities([tool('unknown', 'custom_tool')]).label).toBe('调用了工具');
  });
  it('shows the newest ongoing action over earlier completed tools and failures', () => {
    const thinking = { ...item('r', { status: 'inProgress' }), role: 'activity', kind: 'reasoning' };
    const earlier = [item('failed', { exitCode: 1 }), tool('read', 'Read', 'a.ts')];
    expect(summarizeActivities([...earlier, thinking], true)).toEqual({ label: '正在思考', category: 'reasoning', status: 'inProgress' });
    expect(summarizeActivities([...earlier, thinking, tool('write', 'Write', 'b.ts', 'preparing')], true)).toEqual({ label: '正在准备写文件', category: 'edit', status: 'preparing' });
    expect(summarizeActivities([...earlier, thinking, item('command', { status: 'running' })], true)).toEqual({ label: '正在执行命令', category: 'command', status: 'running' });
  });
  it('does not claim thinking after tool completion or reuse an older plan as current activity', () => {
    const plan = { ...item('plan', { status: 'inProgress' }), role: 'activity', kind: 'executionPlan' };
    expect(summarizeActivities([plan, tool('read', 'Read', 'a.ts')], true)).toEqual({ label: '正在处理', category: 'tool', status: 'inProgress' });
    expect(summarizeActivities([tool('read', 'Read', 'a.ts')], true).label).toBe('正在处理');
    expect(summarizeActivities([tool('read', 'Read', 'a.ts')], false).label).toBe('读取了文件');
    expect(summarizeActivities([plan], true).label).toBe('正在更新计划');
  });
  it('distinguishes live file writes, edits, reads and command execution from parameter generation', () => {
    for (const [name, executing, preparing] of [
      ['Write', '正在写文件', '正在准备写文件'],
      ['Edit', '正在编辑文件', '正在准备编辑文件'],
      ['Read', '正在读取文件', '正在准备读取文件'],
      ['Glob', '正在查找文件', '正在准备查找文件'],
      ['Bash', '正在执行命令', '正在准备执行命令'],
    ]) {
      expect(summarizeActivities([tool('active', name, 'page.html', 'inProgress')], true).label).toBe(executing);
      expect(summarizeActivities([tool('active', name, 'page.html', 'preparing')], true).label).toBe(preparing);
    }
  });
  it('prioritizes an executing tool over concurrent reasoning and ignores stale thoughts after completion', () => {
    const thought = { ...item('thought', { status: 'inProgress' }), kind: 'reasoning', role: 'activity' };
    expect(summarizeActivities([tool('write', 'Write', 'page.html', 'inProgress'), thought], true).label).toBe('正在写文件');
    expect(summarizeActivities([thought, tool('write', 'Write', 'page.html')], true).label).toBe('正在处理');
    expect(summarizeActivities([{ ...thought, data: { status: 'completed' } }, tool('write', 'Write', 'page.html')], true).label).toBe('正在处理');
  });
  it('uses declared Codex file-change kinds without guessing actions from paths or shell text', () => {
    for (const [kind, label] of [['add', '正在写文件'], ['delete', '正在删除文件'], ['update', '正在编辑文件']]) {
      expect(summarizeActivities([{ ...item('file', { status: 'inProgress', changes: [{ path: 'page.html', kind: { type: kind } }] }), kind: 'fileChange' }], true).label).toBe(label);
    }
  });
  it('shows reply streaming only when an actual assistant item is in progress', () => {
    const reply = { ...item('reply', { status: 'inProgress' }), role: 'assistant', kind: 'agentMessage', text: '正在输出' };
    expect(summarizeActivities([reply], true).label).toBe('正在回复');
    expect(summarizeActivities([{ ...reply, data: { status: 'completed' } }], true).label).toBe('正在处理');
  });
});
