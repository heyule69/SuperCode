import { describe, expect, it } from 'vitest';
import { contextUsage, latestChatUsage, usageDay, usageGroups, usageTotals, type UsageRecord } from './usage';

const record = (sessionId: string, model: string, at: number): UsageRecord => ({ sessionId, model, at, turnId: `${sessionId}:${at}`, agent: 'claude', title: sessionId, data: { total: { inputTokens: 1000, outputTokens: 100, totalTokens: 1100, cachedInputTokens: 800 }, last: {} } });
describe('usage views', () => {
  it('counts translation costs without using auxiliary calls as the chat context', () => {
    const chat = record('s', 'model', 1), translation = record('s', 'model', 2), handoff = record('s', 'model', 3);
    translation.data.purpose = 'translation'; handoff.data.purpose = 'handoff';
    expect(latestChatUsage([translation, handoff, chat])).toBe(chat.data);
    expect(latestChatUsage([translation, handoff])).toBeUndefined();
    expect(usageTotals([translation, handoff, chat]).tokens).toBe(3300);
  });
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

describe('context occupancy', () => {
  it('preserves the native context percentage and labels estimated windows', () => {
    const native = { total: {}, last: {}, contextTokens: 25000, modelContextWindow: 100000, contextPercent: 42, contextSource: 'claude' as const, contextApproximate: false };
    expect(contextUsage(native).label).toBe('上下文 42%');
    expect(contextUsage(native).rawPercent).toBe(42);
    expect(contextUsage({ total: {}, last: {}, contextTokens: 150000, contextSource: 'usage' }, 1000000).label).toBe('上下文 ~15%');
    expect(contextUsage({ total: {}, last: {}, contextTokens: 150000, contextSource: 'usage' }).label).toBe('上下文 ~150.0K');
    expect(contextUsage({ ...native, modelContextWindow: undefined }).label).toBe('上下文 25.0K');
  });
  it('normalizes legacy Claude measurements to input only without rewriting stored records', () => {
    const saved = record('s', 'glm-5.3', 1);
    saved.data = { total: { totalTokens: 999999 }, last: { inputTokens: 151336, outputTokens: 456, totalTokens: 151792 }, contextTokens: 151792, modelContextWindow: 1000000, cumulative: false };
    const normalized = latestChatUsage([saved])!;
    expect(normalized.contextTokens).toBe(151336);
    expect(contextUsage(normalized).label).toBe('上下文 ~15%');
    expect(saved.data.contextTokens).toBe(151792);
  });
  it('does not restore context from a previous model or native thread after a switch', () => {
    const old = record('s', 'old-model', 1);
    old.data = { total: {}, last: {}, contextTokens: 80000, modelContextWindow: 1000000, counterEpoch: 'old-native' };
    expect(latestChatUsage([old], { agent: 'claude', model: 'new-model', nativeId: 'old-native' })).toBeUndefined();
    expect(latestChatUsage([old], { agent: 'claude', model: 'old-model', nativeId: 'new-native' })).toBeUndefined();
    expect(latestChatUsage([old], { agent: 'claude', model: 'old-model', nativeId: null })).toBeUndefined();
    expect(latestChatUsage([old], { agent: 'claude', model: 'old-model', nativeId: 'old-native' })?.contextTokens).toBe(80000);
  });
  it('shows real tokens when the provider omits capacity', () => {
    expect(contextUsage({ total: { totalTokens: 90000 }, last: {}, contextTokens: 12345, cumulative: false })).toMatchObject({ used: 12345, window: undefined, percent: undefined, label: '上下文 12.3K', estimated: false });
  });
  it('shows percentage with a real capacity and accepts zero usage', () => {
    expect(contextUsage({ total: {}, last: {}, contextTokens: 16000, modelContextWindow: 128000 }).label).toBe('上下文 13%');
    expect(contextUsage({ total: {}, last: {}, contextTokens: 0, modelContextWindow: 128000 }).label).toBe('上下文 0%');
    expect(contextUsage({ total: {}, last: {}, contextTokens: 0 }).label).toBe('上下文 0');
    expect(contextUsage({ total: {}, last: {}, contextTokens: 100, modelContextWindow: 1000000 }).label).toBe('上下文 <1%');
  });
  it('uses the latest Codex call, never cumulative or multi-call turn totals', () => {
    expect(contextUsage({ total: { totalTokens: 999999 }, last: { totalTokens: 18000, inputTokens: 17000 } }).used).toBe(18000);
    expect(contextUsage({ total: { totalTokens: 999999 }, last: { totalTokens: 0, inputTokens: 17000, outputTokens: 1000 } }).used).toBe(18000);
    expect(contextUsage({ total: { totalTokens: 999999 }, last: { inputTokens: 999999 }, contextTokens: null, cumulative: false }).used).toBeUndefined();
    expect(contextUsage({ total: { totalTokens: 999999 }, last: { inputTokens: 999999 }, cumulative: false }).used).toBeUndefined();
  });
  it('does not turn absent or invalid data into zero or a percentage', () => {
    expect(contextUsage().label).toBe('上下文 —');
    expect(contextUsage({ total: {}, last: {}, contextTokens: NaN, modelContextWindow: 128000 }).label).toBe('上下文 —');
    expect(contextUsage({ total: {}, last: {}, contextTokens: 100, modelContextWindow: -1 }).label).toBe('上下文 100');
  });
});
