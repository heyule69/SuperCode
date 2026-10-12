import { describe, expect, it } from 'vitest';
import { conversationTurns, mergeEvent, reconcileMessages } from './events';
import type { Message, RpcEvent } from './types';
it('retains patch output separately from paths and reserves exact statistics when details are large',()=>{
  let rows=mergeEvent([],{method:'item/started',params:{turnId:'t',item:{id:'edit',type:'fileChange',status:'inProgress',changes:[{path:'a.ts'}]}}},'s');
  rows=mergeEvent(rows,{method:'item/fileChange/outputDelta',params:{turnId:'t',itemId:'edit',delta:'PATCH_OUTPUT'}},'s');
  expect(rows[0].text).toBe('a.ts');
  expect(rows[0].data?.operationOutput).toBe('PATCH_OUTPUT');
  rows=mergeEvent(rows,{method:'item/completed',params:{turnId:'t',item:{id:'edit',type:'fileChange',status:'completed',changes:[{path:'a.ts',diff:'@@ -1 +1 @@\n-old\n+new\n'}],arguments:{a:'x'.repeat(150000)},changeSummary:[{path:'a.ts',added:4000,removed:1}]}}},'s');
  expect(rows[0].data?.operationOutput).toBe('PATCH_OUTPUT');
  expect(rows[0].data?.changeSummary).toEqual([{path:'a.ts',added:4000,removed:1}]);
  expect(rows[0].data?.durationMs).toBeGreaterThanOrEqual(0);
});

it('keeps paginated lifecycle context through native item updates and a later completion', () => {
  const historyRun = {id:'run-old',turnKey:'client-u',data:{turnId:'old',status:'inProgress'}};
  const page: Message[] = [{id:'item',seq:101,sessionId:'s',role:'assistant',kind:'agentMessage',text:'进度',data:{turnId:'old'},historyRun}];
  const updated = mergeEvent(page, {method:'item/completed',params:{turnId:'old',item:{id:'item',type:'agentMessage',text:'最终回复',status:'completed'}}}, 's');
  expect(updated[0].historyRun).toBe(historyRun);
  expect(conversationTurns(updated)[0]).toMatchObject({id:'client-u',run:{id:'run-old',data:{status:'inProgress'}}});
  const completed = mergeEvent(updated, {method:'turn/completed',params:{turn:{id:'old',status:'completed'}}}, 's');
  expect(conversationTurns(completed)[0]).toMatchObject({id:'client-u',run:{id:'run-old',data:{status:'completed'}}});
  expect(completed.filter(message => message.role === 'assistant')).toHaveLength(1);
});

