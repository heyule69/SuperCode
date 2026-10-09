import { describe, expect, it } from 'vitest';
import { indexedTurns, visibleTurnIndex } from './ConversationIndex';
import { conversationTurns } from './events';
import type { Message } from './types';

const message = (id: string, role: string, text: string, kind = 'agentMessage'): Message => ({ id, role, text, kind, data: null, seq: 0, sessionId: 's' });
describe('conversation outline', () => {
  it('indexes user rounds, excluding tool blocks and supplier boundaries', () => {
    const history = [message('u1', 'user', '第一轮', 'userMessage'), message('a1', 'assistant', '开始处理'), message('t', 'tool', '输出', 'commandExecution'), message('a2', 'assistant', '完成'), message('switch', 'system', '已切换', 'modelSwitch'), message('u2', 'user', '第二轮', 'userMessage')];
    const items = indexedTurns(conversationTurns(history));
    expect(items).toHaveLength(2);
    expect(items[0]).toMatchObject({ question: '第一轮', reply: '完成', command: true });
    expect(items[1]).toMatchObject({ question: '第二轮', reply: '' });
  });
  it('adds one mark when a message is sent and bounds long preview text', () => {
    const history = [message('u1', 'user', '附件消息', 'userMessage'), message('a', 'assistant', '内容'.repeat(300))];
    expect(indexedTurns(conversationTurns(history))[0].reply.length).toBe(220);
    expect(indexedTurns(conversationTurns([...history, message('u2', 'user', '新增一轮', 'userMessage')]))).toHaveLength(2);
    expect(indexedTurns(conversationTurns([]))).toEqual([]);
  });
});

describe('visible conversation round', () => {
  const viewport = { top: 100, height: 500, scrollTop: 700, scrollHeight: 4000 };
  it('highlights the round at the reading point before its question reaches the top edge', () => {
    expect(visibleTurnIndex([-500, 220, 950], viewport)).toBe(1);
    expect(visibleTurnIndex([-500, 400, 950], viewport)).toBe(0);
  });
  it('keeps a long reply associated with its user round', () => {
    expect(visibleTurnIndex([-1700, 800, 1600], viewport)).toBe(0);
    expect(visibleTurnIndex([-2200, -100, 1100], viewport)).toBe(1);
  });
  it('selects a short final round at the bottom, including fractional scroll positions', () => {
    expect(visibleTurnIndex([-1000, -50, 520], { ...viewport, scrollTop: 700.5, scrollHeight: 1202 })).toBe(2);
    expect(visibleTurnIndex([-1000, -50, 520], { ...viewport, scrollHeight: 1400 })).toBe(1);
  });
  it('does not select a round below the viewport', () => {
    expect(visibleTurnIndex([-800, 200, 650], { ...viewport, scrollHeight: 1200 })).toBe(1);
  });
  it('recalculates after content growth and viewport resize without a scroll change', () => {
    expect(visibleTurnIndex([-500, 220, 950], viewport)).toBe(1);
    expect(visibleTurnIndex([-500, 420, 1150], viewport)).toBe(0);
    expect(visibleTurnIndex([-500, 220, 950], { ...viewport, height: 200 })).toBe(0);
  });
  it('keeps the same turn identity when earlier history is loaded', () => {
    const before = [{ id: 'u2', top: -500 }, { id: 'u3', top: 220 }, { id: 'u4', top: 950 }];
    const after = [{ id: 'u1', top: -1200 }, ...before];
    expect(before[visibleTurnIndex(before.map(turn => turn.top), viewport)].id).toBe('u3');
    expect(after[visibleTurnIndex(after.map(turn => turn.top), viewport)].id).toBe('u3');
  });
  it('handles a single round, missing nodes and a hidden conversation', () => {
    expect(visibleTurnIndex([], viewport)).toBe(-1);
    expect(visibleTurnIndex([Infinity, 220, Infinity], viewport)).toBe(1);
    expect(visibleTurnIndex([Infinity], viewport)).toBe(-1);
    expect(visibleTurnIndex([220], viewport)).toBe(0);
    expect(visibleTurnIndex([220], { ...viewport, height: 0 })).toBe(-1);
  });
});
