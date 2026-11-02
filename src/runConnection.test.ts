import { describe, expect, it } from 'vitest';
import { isDisplayEvent, mergeEvent, reconcileMessages, conversationTurns } from './events';
import { readRunConnection, updateRunConnection } from './runConnection';
import type { Message, RpcEvent } from './types';
const started = (): Message[] => mergeEvent([], { method:'turn/started', params:{ threadId:'n', turn:{ id:'t' } } }, 's');
const retry = (at = 1000): RpcEvent => ({ method:'turn/connection/updated', params:{ threadId:'n', turnId:'t', connection:{ status:'retrying', at, kind:'network', attempt:2, maxAttempts:5, retryAt:at + 2000 } } });
const state = (rows: Message[]) => readRunConnection(conversationTurns(rows)[0]?.run?.data?.connection);

describe('turn connection metadata', () => {
  it('routes normalized retries into the existing run, not a new chat message', () => {
    const rows = mergeEvent(started(), retry(), 's');
    expect(isDisplayEvent(retry())).toBe(true);
    expect(rows).toHaveLength(1);
    expect(state(rows)).toMatchObject({ status:'retrying', attempt:2, maxAttempts:5 });
    expect(mergeEvent(rows, { ...retry(), id:'approval' }, 's')).toBe(rows);
  });
  it('clears retry state on actual model progress while local output and token counters do not prove recovery', () => {
    let rows = mergeEvent(started(), retry(), 's');
    rows = mergeEvent(rows, { method:'item/commandExecution/outputDelta', params:{ turnId:'t', itemId:'local', delta:'local output' } }, 's');
    expect(state(rows)?.status).toBe('retrying');
    rows = mergeEvent(rows, { method:'thread/tokenUsage/updated', params:{ turnId:'t' } }, 's');
    expect(state(rows)?.status).toBe('retrying');
    rows = mergeEvent(rows, { method:'item/agentMessage/delta', params:{ turnId:'t', itemId:'reply', delta:'已恢复' } }, 's');
    expect(state(rows)?.status).toBe('restored');
    expect(state(rows)?.retryAt).toBeUndefined();
  });
  it('retains failures and deliberate cancellation accurately at completion', () => {
    for (const [status, expected] of [['failed', 'failed'], ['interrupted', 'stopped'], ['completed', 'restored']]) {
      const rows = mergeEvent(mergeEvent(started(), retry(), 's'), { method:'turn/completed', params:{ turn:{ id:'t', status } } }, 's');
      expect(state(rows)?.status).toBe(expected);
      expect(mergeEvent(rows, retry(3000), 's')).toBe(rows);
    }
  });
  it('ignores another session, stale turns and stale retry snapshots', () => {
    const rows = mergeEvent(started(), retry(5000), 's');
    expect(mergeEvent(rows, retry(2000), 's')).toBe(rows);
    expect(mergeEvent(rows, { ...retry(6000), params:{ ...retry(6000).params, turnId:'old' } }, 's')).toBe(rows);
    expect(updateRunConnection(rows, retry(6000), 'other')).toBe(rows);
    const saved = mergeEvent(started(), retry(2000), 's').map(m => ({ ...m, seq:3 }));
    expect(state(reconcileMessages(rows, saved, new Set()))?.at).toBe(5000);
  });
  it('keeps retry details through paginated run context, terminal failure and reload', () => {
    const rows: Message[] = [{ id:'answer', seq:103, sessionId:'s', role:'assistant', kind:'agentMessage', text:'尚未完成', data:{ turnId:'t' }, historyRun:{ id:'run-t', turnKey:'user', data:{ turnId:'t', status:'inProgress' } } }];
    const live = mergeEvent(rows, retry(), 's');
    expect(state(live)?.status).toBe('retrying');
    const finished = mergeEvent(live, { method:'turn/completed', params:{ turn:{ id:'t', status:'failed' } } }, 's');
    expect(state(JSON.parse(JSON.stringify(finished)))?.status).toBe('failed');
    expect(finished.filter(m => m.role === 'assistant')).toHaveLength(1);
  });
  it('accepts recoverable legacy errors without treating arbitrary stderr prose as a retry signal', () => {
    const raw = { method:'error', params:{ turnId:'t', willRetry:true, error:{ message:'Reconnecting... 2/5' } } };
    const rows = mergeEvent(started(), raw, 's'); expect(state(rows)?.status).toBe('retrying');
    expect(state(rows)?.attempt).toBeUndefined();
    const failed = mergeEvent(rows, { method:'error', params:{ turnId:'t', willRetry:false } }, 's');
    expect(state(failed)?.status).toBe('failed');
    expect(state(mergeEvent(started(), { method:'log', params:{ turnId:'t', text:'reconnecting' } }, 's'))).toBeUndefined();
  });
  it('sanitizes unknown, oversized and invalid metadata', () => {
    expect(readRunConnection({ status:'imagined', at:1000 })).toBeUndefined();
    expect(readRunConnection({ status:'retrying', at:NaN })).toBeUndefined();
    expect(readRunConnection({ status:'retrying', at:1000, attempt:-1, maxAttempts:Infinity, retryAt:1e30, kind:'unsafe', text:'private' })).toEqual({ status:'retrying', at:1000, kind:'unknown' });
  });
});