describe('message acknowledgements', () => {
  const user = (id: string, seq: number, clientMessageId?: string): Message => ({id,seq,sessionId:'s',role:'user',kind:'userMessage',text:'继续',data:clientMessageId ? {clientMessageId} : {}});
  it('retains an unacknowledged user before newly persisted work instead of matching an older identical text', () => {
    const older = user('older', 1), pending = user('pending', 0, 'client');
    const run = mergeEvent([], {method:'turn/started',params:{turn:{id:'t'}}}, 's')[0];
    const merged = reconcileMessages([older, pending, run], [older, {...run, seq:2}], new Set(['client']));
    expect(merged.map(message => message.id)).toEqual(['older','pending','run-t']);
    expect(conversationTurns(merged)[1].user).toBe(pending);
    expect(conversationTurns(merged)[1].run?.id).toBe('run-t');
  });
  it('replaces a pending user exactly once on acknowledgement and keeps its turn identity', () => {
    const pending = user('pending', 0, 'client'), saved = user('saved', 1, 'client');
    const run = mergeEvent([], {method:'turn/started',params:{turn:{id:'t'}}}, 's')[0];
    const merged = reconcileMessages([pending, run], [saved], new Set(['client']));
    expect(merged.map(message => message.id)).toEqual(['saved','run-t']);
    expect(conversationTurns(merged)[0].id).toBe(conversationTurns([pending, run])[0].id);
    expect(reconcileMessages(merged, [saved, {...run,seq:2}], new Set()).map(message => message.id)).toEqual(['saved','run-t']);
  });
  it('places new submissions after history that was still loading and before matching live work', () => {
    const older = user('older', 1), pending = user('pending', 0, 'client');
    const run = mergeEvent([], {method:'turn/started',params:{turn:{id:'t'}}}, 's')[0];
    expect(reconcileMessages([pending], [older], new Set(['client'])).map(message => message.id)).toEqual(['older','pending']);
    expect(reconcileMessages([pending,run], [older,{...run,seq:2}], new Set(['client'])).map(message => message.id)).toEqual(['older','pending','run-t']);
  });
  it('removes failed local users and preserves streamed text until authoritative completion', () => {
    const failed = user('failed', 0, 'client');
    const live: Message = {id:'reply',seq:0,sessionId:'s',role:'assistant',text:'完整实时文本',kind:'agentMessage',data:{status:'inProgress'}};
    expect(reconcileMessages([failed,live], [{...live,seq:1,text:'完整'}], new Set())).toEqual([{...live,seq:1}]);
    expect(reconcileMessages([live], [{...live,seq:1,text:'最终',data:{status:'completed'}}], new Set())[0].text).toBe('最终');
  });
  it('does not let a delayed history read reopen a finished turn', () => {
    const started = mergeEvent([], {method:'turn/started',params:{turn:{id:'t'}}}, 's')[0];
    const saved = { ...started, seq:2 };
    for (const status of ['completed', 'failed', 'interrupted']) {
      const finished = mergeEvent([saved], {method:'turn/completed',params:{turn:{id:'t',status}}}, 's')[0];
      // Native start time and the frontend's event receipt time differ slightly.
      const delayed = { ...saved, data:{ ...saved.data, startedAt:Number(saved.data?.startedAt) - 50 } };
      expect(reconcileMessages([finished], [delayed], new Set())[0]).toEqual(finished);
    }
    // A different run generation must still use the new history state.
    const later = { ...saved, id:'run-t2', data:{ ...saved.data, turnId:'t2', startedAt:Number(saved.data?.startedAt) + 1000 } };
    const finished = mergeEvent([saved], {method:'turn/completed',params:{turn:{id:'t',status:'completed'}}}, 's')[0];
    expect(reconcileMessages([finished], [later], new Set())).toEqual([later]);
  });
  it('retains the run marker when a long turn exceeds the latest history page', () => {
    const run = { ...mergeEvent([], {method:'turn/started',params:{turn:{id:'t'}}}, 's')[0], seq:2 };
    const answer:Message = {id:'final',seq:150,sessionId:'s',role:'assistant',text:'完成',kind:'agentMessage',data:{turnId:'t',status:'completed'}};
    const unrelated = { ...run, id:'run-old', seq:1, data:{turnId:'old',status:'completed'} };
    const live = [unrelated, run, answer];
    const merged = reconcileMessages(live, [answer], new Set());
    expect(merged.map(message => message.id)).toEqual(['run-t', 'final']);
    const finished = mergeEvent(merged, {method:'turn/completed',params:{turn:{id:'t',status:'completed'}}}, 's');
    expect(conversationTurns(finished)[0].run?.data?.status).toBe('completed');
    expect(reconcileMessages(finished, [answer], new Set()).map(message => message.id)).toEqual(['run-t', 'final']);
  });
});

describe('Codex event replay', () => {
  it('handles completion with a paginated-away start marker without attaching it to the next turn', () => {
    const old:Message = {id:'old-final',seq:100,sessionId:'s',role:'assistant',text:'之前的结果',kind:'agentMessage',data:{turnId:'old',status:'completed'}};
    const next:Message = {id:'next',seq:101,sessionId:'s',role:'user',text:'下一轮',kind:'userMessage',data:{}};
    const finished = mergeEvent([old, next], {method:'turn/completed',params:{turn:{id:'old',status:'completed'}}}, 's');
    const turns = conversationTurns(finished);
    expect(turns[0].run?.data).toMatchObject({turnId:'old',status:'completed'});
    expect(turns[0].run?.data?.durationMs).toBeUndefined();
    expect(turns[1].run).toBeUndefined();
    expect(mergeEvent(finished, {method:'turn/completed',params:{turn:{id:'unrelated',status:'completed'}}}, 's')).toEqual(finished);
  });
  it('retains final phases and native content groups through metadata truncation and turn completion', () => {
    let messages = mergeEvent([], { method: 'turn/started', params: { turn: { id: 't' } } }, 's');
    messages = mergeEvent(messages, { method: 'item/completed', params: { turnId: 't', item: { id: 'reply', type: 'agentMessage', text: '最终结果', arguments: { details: 'x'.repeat(160000) }, phase: 'final_answer', messageId: 'native-message' } } }, 's');
    messages = mergeEvent(messages, { method: 'turn/completed', params: { turn: { id: 't', status: 'completed' } } }, 's');
    const reply = messages.find(message => message.id === 'reply')!;
    expect(reply.data).toMatchObject({ phase: 'final_answer', messageId: 'native-message', status: 'completed' });
    expect(reply.text).toBe('最终结果');
  });
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
