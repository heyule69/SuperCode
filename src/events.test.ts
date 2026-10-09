import { describe, expect, it } from 'vitest';
import { conversationTurns, mergeEvent } from './events';
import type { RpcEvent } from './types';

describe('Codex event replay', () => {
  it('merges streamed Chinese text and authoritative completion without duplication', () => {
    const event = (method: string, params: Record<string, unknown>): RpcEvent => ({ method, params });
    let messages = mergeEvent([], event('item/started', { item: { id: 'a1', type: 'agentMessage', text: '' } }), 'session');
    messages = mergeEvent(messages, event('item/agentMessage/delta', { itemId: 'a1', delta: '你好' }), 'session');
    messages = mergeEvent(messages, event('item/agentMessage/delta', { itemId: 'a1', delta: '🧋' }), 'session');
    expect(messages[0].text).toBe('你好🧋');
    messages = mergeEvent(messages, event('item/completed', { item: { id: 'a1', type: 'agentMessage', text: '你好🧋，已完成。' } }), 'session');
    expect(messages).toHaveLength(1); expect(messages[0].text).toBe('你好🧋，已完成。');
    expect(mergeEvent(messages, event('future/notification', {}), 'session')).toBe(messages);
  });
  it('keeps command progress and completion in one tool card', () => {
    const started: RpcEvent = { method: 'item/started', params: { item: { id: 'c1', type: 'commandExecution', command: 'git status', status: 'inProgress' } } };
    const completed: RpcEvent = { method: 'item/completed', params: { item: { id: 'c1', type: 'commandExecution', command: 'git status', status: 'completed', aggregatedOutput: '修改了 中文.txt' } } };
    const messages = mergeEvent(mergeEvent([], started, 's'), completed, 's');
    expect(messages).toHaveLength(1); expect(messages[0].role).toBe('tool'); expect(messages[0].text).toContain('中文.txt'); expect(messages[0].data?.status).toBe('completed');
  });
  it('streams reasoning sections and preserves them when final summaries are omitted', () => {
    const start: RpcEvent = { method: 'item/started', params: { turnId: 't', item: { id: 'r', type: 'reasoning', summary: [] } } };
    let messages = mergeEvent([], start, 's');
    for (const [summaryIndex, delta] of [[0, '先阅读中文文件。'], [1, '再验证 🧋。']] as const) messages = mergeEvent(messages, { method: 'item/reasoning/summaryTextDelta', params: { itemId: 'r', summaryIndex, delta } }, 's');
    expect(messages[0].text).toBe('先阅读中文文件。\n\n再验证 🧋。');
    messages = mergeEvent(messages, { method: 'item/completed', params: { turnId: 't', item: { id: 'r', type: 'reasoning', summary: [] } } }, 's');
    expect(messages).toHaveLength(1); expect(messages[0].data?.status).toBe('completed'); expect(messages[0].text).toContain('🧋');
  });
  it('shows command output incrementally, then uses authoritative final output', () => {
    let messages = mergeEvent([], { method: 'item/started', params: { turnId: 't', item: { id: 'c', type: 'commandExecution', command: 'git status', status: 'inProgress' } } }, 's');
    messages = mergeEvent(messages, { method: 'item/commandExecution/outputDelta', params: { itemId: 'c', delta: '正在读取…\n' } }, 's');
    expect(messages[0].text).toContain('正在读取');
    messages = mergeEvent(messages, { method: 'item/completed', params: { turnId: 't', item: { id: 'c', type: 'commandExecution', command: 'git status', status: 'failed', exitCode: 1, aggregatedOutput: '错误：不是仓库' } } }, 's');
    expect(messages[0].text).toBe('git status\n错误：不是仓库'); expect(messages[0].data?.status).toBe('failed');
  });
  it('keeps Claude input fragments, structured arguments, and tool results in one item', () => {
    let messages = mergeEvent([], { method: 'item/started', params: { turnId: 't', item: { id: 'read', type: 'claudeToolCall', tool: 'Read', arguments: {}, status: 'preparing' } } }, 's');
    messages = mergeEvent(messages, { method: 'item/claudeToolCall/inputDelta', params: { itemId: 'read', delta: '{"file_path":"中文.md"}' } }, 's');
    expect(messages[0].data?.inputText).toContain('中文');
    messages = mergeEvent(messages, { method: 'item/updated', params: { turnId: 't', item: { id: 'read', type: 'claudeToolCall', tool: 'Read', arguments: { file_path: '中文.md' }, status: 'inProgress' } } }, 's');
    messages = mergeEvent(messages, { method: 'item/completed', params: { turnId: 't', item: { id: 'read', type: 'claudeToolCall', tool: 'Read', output: '文件内容', status: 'completed' } } }, 's');
    expect(messages).toHaveLength(1); expect(messages[0].text).toBe('文件内容'); expect(messages[0].data?.arguments).toEqual({ file_path: '中文.md' }); expect(messages[0].data?.output).toBeUndefined();
  });
  it('updates the plan and terminates only unfinished items from the stopped turn', () => {
    let messages = mergeEvent([], { method: 'turn/started', params: { turn: { id: 't' } } }, 's');
    messages = mergeEvent(messages, { method: 'turn/plan/updated', params: { turnId: 't', plan: [{ step: '读取项目', status: 'inProgress' }] } }, 's');
    messages = mergeEvent(messages, { method: 'item/started', params: { turnId: 't', item: { id: 'c', type: 'commandExecution', command: 'sleep 30', status: 'inProgress' } } }, 's');
    messages = mergeEvent(messages, { method: 'turn/completed', params: { turn: { id: 't', status: 'interrupted' } } }, 's');
    expect(messages.every(m => m.data?.status === 'interrupted')).toBe(true); expect(messages[0].data?.durationMs).toBeGreaterThanOrEqual(0);
  });
  it('retains MCP progress/result and separates a second turn from the first', () => {
    let messages = mergeEvent([], { method: 'item/started', params: { turnId: 't', item: { id: 'm', type: 'mcpToolCall', server: 'docs', tool: 'search', arguments: { query: 'test' }, status: 'inProgress' } } }, 's');
    messages = mergeEvent(messages, { method: 'item/mcpToolCall/progress', params: { itemId: 'm', message: '找到 2 项' } }, 's');
    messages = mergeEvent(messages, { method: 'item/completed', params: { turnId: 't', item: { id: 'm', type: 'mcpToolCall', result: { content: [{ type: 'text', text: '答案' }] }, status: 'completed' } } }, 's');
    expect(messages[0].data?.progress).toContain('找到 2 项'); expect(messages[0].data?.result).toBeDefined();
    messages.push({ seq: 2, id: 'u', sessionId: 's', role: 'user', text: '下一轮', kind: 'userMessage', data: null });
    messages = mergeEvent(messages, { method: 'turn/started', params: { turn: { id: 't2' } } }, 's');
    expect(conversationTurns(messages)).toHaveLength(2); expect(conversationTurns(messages)[1].run?.data?.turnId).toBe('t2');
  });
  it('bounds large tool payloads while preserving status and Unicode boundaries', () => {
    const output = '中'.repeat(128 * 1024 - 1) + '🧋';
    const messages = mergeEvent([], { method: 'item/completed', params: { turnId: 't', item: { id: 'large', type: 'claudeToolCall', tool: 'Write', status: 'failed', arguments: { content: 'x'.repeat(4 * 1024 * 1024) }, output } } }, 's');
    expect(messages[0].text).not.toMatch(/[\uD800-\uDBFF]$/);
    expect(messages[0].data?.status).toBe('failed'); expect(messages[0].data?.tool).toBe('Write');
    expect(JSON.stringify(messages[0].data).length).toBeLessThan(140 * 1024);
  });
});
