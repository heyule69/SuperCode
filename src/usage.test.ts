import { describe, expect, it } from 'vitest';
import { usageDay, usageGroups, usageTotals, type UsageRecord } from './usage';

const record = (sessionId: string, model: string, at: number): UsageRecord => ({ sessionId, model, at, turnId: `${sessionId}:${at}`, agent: 'claude', title: sessionId, data: { total: { inputTokens: 1000, outputTokens: 100, totalTokens: 1100, cachedInputTokens: 800 }, last: {} } });
describe('usage views', () => {
  it('uses turn deltas and never adds cached input twice', () => {
    const r = record('s1', 'glm', 1); r.data.turn = { inputTokens: 200, outputTokens: 30, cachedInputTokens: 100, totalTokens: 230 }; r.data.costUsd = 0;
    expect(usageTotals([r])).toEqual({ input: 200, output: 30, cached: 100, tokens: 230, cost: 0, priced: 1 });
  });
  it('keeps unknown cost absent and combines a chat across model changes', () => {
    const rows = [record('s1', 'glm', 1), record('s1', 'kimi', 2), record('s2', 'glm', 3)];
    expect(usageTotals(rows).priced).toBe(0);
    const chats = usageGroups(rows, 'chat');
    expect(chats.map(g => g.id)).toEqual(['s2', 's1']);
    expect(chats[1].models.size).toBe(2); expect(chats[1].totals.tokens).toBe(2200);
    expect(usageGroups(rows, 'model')).toHaveLength(2);
  });
  it('uses local calendar days rather than UTC dates', () => {
    const date = new Date(2026, 9, 7, 0, 15);
    expect(usageDay(date.getTime() / 1000)).toBe('2026-10-07');
  });
});
